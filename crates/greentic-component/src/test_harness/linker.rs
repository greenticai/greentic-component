use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use greentic_interfaces::runner_host_v1::{self, RunnerHost};
use greentic_interfaces_host::component::v0_5::{self, ControlHost};
use greentic_interfaces_wasmtime::host_helpers::v1::secrets_store::{
    SecretsError, SecretsStoreHost, add_secrets_store_to_linker,
};
use greentic_interfaces_wasmtime::host_helpers::v1::state_store::{
    OpAck, StateStoreError, StateStoreHost, TenantCtx as WitTenantCtx, add_state_store_to_linker,
};
use reqwest::blocking::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use wasmtime::component::Linker;
use wasmtime::{Engine, ResourceLimiter};
use wasmtime_wasi::clocks::{HostMonotonicClock, HostWallClock};
use wasmtime_wasi::random::Deterministic;
use wasmtime_wasi::{
    DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

use crate::test_harness::WasiPreopen;
use crate::test_harness::secrets::InMemorySecretsStore;
use crate::test_harness::state::{InMemoryStateStore, StateScope};

pub struct HostState {
    control: ControlHostImpl,
    runner: RunnerHostImpl,
    state: StateStoreHostImpl,
    secrets: SecretsStoreHostImpl,
    wasi_ctx: WasiCtx,
    wasi_table: ResourceTable,
    limits: HostLimits,
    memory_limit_hit: Arc<AtomicBool>,
}

pub struct HostStateConfig {
    pub base_scope: StateScope,
    pub state_store: Arc<InMemoryStateStore>,
    pub secrets: Arc<InMemorySecretsStore>,
    pub allow_state_read: bool,
    pub allow_state_write: bool,
    pub allow_state_delete: bool,
    pub wasi_preopens: Vec<WasiPreopen>,
    pub allow_http: bool,
    pub config_json: Option<String>,
    pub max_memory_bytes: usize,
}

impl HostState {
    pub fn new(config: HostStateConfig) -> Result<Self> {
        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.secure_random(Deterministic::new(vec![0, 1, 2, 3]));
        wasi_builder.insecure_random(Deterministic::new(vec![4, 5, 6, 7]));
        wasi_builder.insecure_random_seed(0);
        wasi_builder.wall_clock(FixedWallClock::new());
        wasi_builder.monotonic_clock(FixedMonotonicClock::new());
        for preopen in &config.wasi_preopens {
            let (dir_perms, file_perms) = if preopen.read_only {
                (DirPerms::READ, FilePerms::READ)
            } else {
                (DirPerms::all(), FilePerms::all())
            };
            wasi_builder
                .preopened_dir(
                    &preopen.host_path,
                    &preopen.guest_path,
                    dir_perms,
                    file_perms,
                )
                .map_err(|err| {
                    anyhow!(
                        "failed to preopen {} as {}: {}",
                        preopen.host_path.display(),
                        preopen.guest_path,
                        err
                    )
                })?;
        }

        let memory_limit_hit = Arc::new(AtomicBool::new(false));
        let limits = HostLimits::new(config.max_memory_bytes, memory_limit_hit.clone());

        Ok(Self {
            control: ControlHostImpl,
            runner: RunnerHostImpl::new(config.allow_http, config.config_json),
            state: StateStoreHostImpl::new(
                config.base_scope,
                config.state_store,
                config.allow_state_read,
                config.allow_state_write,
                config.allow_state_delete,
            ),
            secrets: SecretsStoreHostImpl::new(config.secrets),
            wasi_ctx: wasi_builder.build(),
            wasi_table: ResourceTable::new(),
            limits,
            memory_limit_hit,
        })
    }

    pub fn memory_limit_hit(&self) -> bool {
        self.memory_limit_hit.load(Ordering::Relaxed)
    }

    pub fn limits_mut(&mut self) -> &mut dyn ResourceLimiter {
        &mut self.limits
    }
}

pub fn build_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::<HostState>::new(engine);
    runner_host_v1::add_to_linker(&mut linker, |state: &mut HostState| &mut state.runner)
        .map_err(|err| anyhow!("failed to add runner host linker: {err}"))?;
    v0_5::add_control_to_linker(&mut linker, |state: &mut HostState| &mut state.control)
        .map_err(|err| anyhow!("failed to add control linker: {err}"))?;
    add_state_store_to_linker(&mut linker, |state: &mut HostState| &mut state.state)
        .map_err(|err| anyhow!("failed to add state-store linker: {err}"))?;
    add_secrets_store_to_linker(&mut linker, |state: &mut HostState| &mut state.secrets)
        .map_err(|err| anyhow!("failed to add secrets-store linker: {err}"))?;
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|err| anyhow!("failed to add wasi linker: {err}"))?;
    Ok(linker)
}

