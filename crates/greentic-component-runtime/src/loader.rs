use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use component_manifest::{CapabilityRef, CompiledExportSchema, ComponentInfo, WitCompat};
use greentic_interfaces_host::component::v0_6::exports::greentic::component::node::{
    ComponentDescriptor, GuestIndices,
};
use greentic_types::cbor::canonical;
use greentic_types::schemas::component::v0_6_0::ComponentDescribe;
use jsonschema::{Validator, validator_for};
use serde_json::{Map, Value, json};
use wasmtime::component::{Component as WasmComponent, Func, InstancePre, Val};
use wasmtime::{Config, Engine};

use crate::error::CompError;
use crate::host_imports::{HostState, build_linker};
use crate::policy::LoadPolicy;

const SELF_DESCRIBE_TAG: [u8; 3] = [0xd9, 0xd9, 0xf7];

#[derive(Debug, Clone)]
pub struct ComponentRef {
    pub name: String,
    pub locator: String,
}

pub struct Loader;

impl Default for Loader {
    fn default() -> Self {
        Self
    }
}

impl Loader {
    pub fn load(
        &self,
        cref: &ComponentRef,
        policy: &LoadPolicy,
    ) -> Result<ComponentHandle, CompError> {
        let artifact = policy
            .store
            .fetch_from_str(&cref.locator, &policy.verification)?;

        let engine = create_engine()?;
        let component = WasmComponent::from_binary(&engine, &artifact.bytes)?;

        let linker = build_linker(&engine, &policy.host)?;
        let instance_pre = linker.instantiate_pre(&component)?;
        let guest_indices = GuestIndices::new(&instance_pre)?;
        let host_state = HostState::empty(policy.host.clone());
        let mut store = wasmtime::Store::new(&engine, host_state);

        let instance = instance_pre.instantiate(&mut store)?;
        let guest = guest_indices.load(&mut store, &instance)?;
        let descriptor = guest.call_describe(&mut store)?;
        let config_schema_value =
            load_config_schema_from_describe(&instance, &mut store)?.unwrap_or_else(|| json!({}));
        let info = component_info_from_descriptor(&descriptor, config_schema_value.clone());
        let config_schema = validator_for(&config_schema_value)
            .map_err(|err| CompError::SchemaValidation(err.to_string()))?;

        Ok(ComponentHandle {
            inner: Arc::new(ComponentInner {
                cref: cref.clone(),
                info,
                config_schema: Arc::new(config_schema),
                engine,
                instance_pre,
                guest_indices,
                host_policy: policy.host.clone(),
                bindings: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn describe(&self, handle: &ComponentHandle) -> Result<ComponentInfo, CompError> {
        Ok(handle.inner.info.clone())
    }
}

fn component_info_from_descriptor(
    descriptor: &ComponentDescriptor,
    config_schema: Value,
) -> ComponentInfo {
    let capabilities = descriptor
        .capabilities
        .iter()
        .cloned()
        .map(CapabilityRef)
        .collect();
    let exports = descriptor
        .ops
        .iter()
        .map(|op| CompiledExportSchema {
            operation: op.name.clone(),
            description: op.summary.clone(),
            input_schema: None,
            output_schema: None,
        })
        .collect();

    let raw = json!({
        "name": descriptor.name,
        "description": descriptor.summary,
        "capabilities": descriptor.capabilities,
        "exports": descriptor.ops.iter().map(|op| json!({ "operation": op.name, "description": op.summary })).collect::<Vec<_>>(),
        "config_schema": config_schema,
        "secret_requirements": [],
        "wit_compat": {
            "package": "greentic:component",
            "min": "0.6.0"
        }
    });

    ComponentInfo {
        name: Some(descriptor.name.clone()),
        description: descriptor.summary.clone(),
        capabilities,
        exports,
        config_schema,
        secret_requirements: Vec::new(),
        wit_compat: WitCompat {
            package: "greentic:component".to_string(),
            min: "0.6.0".to_string(),
            max: None,
        },
        metadata: Map::new(),
        raw,
    }
}

fn load_config_schema_from_describe(
    instance: &wasmtime::component::Instance,
    store: &mut wasmtime::Store<HostState>,
) -> Result<Option<Value>, CompError> {
    let Some(interface_index) = resolve_interface_index(instance, store, "component-descriptor")
    else {
        return Ok(None);
    };
    let Some(func_index) =
        instance.get_export_index(&mut *store, Some(&interface_index), "describe")
    else {
        return Ok(None);
    };
    let func = instance.get_func(&mut *store, func_index).ok_or_else(|| {
        CompError::Runtime("component-descriptor.describe is not callable".into())
    })?;
    let describe_bytes = call_component_func(store, &func, &[]).and_then(|values| {
        values
            .first()
            .ok_or_else(|| CompError::Runtime("describe returned no values".into()))
            .and_then(val_to_bytes)
    })?;
    let payload = strip_self_describe_tag(&describe_bytes);
    let describe: ComponentDescribe = canonical::from_cbor(payload)
        .map_err(|err| CompError::SchemaValidation(err.to_string()))?;
    serde_json::to_value(describe.config_schema)
        .map(Some)
        .map_err(CompError::from)
}

fn resolve_interface_index(
    instance: &wasmtime::component::Instance,
    store: &mut wasmtime::Store<HostState>,
    interface: &str,
) -> Option<wasmtime::component::ComponentExportIndex> {
    for candidate in interface_candidates(interface) {
        if let Some(index) = instance.get_export_index(&mut *store, None, &candidate) {
            return Some(index);
        }
    }
    None
}

fn interface_candidates(interface: &str) -> [String; 3] {
    [
        interface.to_string(),
        format!("greentic:component/{interface}@0.6.0"),
        format!("greentic:component/{interface}"),
    ]
}

fn call_component_func(
    store: &mut wasmtime::Store<HostState>,
    func: &Func,
    params: &[Val],
) -> Result<Vec<Val>, CompError> {
    let results_len = func.ty(&mut *store).results().len();
    let mut results = vec![Val::Bool(false); results_len];
    func.call(&mut *store, params, &mut results)
        .map_err(|err| CompError::Runtime(format!("call failed: {err}")))?;
    Ok(results)
}

fn val_to_bytes(val: &Val) -> Result<Vec<u8>, CompError> {
    match val {
        Val::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Val::U8(byte) => out.push(*byte),
                    _ => {
                        return Err(CompError::Runtime(
                            "describe returned list with non-u8 items".to_string(),
                        ));
                    }
                }
            }
            Ok(out)
        }
        _ => Err(CompError::Runtime(
            "describe returned non-byte list payload".to_string(),
        )),
    }
}

fn strip_self_describe_tag(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&SELF_DESCRIBE_TAG) {
        &bytes[SELF_DESCRIBE_TAG.len()..]
    } else {
        bytes
    }
}

fn create_engine() -> Result<Engine, CompError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
    Engine::new(&config).map_err(|err| CompError::Runtime(err.to_string()))
}

