use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::{Arc, Mutex};

use greentic_interfaces::runner_host_v1::{self, RunnerHost};
use greentic_interfaces_host::component::v0_6::exports::greentic::component::node;
use greentic_interfaces_host::component_v0_6::greentic::component::control::Host as ControlHost;
use greentic_interfaces_wasmtime::host_helpers::v1::state_store::{
    OpAck, StateStoreError, StateStoreHost, TenantCtx as WitTenantCtx, add_state_store_to_linker,
};
use greentic_types::TenantCtx;
use greentic_types::cbor::canonical;
use reqwest::blocking::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use wasmtime::StoreContextMut;
use wasmtime::component::{Linker, ResourceTable};
use wasmtime::{Engine, Result as WasmtimeResult};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use crate::error::CompError;
use crate::loader::ComponentRef;
use crate::policy::HostPolicy;

pub struct HostState {
    _tenant: Option<TenantCtx>,
    _config: Value,
    _secrets: HashMap<String, Vec<u8>>,
    wasi_ctx: WasiCtx,
    wasi_table: ResourceTable,
    policy: HostPolicy,
    state_store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    runner: RunnerHostImpl,
    control: ControlHostImpl,
}

impl HostState {
    pub fn empty(policy: HostPolicy) -> Self {
        let (wasi_ctx, wasi_table) = build_wasi_state();
        let runner_policy = policy.clone();
        let state_store = policy.state_store.clone();
        Self {
            _tenant: None,
            _config: Value::Null,
            _secrets: HashMap::new(),
            wasi_ctx,
            wasi_table,
            state_store,
            policy,
            runner: RunnerHostImpl::new(runner_policy),
            control: ControlHostImpl,
        }
    }

    pub fn from_binding(
        tenant: TenantCtx,
        config: Value,
        secrets: HashMap<String, Vec<u8>>,
        policy: HostPolicy,
    ) -> Self {
        let (wasi_ctx, wasi_table) = build_wasi_state();
        let runner_policy = policy.clone();
        let state_store = policy.state_store.clone();
        Self {
            _tenant: Some(tenant),
            _config: config,
            _secrets: secrets,
            wasi_ctx,
            wasi_table,
            state_store,
            policy,
            runner: RunnerHostImpl::new(runner_policy),
            control: ControlHostImpl,
        }
    }
}

fn build_wasi_state() -> (WasiCtx, ResourceTable) {
    let mut wasi_builder = WasiCtxBuilder::new();
    (wasi_builder.build(), ResourceTable::new())
}

struct RunnerHostImpl {
    policy: HostPolicy,
    http_client: HttpClient,
}

impl RunnerHostImpl {
    fn new(policy: HostPolicy) -> Self {
        Self {
            policy,
            http_client: HttpClient::new(),
        }
    }
}

impl RunnerHost for RunnerHostImpl {
    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<String>,
        body: Option<Vec<u8>>,
    ) -> WasmtimeResult<Result<Vec<u8>, String>> {
        if !self.policy.allow_http_fetch {
            return Ok(Err("http fetch denied by policy".into()));
        }

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|err| CompError::Runtime(err.to_string()))?;
        let url = url
            .parse::<reqwest::Url>()
            .map_err(|err| CompError::Runtime(err.to_string()))?;

        let mut builder = self.http_client.request(method, url);

        if !headers.is_empty() {
            let mut header_map = HeaderMap::new();
            for entry in headers {
                if let Some((name, value)) = entry.split_once(':') {
                    let header_name = HeaderName::from_bytes(name.trim().as_bytes())
                        .map_err(|err| CompError::Runtime(err.to_string()))?;
                    let header_value = HeaderValue::from_str(value.trim())
                        .map_err(|err| CompError::Runtime(err.to_string()))?;
                    header_map.append(header_name, header_value);
                }
            }
            builder = builder.headers(header_map);
        }

        if let Some(body) = body {
            builder = builder.body(body);
        }

        let response = builder
            .send()
            .map_err(|err| CompError::Runtime(err.to_string()))?;
        let bytes = response
            .bytes()
            .map_err(|err| CompError::Runtime(err.to_string()))?;

        Ok(Ok(bytes.to_vec()))
    }

    fn kv_get(&mut self, _ns: String, _key: String) -> WasmtimeResult<Option<String>> {
        // Legacy runner-host surface; routed to the state store for compatibility.
        let key = format!("{_ns}:{_key}");
        if !self.policy.allow_state_read {
            return Ok(None);
        }
        let guard = self
            .policy
            .state_store
            .lock()
            .expect("state store mutex poisoned");
        match guard.get(&key) {
            Some(bytes) => Ok(Some(
                String::from_utf8(bytes.clone())
                    .map_err(|err| CompError::Runtime(err.to_string()))?,
            )),
            None => Ok(None),
        }
    }

    fn kv_put(&mut self, _ns: String, _key: String, _val: String) -> WasmtimeResult<()> {
        // Legacy runner-host surface; routed to the state store for compatibility.
        if !self.policy.allow_state_write {
            return Ok(());
        }
        let key = format!("{_ns}:{_key}");
        let mut guard = self
            .policy
            .state_store
            .lock()
            .expect("state store mutex poisoned");
        let bytes = _val.into_bytes();
        guard.insert(key, canonicalize_cbor_or_passthrough(&bytes));
        Ok(())
    }
}