pub struct ControlHostImpl;

impl ControlHost for ControlHostImpl {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn yield_now(&mut self) {}
}

pub struct RunnerHostImpl {
    allow_http: bool,
    config_json: Option<String>,
    http_client: HttpClient,
}

impl RunnerHostImpl {
    fn new(allow_http: bool, config_json: Option<String>) -> Self {
        Self {
            allow_http,
            config_json,
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
    ) -> wasmtime::Result<Result<Vec<u8>, String>> {
        if !self.allow_http {
            return Ok(Err(
                "http fetch denied in greentic-component test harness".to_string()
            ));
        }

        let method = match reqwest::Method::from_bytes(method.as_bytes()) {
            Ok(method) => method,
            Err(err) => return Ok(Err(format!("invalid http method: {err}"))),
        };
        let url = match url.parse::<reqwest::Url>() {
            Ok(url) => url,
            Err(err) => return Ok(Err(format!("invalid http url: {err}"))),
        };

        let mut builder = self.http_client.request(method, url);

        if !headers.is_empty() {
            let mut header_map = HeaderMap::new();
            for entry in headers {
                if let Some((name, value)) = entry.split_once(':') {
                    let header_name = match HeaderName::from_bytes(name.trim().as_bytes()) {
                        Ok(header_name) => header_name,
                        Err(err) => return Ok(Err(format!("invalid header name: {err}"))),
                    };
                    let header_value = match HeaderValue::from_str(value.trim()) {
                        Ok(header_value) => header_value,
                        Err(err) => return Ok(Err(format!("invalid header value: {err}"))),
                    };
                    header_map.append(header_name, header_value);
                }
            }
            builder = builder.headers(header_map);
        }

        if let Some(body) = body {
            builder = builder.body(body);
        }

        let response = match builder.send() {
            Ok(response) => response,
            Err(err) => return Ok(Err(format!("http request failed: {err}"))),
        };
        let status = response.status();
        let bytes = match response.bytes() {
            Ok(bytes) => bytes,
            Err(err) => return Ok(Err(format!("http response body failed: {err}"))),
        };
        if status.is_success() {
            Ok(Ok(bytes.to_vec()))
        } else {
            Ok(Err(format!("http request failed with status {status}")))
        }
    }

    fn kv_get(&mut self, _ns: String, _key: String) -> wasmtime::Result<Option<String>> {
        if _ns == "config" && _key == "json" {
            return Ok(self.config_json.clone());
        }
        Ok(None)
    }

    fn kv_put(&mut self, _ns: String, _key: String, _val: String) -> wasmtime::Result<()> {
        Ok(())
    }
}

pub struct StateStoreHostImpl {
    base_scope: StateScope,
    state_store: Arc<InMemoryStateStore>,
    allow_state_read: bool,
    allow_state_write: bool,
    allow_state_delete: bool,
}

impl StateStoreHostImpl {
    fn new(
        base_scope: StateScope,
        state_store: Arc<InMemoryStateStore>,
        allow_state_read: bool,
        allow_state_write: bool,
        allow_state_delete: bool,
    ) -> Self {
        Self {
            base_scope,
            state_store,
            allow_state_read,
            allow_state_write,
            allow_state_delete,
        }
    }

