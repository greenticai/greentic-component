//! `greentic-component manifest` subcommand.
//!
//! Loads a compiled WASM component with Wasmtime (component model), calls its
//! `greentic:component/descriptor@0.6.x` `describe()` export, decodes the CBOR
//! response, and writes a `component.manifest.json` to stdout or an `--output`
//! file.
//!
//! This is a generic developer tool — it works with any Greentic WASM component
//! that exports the descriptor interface, regardless of the component's purpose.
//!
//! The CBOR payload is decoded generically via `ciborium` so the tool handles
//! both the canonical `ComponentDescribe` schema (from `greentic-types`) and
//! older provider-specific describe payloads without requiring a specific struct
//! version to compile against.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use greentic_interfaces_wasmtime::host_helpers::v1::{
    HostFns, add_all_v1_to_linker, http_client, secrets_store, state_store,
};
use serde::Serialize;
use wasmtime::component::{Component, Instance, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
#[command(about = "Generate component.manifest.json from a WASM component's describe() export")]
pub struct ManifestArgs {
    /// Path to the compiled WASM component file.
    pub wasm_path: PathBuf,

    /// Override the version field (e.g. from Cargo workspace version).
    #[arg(long)]
    pub version: Option<String>,

    /// Write JSON to this file instead of stdout.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Manifest JSON types (output format)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ManifestJson {
    id: String,
    name: String,
    version: String,
    world: String,
    description: String,
    profiles: ManifestProfiles,
    config_schema: serde_json::Value,
    secret_requirements: Vec<ManifestSecretRequirement>,
    operations: Vec<ManifestOperation>,
    capabilities: ManifestCapabilities,
}

#[derive(Serialize, PartialEq, Debug)]
struct ManifestProfiles {
    default: String,
    supported: Vec<String>,
}

#[derive(Serialize, PartialEq, Debug)]
struct ManifestSecretRequirement {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct ManifestOperation {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ManifestCapabilities {
    wasi: serde_json::Value,
    host: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Generic CBOR-to-JSON describe payload mapping
//
// The describe() export returns CBOR. We decode it to serde_json::Value via
// ciborium so the tool is independent of the exact Rust struct version in
// greentic-types. This lets it handle both the canonical ComponentDescribe
// and older provider-specific DescribePayload formats.
// ---------------------------------------------------------------------------

fn decode_describe_cbor(bytes: &[u8]) -> Result<serde_json::Value> {
    let cbor_value: ciborium::Value =
        ciborium::de::from_reader(bytes).map_err(|err| anyhow!("CBOR decode failed: {err}"))?;
    cbor_to_json(&cbor_value)
}

/// Recursively convert a ciborium CBOR value to serde_json::Value.
fn cbor_to_json(value: &ciborium::Value) -> Result<serde_json::Value> {
    match value {
        ciborium::Value::Null => Ok(serde_json::Value::Null),
        ciborium::Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        ciborium::Value::Integer(i) => {
            let n: i128 = (*i).into();
            if let Ok(v) = i64::try_from(n) {
                Ok(serde_json::Value::Number(v.into()))
            } else if let Ok(v) = u64::try_from(n) {
                Ok(serde_json::Value::Number(v.into()))
            } else {
                Ok(serde_json::Value::String(n.to_string()))
            }
        }
        ciborium::Value::Float(f) => {
            let num = serde_json::Number::from_f64(*f)
                .ok_or_else(|| anyhow!("non-finite float in CBOR"))?;
            Ok(serde_json::Value::Number(num))
        }
        ciborium::Value::Bytes(b) => {
            // Represent byte strings as base64 for readability.
            use base64::Engine as _;
            Ok(serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(b),
            ))
        }
        ciborium::Value::Text(s) => Ok(serde_json::Value::String(s.clone())),
        ciborium::Value::Array(arr) => {
            let items: Result<Vec<_>> = arr.iter().map(cbor_to_json).collect();
            Ok(serde_json::Value::Array(items?))
        }
        ciborium::Value::Map(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                let key = match k {
                    ciborium::Value::Text(s) => s.clone(),
                    ciborium::Value::Integer(i) => {
                        let n: i128 = (*i).into();
                        n.to_string()
                    }
                    _ => format!("{k:?}"),
                };
                map.insert(key, cbor_to_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        ciborium::Value::Tag(_tag, inner) => cbor_to_json(inner),
        _ => Ok(serde_json::Value::Null),
    }
}

// ---------------------------------------------------------------------------
// Mapping from decoded JSON describe payload to ManifestJson
// ---------------------------------------------------------------------------

fn build_world_string(component_id: &str) -> String {
    if component_id.starts_with("greentic:component/") {
        component_id.to_string()
    } else {
        format!("greentic:component/{component_id}@0.6.1")
    }
}

/// Extract the component identifier from either the canonical `info.id` field
/// or the legacy `provider` field.
fn extract_component_id(payload: &serde_json::Value) -> String {
    // Canonical: payload.info.id
    if let Some(id) = payload
        .get("info")
        .and_then(|info| info.get("id"))
        .and_then(|v| v.as_str())
    {
        return id.to_string();
    }
    // Legacy: payload.provider
    if let Some(provider) = payload.get("provider").and_then(|v| v.as_str()) {
        return provider.to_string();
    }
    "unknown".to_string()
}

/// Extract the component version from either the canonical `info.version` field
/// or fall back to "0.0.0".
fn extract_component_version(payload: &serde_json::Value) -> String {
    if let Some(version) = payload
        .get("info")
        .and_then(|info| info.get("version"))
        .and_then(|v| v.as_str())
    {
        return version.to_string();
    }
    "0.0.0".to_string()
}

/// Extract operations from either the canonical or legacy format.
fn extract_operations(payload: &serde_json::Value) -> Vec<ManifestOperation> {
    let ops = match payload.get("operations").and_then(|v| v.as_array()) {
        Some(ops) => ops,
        None => return vec![],
    };

    ops.iter()
        .filter_map(|op| {
            // Canonical format uses `id`, legacy uses `name`
            let name = op
                .get("id")
                .or_else(|| op.get("name"))
                .and_then(|v| v.as_str())?;
            Some(ManifestOperation {
                name: name.to_string(),
                input_schema: None,
                output_schema: None,
            })
        })
        .collect()
}

/// Extract profiles from the describe payload.
fn extract_profiles(payload: &serde_json::Value) -> ManifestProfiles {
    let default_profiles = ManifestProfiles {
        default: "default".into(),
        supported: vec!["default".into()],
    };

    let Some(profiles) = payload.get("profiles") else {
        return default_profiles;
    };

    if profiles.is_null() {
        return default_profiles;
    }

    let default = profiles
        .get("default")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let supported = profiles
        .get("supported")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec!["default".into()]);

    let supported = if supported.is_empty() {
        vec!["default".into()]
    } else {
        supported
    };

    ManifestProfiles { default, supported }
}

/// Extract secret requirements from the describe payload.
fn extract_secret_requirements(payload: &serde_json::Value) -> Vec<ManifestSecretRequirement> {
    let Some(secrets) = payload
        .get("secret_requirements")
        .and_then(|v| v.as_array())
    else {
        return vec![];
    };

    secrets
        .iter()
        .filter_map(|secret| {
            let name = secret.get("key").and_then(|v| v.as_str())?;
            let scope = secret
                .get("scope")
                .and_then(|v| v.as_str())
                .map(String::from);
            let description = secret
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(ManifestSecretRequirement {
                name: name.to_string(),
                scope,
                description,
            })
        })
        .collect()
}

/// Extract capabilities from the describe payload and convert to the manifest
/// JSON structure.
fn extract_capabilities(payload: &serde_json::Value) -> ManifestCapabilities {
    let empty = ManifestCapabilities {
        wasi: serde_json::json!({}),
        host: serde_json::json!({}),
    };

    let Some(caps) = payload.get("capabilities") else {
        return empty;
    };

    if caps.is_null() {
        return empty;
    }

    // WASI capabilities
    let mut wasi = serde_json::Map::new();
    if let Some(wasi_caps) = caps.get("wasi") {
        if wasi_caps.get("random").and_then(|v| v.as_bool()) == Some(true) {
            wasi.insert("random".into(), serde_json::json!(true));
        }
        if wasi_caps.get("clocks").and_then(|v| v.as_bool()) == Some(true) {
            wasi.insert("clocks".into(), serde_json::json!(true));
        }
    }

    // Host capabilities
    let mut host = serde_json::Map::new();
    if let Some(host_caps) = caps.get("host") {
        if let Some(state) = host_caps.get("state") {
            let mut state_map = serde_json::Map::new();
            if state.get("read").and_then(|v| v.as_bool()) == Some(true) {
                state_map.insert("read".into(), serde_json::json!(true));
            }
            if state.get("write").and_then(|v| v.as_bool()) == Some(true) {
                state_map.insert("write".into(), serde_json::json!(true));
            }
            if !state_map.is_empty() {
                host.insert("state".into(), serde_json::Value::Object(state_map));
            }
        }
        if let Some(secrets) = host_caps.get("secrets")
            && let Some(required) = secrets.get("required").and_then(|v| v.as_array())
        {
            let entries: Vec<serde_json::Value> = required
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "key": s.get("key").and_then(|v| v.as_str()).unwrap_or(""),
                        "required": s.get("required").and_then(|v| v.as_bool()).unwrap_or(true),
                    })
                })
                .collect();
            host.insert("secrets".into(), serde_json::json!({ "required": entries }));
        }
        if let Some(messaging) = host_caps.get("messaging") {
            let mut messaging_map = serde_json::Map::new();
            if messaging.get("inbound").and_then(|v| v.as_bool()) == Some(true) {
                messaging_map.insert("inbound".into(), serde_json::json!(true));
            }
            if messaging.get("outbound").and_then(|v| v.as_bool()) == Some(true) {
                messaging_map.insert("outbound".into(), serde_json::json!(true));
            }
            if !messaging_map.is_empty() {
                host.insert("messaging".into(), serde_json::Value::Object(messaging_map));
            }
        }
        if let Some(events) = host_caps.get("events") {
            let mut events_map = serde_json::Map::new();
            if events.get("inbound").and_then(|v| v.as_bool()) == Some(true) {
                events_map.insert("inbound".into(), serde_json::json!(true));
            }
            if events.get("outbound").and_then(|v| v.as_bool()) == Some(true) {
                events_map.insert("outbound".into(), serde_json::json!(true));
            }
            if !events_map.is_empty() {
                host.insert("events".into(), serde_json::Value::Object(events_map));
            }
        }
        if let Some(http) = host_caps.get("http") {
            let mut http_map = serde_json::Map::new();
            if http.get("client").and_then(|v| v.as_bool()) == Some(true) {
                http_map.insert("client".into(), serde_json::json!(true));
            }
            if http.get("server").and_then(|v| v.as_bool()) == Some(true) {
                http_map.insert("server".into(), serde_json::json!(true));
            }
            if !http_map.is_empty() {
                host.insert("http".into(), serde_json::Value::Object(http_map));
            }
        }
    }

    ManifestCapabilities {
        wasi: serde_json::Value::Object(wasi),
        host: serde_json::Value::Object(host),
    }
}