pub struct ComponentHandle {
    pub(crate) inner: Arc<ComponentInner>,
}

pub(crate) struct ComponentInner {
    pub(crate) cref: ComponentRef,
    pub(crate) info: ComponentInfo,
    pub(crate) config_schema: Arc<Validator>,
    pub(crate) engine: Engine,
    pub(crate) instance_pre: InstancePre<HostState>,
    pub(crate) guest_indices: GuestIndices,
    pub(crate) host_policy: crate::policy::HostPolicy,
    pub(crate) bindings: Mutex<HashMap<String, TenantBinding>>,
}

#[derive(Debug, Clone)]
pub(crate) struct TenantBinding {
    pub config: Value,
    pub secrets: HashMap<String, Vec<u8>>,
}

impl ComponentHandle {
    pub fn info(&self) -> &ComponentInfo {
        &self.inner.info
    }

    pub fn cref(&self) -> &ComponentRef {
        &self.inner.cref
    }
}

impl Clone for ComponentHandle {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wasmtime::component::Val;

    fn descriptor_fixture() -> ComponentDescriptor {
        ComponentDescriptor {
            name: "fixture".to_string(),
            version: "0.1.0".to_string(),
            summary: Some("summary".to_string()),
            capabilities: vec!["telemetry".to_string()],
            ops: vec![],
            schemas: vec![],
            setup: None,
        }
    }

    #[test]
    fn descriptor_maps_to_component_info() {
        let config_schema = json!({"type":"object"});
        let info = component_info_from_descriptor(&descriptor_fixture(), config_schema.clone());
        assert_eq!(info.wit_compat.package, "greentic:component");
        assert_eq!(info.wit_compat.min, "0.6.0");
        assert_eq!(info.config_schema, config_schema);
        assert_eq!(info.capabilities.len(), 1);
    }

    #[test]
    fn strips_self_describe_tag_only_when_present() {
        let tagged = [SELF_DESCRIBE_TAG.as_slice(), &[1_u8, 2, 3]].concat();
        assert_eq!(strip_self_describe_tag(&tagged), &[1_u8, 2, 3]);
        assert_eq!(strip_self_describe_tag(&[7_u8, 8, 9]), &[7_u8, 8, 9]);
    }

    #[test]
    fn interface_candidates_cover_short_and_versioned_names() {
        let names = interface_candidates("component-descriptor");
        assert_eq!(names[0], "component-descriptor");
        assert_eq!(names[1], "greentic:component/component-descriptor@0.6.0");
        assert_eq!(names[2], "greentic:component/component-descriptor");
    }

    #[test]
    fn val_to_bytes_accepts_only_u8_lists() {
        let ok = val_to_bytes(&Val::List(vec![Val::U8(1), Val::U8(2), Val::U8(3)]))
            .expect("u8 list should decode");
        assert_eq!(ok, vec![1, 2, 3]);

        let err = val_to_bytes(&Val::List(vec![Val::U8(1), Val::Bool(true)]))
            .expect_err("mixed lists must be rejected");
        assert!(matches!(err, CompError::Runtime(message) if message.contains("non-u8")));

        let err = val_to_bytes(&Val::Bool(true)).expect_err("non-lists must be rejected");
        assert!(matches!(err, CompError::Runtime(message) if message.contains("non-byte list")));
    }

    #[test]
    fn create_engine_enables_component_model_support() {
        let engine = create_engine().expect("engine");
        let empty_component = b"\0asm\x0d\0\x01\0";
        WasmComponent::from_binary(&engine, empty_component).expect("component should parse");
    }
}