    fn scope_for_ctx(&self, ctx: Option<&WitTenantCtx>) -> StateScope {
        let mut scope = self.base_scope.clone();
        if let Some(ctx) = ctx {
            if !ctx.env.is_empty() {
                scope.env = ctx.env.clone();
            }
            if !ctx.tenant.is_empty() {
                scope.tenant = ctx.tenant.clone();
            }
            if let Some(team) = &ctx.team {
                scope.team = Some(team.clone());
            }
            if let Some(user) = &ctx.user {
                scope.user = Some(user.clone());
            }
        }
        scope
    }
}

impl StateStoreHost for StateStoreHostImpl {
    fn read(
        &mut self,
        key: String,
        ctx: Option<WitTenantCtx>,
    ) -> std::result::Result<Vec<u8>, StateStoreError> {
        if !self.allow_state_read {
            return Err(StateStoreError {
                code: "state.read.denied".into(),
                message: "state store reads are disabled by manifest capability".into(),
            });
        }
        let scope = self.scope_for_ctx(ctx.as_ref());
        self.state_store
            .read(&scope, &key)
            .ok_or_else(|| StateStoreError {
                code: "state.read.miss".into(),
                message: format!("state key `{key}` not found"),
            })
    }

    fn write(
        &mut self,
        key: String,
        bytes: Vec<u8>,
        ctx: Option<WitTenantCtx>,
    ) -> std::result::Result<OpAck, StateStoreError> {
        if !self.allow_state_write {
            return Err(StateStoreError {
                code: "state.write.denied".into(),
                message: "state store writes are disabled by manifest capability".into(),
            });
        }
        let scope = self.scope_for_ctx(ctx.as_ref());
        self.state_store.write(&scope, &key, bytes);
        Ok(OpAck::Ok)
    }

    fn delete(
        &mut self,
        key: String,
        ctx: Option<WitTenantCtx>,
    ) -> std::result::Result<OpAck, StateStoreError> {
        if !self.allow_state_delete {
            return Err(StateStoreError {
                code: "state.delete.denied".into(),
                message: "state store deletes are disabled by manifest capability".into(),
            });
        }
        let scope = self.scope_for_ctx(ctx.as_ref());
        self.state_store.delete(&scope, &key);
        Ok(OpAck::Ok)
    }
}

pub struct SecretsStoreHostImpl {
    secrets: Arc<InMemorySecretsStore>,
}

impl SecretsStoreHostImpl {
    fn new(secrets: Arc<InMemorySecretsStore>) -> Self {
        Self { secrets }
    }
}

impl SecretsStoreHost for SecretsStoreHostImpl {
    fn get(
        &mut self,
        key: wasmtime::component::__internal::String,
    ) -> std::result::Result<Option<wasmtime::component::__internal::Vec<u8>>, SecretsError> {
        self.secrets.get(&key)
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.wasi_table,
        }
    }
}

struct HostLimits {
    max_memory_bytes: usize,
    hit: Arc<AtomicBool>,
}

impl HostLimits {
    fn new(max_memory_bytes: usize, hit: Arc<AtomicBool>) -> Self {
        Self {
            max_memory_bytes,
            hit,
        }
    }
}

impl ResourceLimiter for HostLimits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes {
            self.hit.store(true, Ordering::Relaxed);
            return Err(wasmtime::Error::msg(format!(
                "memory limit exceeded (requested {desired} bytes, max {})",
                self.max_memory_bytes
            )));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

#[derive(Clone)]
struct FixedWallClock {
    now: Duration,
    resolution: Duration,
}

impl FixedWallClock {
    fn new() -> Self {
        Self {
            now: Duration::from_secs(1_700_000_000),
            resolution: Duration::from_secs(1),
        }
    }
}

impl HostWallClock for FixedWallClock {
    fn resolution(&self) -> Duration {
        self.resolution
    }