fn map_to_manifest(payload: &serde_json::Value, version_override: String) -> ManifestJson {
    let component_id = extract_component_id(payload);
    let component_version = extract_component_version(payload);

    let version = if version_override == "0.0.0" {
        component_version
    } else {
        version_override
    };

    ManifestJson {
        id: component_id.clone(),
        name: component_id.clone(),
        version,
        world: build_world_string(&component_id),
        description: String::new(),
        profiles: extract_profiles(payload),
        config_schema: serde_json::json!({}),
        secret_requirements: extract_secret_requirements(payload),
        operations: extract_operations(payload),
        capabilities: extract_capabilities(payload),
    }
}

// ---------------------------------------------------------------------------
// WASM loading and describe() invocation
// ---------------------------------------------------------------------------

/// Candidate interface export names for the descriptor interface.
const DESCRIPTOR_CANDIDATES: &[&str] = &[
    "greentic:component/descriptor@0.6.1",
    "greentic:component/descriptor@0.6.0",
    "greentic:component/descriptor",
    "component-descriptor",
];

fn call_describe(wasm_path: &std::path::Path) -> Result<Vec<u8>> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    let engine =
        Engine::new(&config).map_err(|err| anyhow!("failed to create wasmtime engine: {err}"))?;
    let component = Component::from_file(&engine, wasm_path).map_err(|err| {
        anyhow!(
            "failed to load WASM component {}: {err}",
            wasm_path.display()
        )
    })?;

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|err| anyhow!("failed to add WASI to linker: {err}"))?;
    add_greentic_host_stubs(&mut linker)?;

    let state = StubHostState::new();
    let mut store = Store::new(&engine, state);
    // Limit fuel to prevent runaway describe() calls (10M instructions is generous).
    store
        .set_fuel(10_000_000)
        .map_err(|err| anyhow!("failed to set fuel limit: {err}"))?;
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|err| anyhow!("failed to instantiate component: {err}"))?;

    let describe_index = find_describe_export(&instance, &mut store).ok_or_else(|| {
        anyhow!(
            "WASM component does not export a describe() function under any known \
             descriptor interface ({:?})",
            DESCRIPTOR_CANDIDATES
        )
    })?;

    let func = instance
        .get_func(&mut store, describe_index)
        .ok_or_else(|| anyhow!("describe export is not callable"))?;

    let mut results =
        vec![wasmtime::component::Val::Bool(false); func.ty(&mut store).results().len()];
    func.call(&mut store, &[], &mut results)
        .map_err(|err| anyhow!("describe() call failed: {err}"))?;

    let val = results
        .first()
        .ok_or_else(|| anyhow!("describe returned no value"))?;
    val_to_bytes(val)
}