struct ControlHostImpl;

impl ControlHost for ControlHostImpl {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn yield_now(&mut self) {}
}

pub fn build_linker(engine: &Engine, _policy: &HostPolicy) -> Result<Linker<HostState>, CompError> {
    let mut linker = Linker::<HostState>::new(engine);
    runner_host_v1::add_to_linker(&mut linker, |state: &mut HostState| &mut state.runner)?;
    add_control_to_linker_v0_6(&mut linker, |state: &mut HostState| &mut state.control)?;
    add_state_store_to_linker(&mut linker, |state: &mut HostState| state)?;
    p2::add_to_linker_sync(&mut linker)?;
    Ok(linker)
}

fn add_control_to_linker_v0_6<T>(
    linker: &mut Linker<T>,
    get_host: impl Fn(&mut T) -> &mut (dyn ControlHost + Send + Sync + 'static)
    + Send
    + Sync
    + Copy
    + 'static,
) -> wasmtime::Result<()>
where
    T: Send + 'static,
{
    let mut inst = linker.instance("greentic:component/control@0.6.0")?;

    inst.func_wrap(
        "should-cancel",
        move |mut caller: StoreContextMut<'_, T>, (): ()| {
            let host = get_host(caller.data_mut());
            let result = host.should_cancel();
            Ok((result,))
        },
    )?;

    inst.func_wrap(
        "yield-now",
        move |mut caller: StoreContextMut<'_, T>, (): ()| {
            let host = get_host(caller.data_mut());
            host.yield_now();
            Ok(())
        },
    )?;

    Ok(())
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.wasi_table,
        }
    }
}

impl StateStoreHost for HostState {
    fn read(
        &mut self,
        key: String,
        _ctx: Option<WitTenantCtx>,
    ) -> Result<Vec<u8>, StateStoreError> {
        if !self.policy.allow_state_read {
            return Err(StateStoreError {
                code: "state.read.denied".into(),
                message: "state store reads are disabled by policy".into(),
            });
        }
        let guard = self.state_store.lock().expect("state store mutex poisoned");
        match guard.get(&key) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(StateStoreError {
                code: "state.read.miss".into(),
                message: format!("state key `{key}` not found"),
            }),
        }
    }

    fn write(
        &mut self,
        key: String,
        bytes: Vec<u8>,
        _ctx: Option<WitTenantCtx>,
    ) -> Result<OpAck, StateStoreError> {
        if !self.policy.allow_state_write {
            return Err(StateStoreError {
                code: "state.write.denied".into(),
                message: "state store writes are disabled by policy".into(),
            });
        }
        let mut guard = self.state_store.lock().expect("state store mutex poisoned");
        guard.insert(key, canonicalize_cbor_or_passthrough(&bytes));
        Ok(OpAck::Ok)
    }

    fn delete(
        &mut self,
        key: String,
        _ctx: Option<WitTenantCtx>,
    ) -> Result<OpAck, StateStoreError> {
        if !self.policy.allow_state_delete {
            return Err(StateStoreError {
                code: "state.delete.denied".into(),
                message: "state store deletes are disabled by policy".into(),
            });
        }
        let mut guard = self.state_store.lock().expect("state store mutex poisoned");
        guard.remove(&key);
        Ok(OpAck::Ok)
    }
}