    fn now(&self) -> Duration {
        self.now
    }
}

#[derive(Clone)]
struct FixedMonotonicClock {
    now: u64,
    resolution: u64,
}

impl FixedMonotonicClock {
    fn new() -> Self {
        Self {
            now: 0,
            resolution: 1,
        }
    }
}

impl HostMonotonicClock for FixedMonotonicClock {
    fn resolution(&self) -> u64 {
        self.resolution
    }

    fn now(&self) -> u64 {
        self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_interfaces_wasmtime::host_helpers::v1::state_store::TenantCtx as WitTenantCtx;
    use std::collections::{HashMap, HashSet};

    fn base_scope() -> StateScope {
        StateScope {
            env: "dev".into(),
            tenant: "tenant-a".into(),
            team: None,
            user: None,
            prefix: "test".into(),
        }
    }

    #[test]
    fn runner_host_returns_config_json_and_denies_http_when_disabled() {
        let mut runner = RunnerHostImpl::new(false, Some("{\"enabled\":true}".into()));
        assert_eq!(
            RunnerHost::kv_get(&mut runner, "config".into(), "json".into()).unwrap(),
            Some("{\"enabled\":true}".into())
        );
        assert!(matches!(
            RunnerHost::http_request(
                &mut runner,
                "GET".into(),
                "http://localhost".into(),
                vec![],
                None
            ),
            Ok(Err(message)) if message.contains("denied")
        ));
    }

    #[test]
    fn state_scope_overrides_from_tenant_context() {
        let state = StateStoreHostImpl::new(
            base_scope(),
            Arc::new(InMemoryStateStore::new()),
            true,
            true,
            true,
        );
        let ctx = WitTenantCtx {
            env: "prod".into(),
            tenant: "tenant-b".into(),
            tenant_id: "tenant-b".into(),
            team: Some("ops".into()),
            team_id: Some("ops".into()),
            user: Some("alice".into()),
            user_id: Some("alice".into()),
            trace_id: None,
            i18n_id: None,
            correlation_id: None,
            session_id: None,
            flow_id: None,
            node_id: None,
            provider_id: None,
            deadline_ms: None,
            attempt: 0,
            idempotency_key: None,
            impersonation: None,
            attributes: vec![],
        };

        let scope = state.scope_for_ctx(Some(&ctx));
        assert_eq!(scope.env, "prod");
        assert_eq!(scope.tenant, "tenant-b");
        assert_eq!(scope.team.as_deref(), Some("ops"));
        assert_eq!(scope.user.as_deref(), Some("alice"));
        assert_eq!(scope.prefix, "test");
    }

    #[test]
    fn host_limits_track_memory_limit_hits() {
        let hit = Arc::new(AtomicBool::new(false));
        let mut limits = HostLimits::new(64, hit.clone());
        assert!(limits.memory_growing(0, 32, None).unwrap());
        let err = limits
            .memory_growing(32, 128, None)
            .expect_err("should exceed limit");
        assert!(err.to_string().contains("memory limit exceeded"));
        assert!(hit.load(Ordering::Relaxed));
    }

    #[test]
    fn secrets_store_host_delegates_to_in_memory_store() {
        let secrets = Arc::new(
            InMemorySecretsStore::new(true, HashSet::from(["API_TOKEN".to_string()])).with_secrets(
                HashMap::from([("API_TOKEN".to_string(), "secret".to_string())]),
            ),
        );
        let mut host = SecretsStoreHostImpl::new(secrets);
        let bytes = SecretsStoreHost::get(&mut host, "API_TOKEN".into())
            .unwrap()
            .expect("secret");
        assert_eq!(bytes, b"secret");
    }
}