fn find_describe_export(
    instance: &Instance,
    store: &mut Store<StubHostState>,
) -> Option<wasmtime::component::ComponentExportIndex> {
    for candidate in DESCRIPTOR_CANDIDATES {
        if let Some(interface_index) = instance.get_export_index(&mut *store, None, candidate)
            && let Some(describe_index) =
                instance.get_export_index(&mut *store, Some(&interface_index), "describe")
        {
            return Some(describe_index);
        }
    }
    None
}

fn val_to_bytes(val: &wasmtime::component::Val) -> Result<Vec<u8>> {
    match val {
        wasmtime::component::Val::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    wasmtime::component::Val::U8(byte) => out.push(*byte),
                    _ => return Err(anyhow!("expected list<u8>")),
                }
            }
            Ok(out)
        }
        _ => Err(anyhow!("expected list<u8>")),
    }
}

/// Strip the CBOR self-describe tag (0xd9d9f7) if present.
fn strip_self_describe_tag(bytes: &[u8]) -> &[u8] {
    const SELF_DESCRIBE_TAG: [u8; 3] = [0xd9, 0xd9, 0xf7];
    if bytes.starts_with(&SELF_DESCRIBE_TAG) {
        &bytes[SELF_DESCRIBE_TAG.len()..]
    } else {
        bytes
    }
}