fn canonicalize_cbor_or_passthrough(bytes: &[u8]) -> Vec<u8> {
    match canonical::canonicalize_allow_floats(bytes) {
        Ok(canonical_bytes) => canonical_bytes,
        Err(_) => bytes.to_vec(),
    }
}

pub fn make_invocation_envelope(
    cref: &ComponentRef,
    tenant: &TenantCtx,
    operation: &str,
    payload_cbor: Vec<u8>,
) -> node::InvocationEnvelope {
    node::InvocationEnvelope {
        ctx: make_component_tenant_ctx(tenant),
        flow_id: cref.name.clone(),
        step_id: tenant
            .node_id
            .clone()
            .unwrap_or_else(|| operation.to_string()),
        component_id: tenant
            .node_id
            .clone()
            .unwrap_or_else(|| "component".to_string()),
        attempt: tenant.attempt,
        payload_cbor,
        metadata_cbor: None,
    }
}

pub fn make_component_tenant_ctx(tenant: &TenantCtx) -> node::TenantCtx {
    node::TenantCtx {
        tenant_id: tenant.tenant_id.as_str().to_string(),
        team_id: tenant.team_id.as_ref().map(|t| t.as_str().to_string()),
        user_id: tenant.user_id.as_ref().map(|u| u.as_str().to_string()),
        env_id: tenant.env.as_str().to_string(),
        trace_id: tenant
            .trace_id
            .clone()
            .unwrap_or_else(|| "trace-local".to_string()),
        correlation_id: tenant
            .correlation_id
            .clone()
            .unwrap_or_else(|| "corr-local".to_string()),
        i18n_id: tenant.i18n_id.clone().unwrap_or_else(|| "en".to_string()),
        deadline_ms: tenant
            .deadline
            .and_then(|deadline| u64::try_from(deadline.unix_millis()).ok())
            .unwrap_or(0),
        attempt: tenant.attempt,
        idempotency_key: tenant.idempotency_key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::{EnvId, TenantId};
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn spawn_http_server() -> std::io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let response = "\
                    HTTP/1.1 200 OK\r\n\
                    Content-Type: text/plain\r\n\
                    X-Custom: value\r\n\
                    Content-Length: 5\r\n\
                    \r\n\
                    hello";
                let _ = stream.write_all(response.as_bytes());
            }
        });

        Ok(format!("http://{}:{}/test", addr.ip(), addr.port()))
    }

    fn host_state(
        allow_http: bool,
        allow_state_read: bool,
        allow_state_write: bool,
        allow_state_delete: bool,
    ) -> HostState {
        let state_store = Arc::new(Mutex::new(HashMap::new()));
        let policy = HostPolicy {
            allow_http_fetch: allow_http,
            allow_telemetry: true,
            allow_state_read,
            allow_state_write,
            allow_state_delete,
            state_store: state_store.clone(),
        };
        HostState::empty(policy)
    }
    #[test]
    fn http_fetch_denied_by_policy() {
        let mut host = host_state(false, false, false, false);
        let result = RunnerHost::http_request(
            &mut host.runner,
            "GET".into(),
            "http://localhost".into(),
            vec![],
            None,
        );
        assert!(matches!(result, Ok(Err(_))));
    }

    #[test]
    fn http_fetch_success() {
        let url = match spawn_http_server() {
            Ok(url) => url,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping http_fetch_success: {err}");
                return;
            }
            Err(err) => panic!("bind http listener: {err}"),
        };
        let mut host = host_state(true, false, false, false);
        let response = RunnerHost::http_request(&mut host.runner, "GET".into(), url, vec![], None)
            .expect("http fetch");
        let body = response.expect("http ok");
        assert_eq!(body, b"hello");
    }

    #[test]
    fn state_store_denies_write() {
        let mut host = host_state(false, false, false, false);
        let result = StateStoreHost::write(&mut host, "demo".into(), b"data".to_vec(), None);
        assert!(matches!(result, Err(err) if err.code == "state.write.denied"));
    }

    #[test]
    fn state_store_roundtrip() {
        let mut host = host_state(false, true, true, true);
        let write = StateStoreHost::write(&mut host, "demo".into(), b"data".to_vec(), None);
        assert!(matches!(write, Ok(OpAck::Ok)));

        let read = StateStoreHost::read(&mut host, "demo".into(), None).expect("state read");
        assert_eq!(read, b"data");

        let delete = StateStoreHost::delete(&mut host, "demo".into(), None);
        assert!(matches!(delete, Ok(OpAck::Ok)));

        let missing = StateStoreHost::read(&mut host, "demo".into(), None);
        assert!(matches!(missing, Err(err) if err.code == "state.read.miss"));
    }

    #[test]
    fn state_store_write_canonicalizes_cbor_payload() {
        let mut host = host_state(false, true, true, false);
        let non_canonical = vec![0xa2, 0x61, b'b', 0x01, 0x61, b'a', 0x02];
        let expected = canonical::canonicalize_allow_floats(&non_canonical).expect("canonical");

        let write = StateStoreHost::write(&mut host, "demo".into(), non_canonical, None);
        assert!(matches!(write, Ok(OpAck::Ok)));

        let read = StateStoreHost::read(&mut host, "demo".into(), None).expect("state read");
        assert_eq!(read, expected);
    }

    #[test]
    fn state_store_denies_read_and_delete_when_disabled() {
        let mut host = host_state(false, false, true, false);
        let write = StateStoreHost::write(&mut host, "demo".into(), b"data".to_vec(), None);
        assert!(matches!(write, Ok(OpAck::Ok)));

        let read = StateStoreHost::read(&mut host, "demo".into(), None);
        assert!(matches!(read, Err(err) if err.code == "state.read.denied"));

        let delete = StateStoreHost::delete(&mut host, "demo".into(), None);
        assert!(matches!(delete, Err(err) if err.code == "state.delete.denied"));
    }

    #[test]
    fn kv_get_and_put_follow_state_policy_and_utf8_rules() {
        let mut denied = host_state(false, false, false, false);
        RunnerHost::kv_put(
            &mut denied.runner,
            "ns".into(),
            "key".into(),
            "value".into(),
        )
        .expect("denied writes become no-ops");
        assert_eq!(
            RunnerHost::kv_get(&mut denied.runner, "ns".into(), "key".into())
                .expect("denied reads return none"),
            None
        );

        let mut allowed = host_state(false, true, true, false);
        RunnerHost::kv_put(
            &mut allowed.runner,
            "ns".into(),
            "key".into(),
            "value".into(),
        )
        .expect("write");
        assert_eq!(
            RunnerHost::kv_get(&mut allowed.runner, "ns".into(), "key".into()).expect("read"),
            Some("value".into())
        );

        allowed
            .policy
            .state_store
            .lock()
            .expect("lock")
            .insert("ns:binary".into(), vec![0xff, 0xfe]);
        let err = RunnerHost::kv_get(&mut allowed.runner, "ns".into(), "binary".into())
            .expect_err("non-utf8 values should fail");
        assert!(err.to_string().contains("invalid utf-8"));
    }

    #[test]
    fn invocation_context_uses_tenant_and_fallback_values() {
        let mut tenant = TenantCtx::new(EnvId("dev".into()), TenantId("tenant".into()));
        tenant.attempt = 3;
        let ctx = make_component_tenant_ctx(&tenant);
        assert_eq!(ctx.env_id, "dev");
        assert_eq!(ctx.tenant_id, "tenant");
        assert_eq!(ctx.trace_id, "trace-local");
        assert_eq!(ctx.correlation_id, "corr-local");
        assert_eq!(ctx.i18n_id, "en");
        assert_eq!(ctx.attempt, 3);

        let envelope = make_invocation_envelope(
            &ComponentRef {
                name: "fixture".into(),
                locator: "file:///tmp/component.wasm".into(),
            },
            &tenant,
            "handle_message",
            vec![1, 2, 3],
        );
        assert_eq!(envelope.flow_id, "fixture");
        assert_eq!(envelope.step_id, "handle_message");
        assert_eq!(envelope.component_id, "component");
        assert_eq!(envelope.payload_cbor, vec![1, 2, 3]);
    }
}