// ---------------------------------------------------------------------------
// Stub host state (minimal implementations for instantiation)
// ---------------------------------------------------------------------------

struct StubHostState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
}

impl StubHostState {
    fn new() -> Self {
        Self {
            wasi_ctx: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for StubHostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

impl http_client::HttpClientHostV1_1 for StubHostState {
    fn send(
        &mut self,
        _req: http_client::RequestV1_1,
        _opts: Option<http_client::RequestOptionsV1_1>,
        _ctx: Option<http_client::TenantCtxV1_1>,
    ) -> Result<http_client::ResponseV1_1, http_client::HttpClientErrorV1_1> {
        Err(http_client::HttpClientErrorV1_1 {
            code: "stub".into(),
            message: "HTTP client not available in manifest extraction mode".into(),
        })
    }
}

impl secrets_store::SecretsStoreHostV1_1 for StubHostState {
    fn get(&mut self, _key: String) -> Result<Option<Vec<u8>>, secrets_store::SecretsErrorV1_1> {
        Ok(None)
    }

    fn put(&mut self, _key: String, _value: Vec<u8>) {}
}

impl state_store::StateStoreHost for StubHostState {
    fn read(
        &mut self,
        _key: state_store::StateKey,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<Vec<u8>, state_store::StateStoreError> {
        Err(state_store::StateStoreError {
            code: "stub".into(),
            message: "state store not available in manifest extraction mode".into(),
        })
    }

    fn write(
        &mut self,
        _key: state_store::StateKey,
        _bytes: Vec<u8>,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<state_store::OpAck, state_store::StateStoreError> {
        Ok(state_store::OpAck::Ok)
    }

    fn delete(
        &mut self,
        _key: state_store::StateKey,
        _ctx: Option<state_store::TenantCtx>,
    ) -> Result<state_store::OpAck, state_store::StateStoreError> {
        Ok(state_store::OpAck::Ok)
    }
}

/// Register greentic host interfaces with stub implementations so that
/// components importing these interfaces can be instantiated. The stubs are
/// never called during `describe()` — they only satisfy the linker.
fn add_greentic_host_stubs(linker: &mut Linker<StubHostState>) -> Result<()> {
    add_all_v1_to_linker(
        linker,
        HostFns {
            http_client_v1_1: Some(|state| state as &mut dyn http_client::HttpClientHostV1_1),
            http_client: None,
            oauth_broker: None,
            runner_host_http: None,
            runner_host_kv: None,
            telemetry_logger: None,
            state_store: Some(|state| state as &mut dyn state_store::StateStoreHost),
            secrets_store_v1_1: Some(|state| state as &mut dyn secrets_store::SecretsStoreHostV1_1),
            secrets_store: None,
        },
    )
    .map_err(|err| anyhow!("failed to add greentic host stubs to linker: {err}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

pub fn run(args: ManifestArgs) -> Result<()> {
    if !args.wasm_path.exists() {
        anyhow::bail!("WASM file does not exist: {}", args.wasm_path.display());
    }

    let raw_bytes = call_describe(&args.wasm_path)?;
    let payload_bytes = strip_self_describe_tag(&raw_bytes);

    let payload = decode_describe_cbor(payload_bytes)?;

    let version = args.version.unwrap_or_else(|| "0.0.0".into());
    let manifest = map_to_manifest(&payload, version);

    let json_output =
        serde_json::to_string_pretty(&manifest).context("failed to serialize manifest JSON")?;

    match args.output {
        Some(ref output_path) => {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create output directory: {}", parent.display())
                })?;
            }
            std::fs::write(output_path, format!("{json_output}\n")).with_context(|| {
                format!("failed to write manifest to {}", output_path.display())
            })?;
            eprintln!("Wrote {}", output_path.display());
        }
        None => {
            println!("{json_output}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_payload() -> serde_json::Value {
        serde_json::json!({
            "info": {
                "id": "messaging-provider-test",
                "version": "0.1.0",
                "role": "provider"
            },
            "provided_capabilities": [],
            "required_capabilities": [],
            "metadata": {},
            "operations": [
                {
                    "id": "send",
                    "display_name": { "key": "test.op.send.title" },
                    "input": { "schema": { "type": "bool" } },
                    "output": { "schema": { "type": "bool" } },
                    "defaults": {},
                    "redactions": [],
                    "constraints": {},
                    "schema_hash": "abc123"
                }
            ],
            "config_schema": { "type": "bool" },
            "capabilities": {
                "wasi": { "random": false, "clocks": false },
                "host": {
                    "state": { "read": true, "write": true },
                    "http": { "client": true, "server": false }
                }
            },
            "profiles": {
                "default": "default",
                "supported": ["default"]
            },
            "secret_requirements": [
                {
                    "key": "TEST_TOKEN",
                    "required": true,
                    "description": "Test token",
                    "scope": "tenant"
                }
            ]
        })
    }

    fn legacy_payload() -> serde_json::Value {
        serde_json::json!({
            "provider": "messaging-provider-legacy",
            "world": "component-v0-v6-v0",
            "operations": [
                { "name": "send", "title": { "key": "t" }, "description": { "key": "d" } }
            ],
            "capabilities": {
                "wasi": { "random": true, "clocks": false },
                "host": {
                    "state": { "read": true, "write": false },
                    "messaging": { "inbound": true, "outbound": true }
                }
            },
            "profiles": {
                "default": "default",
                "supported": ["default"]
            },
            "secret_requirements": [
                { "key": "BOT_TOKEN", "required": true, "scope": "tenant" }
            ]
        })
    }

    #[test]
    fn maps_canonical_payload_to_manifest() {
        let manifest = map_to_manifest(&canonical_payload(), "1.2.3".into());
        assert_eq!(manifest.id, "messaging-provider-test");
        assert_eq!(manifest.name, "messaging-provider-test");
        assert_eq!(manifest.version, "1.2.3");
        assert_eq!(
            manifest.world,
            "greentic:component/messaging-provider-test@0.6.1"
        );
        assert_eq!(manifest.profiles.default, "default");
        assert_eq!(manifest.profiles.supported, vec!["default"]);
        assert_eq!(manifest.secret_requirements.len(), 1);
        assert_eq!(manifest.secret_requirements[0].name, "TEST_TOKEN");
        assert_eq!(
            manifest.secret_requirements[0].scope.as_deref(),
            Some("tenant")
        );
        assert_eq!(manifest.operations.len(), 1);
        assert_eq!(manifest.operations[0].name, "send");
    }

    #[test]
    fn maps_legacy_payload_to_manifest() {
        let manifest = map_to_manifest(&legacy_payload(), "0.5.0".into());
        assert_eq!(manifest.id, "messaging-provider-legacy");
        assert_eq!(manifest.operations.len(), 1);
        assert_eq!(manifest.operations[0].name, "send");
        assert_eq!(manifest.secret_requirements.len(), 1);
        assert_eq!(manifest.secret_requirements[0].name, "BOT_TOKEN");
    }

    #[test]
    fn maps_capabilities_with_state_and_http() {
        let manifest = map_to_manifest(&canonical_payload(), "0.0.0".into());
        let host = &manifest.capabilities.host;
        let state = host.get("state").expect("missing state capability");
        assert_eq!(state.get("read").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(state.get("write").and_then(|v| v.as_bool()), Some(true));
        let http = host.get("http").expect("missing http capability");
        assert_eq!(http.get("client").and_then(|v| v.as_bool()), Some(true));
        assert!(http.get("server").is_none());
    }

    #[test]
    fn maps_none_capabilities_to_empty_objects() {
        let mut payload = canonical_payload();
        payload["capabilities"] = serde_json::Value::Null;
        let manifest = map_to_manifest(&payload, "0.0.0".into());
        assert_eq!(manifest.capabilities.wasi, serde_json::json!({}));
        assert_eq!(manifest.capabilities.host, serde_json::json!({}));
    }

    #[test]
    fn maps_missing_capabilities_to_empty_objects() {
        let payload = serde_json::json!({
            "info": { "id": "test", "version": "1.0.0", "role": "provider" },
            "operations": []
        });
        let manifest = map_to_manifest(&payload, "1.0.0".into());
        assert_eq!(manifest.capabilities.wasi, serde_json::json!({}));
        assert_eq!(manifest.capabilities.host, serde_json::json!({}));
    }

    #[test]
    fn builds_world_string_with_prefix_when_missing() {
        assert_eq!(
            build_world_string("component-v0-v6-v0"),
            "greentic:component/component-v0-v6-v0@0.6.1"
        );
    }

    #[test]
    fn preserves_world_string_when_already_prefixed() {
        let world = "greentic:component/custom-world@1.0.0";
        assert_eq!(build_world_string(world), world);
    }

    #[test]
    fn maps_empty_profiles_to_defaults() {
        let mut payload = canonical_payload();
        payload.as_object_mut().unwrap().remove("profiles");
        let manifest = map_to_manifest(&payload, "0.0.0".into());
        assert_eq!(manifest.profiles.default, "default");
        assert_eq!(manifest.profiles.supported, vec!["default"]);
    }

    #[test]
    fn uses_describe_version_when_override_is_zero() {
        let manifest = map_to_manifest(&canonical_payload(), "0.0.0".into());
        assert_eq!(manifest.version, "0.1.0");
    }

    #[test]
    fn uses_override_version_when_provided() {
        let manifest = map_to_manifest(&canonical_payload(), "2.0.0".into());
        assert_eq!(manifest.version, "2.0.0");
    }

    #[test]
    fn extract_operations_handles_canonical_and_legacy() {
        let canonical = serde_json::json!({ "operations": [{ "id": "run" }] });
        let legacy = serde_json::json!({ "operations": [{ "name": "send" }] });
        assert_eq!(extract_operations(&canonical)[0].name, "run");
        assert_eq!(extract_operations(&legacy)[0].name, "send");
    }

    #[test]
    fn extract_component_id_handles_canonical_and_legacy() {
        let canonical = serde_json::json!({ "info": { "id": "test-comp" } });
        let legacy = serde_json::json!({ "provider": "test-provider" });
        let neither = serde_json::json!({});
        assert_eq!(extract_component_id(&canonical), "test-comp");
        assert_eq!(extract_component_id(&legacy), "test-provider");
        assert_eq!(extract_component_id(&neither), "unknown");
    }

    #[test]
    fn cbor_to_json_roundtrip() {
        let original = serde_json::json!({
            "info": { "id": "test", "version": "1.0.0" },
            "operations": [{ "id": "run" }],
            "flag": true,
            "count": 42
        });

        let mut cbor_bytes = Vec::new();
        ciborium::ser::into_writer(&original, &mut cbor_bytes).unwrap();
        let decoded = decode_describe_cbor(&cbor_bytes).unwrap();

        assert_eq!(
            decoded.get("info").unwrap().get("id").unwrap().as_str(),
            Some("test")
        );
        assert_eq!(decoded.get("flag").unwrap().as_bool(), Some(true));
        assert_eq!(decoded.get("count").unwrap().as_i64(), Some(42));
    }
}
