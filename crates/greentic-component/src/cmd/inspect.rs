use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Parser};
use serde::Serialize;
use serde_json::Value;
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use super::path::strip_file_scheme;
use crate::describe::{DescribePayload, from_wit_world};
use crate::embedded_compare::{
    EmbeddedManifestComparisonReport, compare_embedded_with_describe,
    compare_embedded_with_manifest,
};
use crate::embedded_descriptor::{
    EMBEDDED_COMPONENT_MANIFEST_SECTION_V1, read_and_verify_embedded_component_manifest_section_v1,
};
use crate::{ComponentError, PreparedComponent, parse_manifest, prepare_component_with_manifest};
use greentic_types::cbor::canonical;
use greentic_types::schemas::common::schema_ir::{AdditionalProperties, SchemaIr};
use greentic_types::schemas::component::v0_6_0::{ComponentDescribe, schema_hash};

#[derive(Args, Debug, Clone)]
#[command(about = "Inspect a Greentic component artifact")]
pub struct InspectArgs {
    /// Path or identifier resolvable by the loader
    #[arg(value_name = "TARGET", required_unless_present = "describe")]
    pub target: Option<String>,
    /// Explicit path to component.manifest.json when it is not adjacent to the wasm
    #[arg(long)]
    pub manifest: Option<PathBuf>,
    /// Inspect a pre-generated describe CBOR file (skip WASM execution)
    #[arg(long)]
    pub describe: Option<PathBuf>,
    /// Emit structured JSON instead of human output
    #[arg(long)]
    pub json: bool,
    /// Verify schema_hash values against typed SchemaIR
    #[arg(long)]
    pub verify: bool,
    /// Treat warnings as errors
    #[arg(long)]
    pub strict: bool,
}

#[derive(Parser, Debug)]
struct InspectCli {
    #[command(flatten)]
    args: InspectArgs,
}

pub fn parse_from_cli() -> InspectArgs {
    InspectCli::parse().args
}

#[derive(Default)]
pub struct InspectResult {
    pub warnings: Vec<String>,
}

pub fn run(args: &InspectArgs) -> Result<InspectResult, ComponentError> {
    if args.describe.is_some() {
        return inspect_describe(args);
    }

    if should_inspect_wasm_artifact(args) {
        return inspect_artifact(args);
    }

    let target = args
        .target
        .as_ref()
        .ok_or_else(|| ComponentError::Doctor("inspect target is required".to_string()))?;
    let manifest_override = args.manifest.as_deref().map(strip_file_scheme);
    let prepared = prepare_component_with_manifest(target, manifest_override.as_deref())?;
    if args.json {
        let json = serde_json::to_string_pretty(&build_report(&prepared))
            .expect("serializing inspect report");
        println!("{json}");
    } else {
        println!("component: {}", prepared.manifest.id.as_str());
        println!("  wasm: {}", prepared.wasm_path.display());
        println!("  world ok: {}", prepared.world_ok);
        println!("  hash: {}", prepared.wasm_hash);
        println!("  supports: {:?}", prepared.manifest.supports);
        println!(
            "  profiles: default={:?} supported={:?}",
            prepared.manifest.profiles.default, prepared.manifest.profiles.supported
        );
        println!(
            "  lifecycle: init={} health={} shutdown={}",
            prepared.lifecycle.init, prepared.lifecycle.health, prepared.lifecycle.shutdown
        );
        let caps = &prepared.manifest.capabilities;
        println!(
            "  capabilities: wasi(fs={}, env={}, random={}, clocks={}) host(secrets={}, state={}, messaging={}, events={}, http={}, telemetry={}, iac={})",
            caps.wasi.filesystem.is_some(),
            caps.wasi.env.is_some(),
            caps.wasi.random,
            caps.wasi.clocks,
            caps.host.secrets.is_some(),
            caps.host.state.is_some(),
            caps.host.messaging.is_some(),
            caps.host.events.is_some(),
            caps.host.http.is_some(),
            caps.host.telemetry.is_some(),
            caps.host.iac.is_some(),
        );
        println!(
            "  limits: {}",
            prepared
                .manifest
                .limits
                .as_ref()
                .map(|l| format!("{} MB / {} ms", l.memory_mb, l.wall_time_ms))
                .unwrap_or_else(|| "default".into())
        );
        println!(
            "  telemetry prefix: {}",
            prepared
                .manifest
                .telemetry
                .as_ref()
                .map(|t| t.span_prefix.as_str())
                .unwrap_or("<none>")
        );
        println!("  describe versions: {}", prepared.describe.versions.len());
        println!("  redaction paths: {}", prepared.redaction_paths().len());
        println!("  defaults applied: {}", prepared.defaults_applied().len());
    }
    Ok(InspectResult::default())
}

#[derive(Debug, Serialize)]
struct EmbeddedInspectStatus {
    present: bool,
    section_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    envelope_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_hash_blake3: Option<String>,
    hash_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest: Option<crate::embedded_descriptor::EmbeddedComponentManifestV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compare_manifest: Option<EmbeddedManifestComparisonReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compare_describe: Option<EmbeddedManifestComparisonReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ArtifactInspectReport {
    wasm_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest: Option<ArtifactManifestStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    describe: Option<ArtifactDescribeStatus>,
    embedded: EmbeddedInspectStatus,
}

#[derive(Debug, Serialize)]
struct ArtifactManifestStatus {
    path: PathBuf,
    component_id: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    compare_embedded: Option<EmbeddedManifestComparisonReport>,
}

#[derive(Debug, Serialize)]
struct ArtifactDescribeStatus {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    world: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    versions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compare_embedded: Option<EmbeddedManifestComparisonReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

fn inspect_artifact(args: &InspectArgs) -> Result<InspectResult, ComponentError> {
    let target = args
        .target
        .as_ref()
        .ok_or_else(|| ComponentError::Doctor("inspect target is required".to_string()))?;
    let wasm_path = resolve_wasm_path(target).map_err(ComponentError::Doctor)?;
    let manifest_path = args
        .manifest
        .clone()
        .or_else(|| discover_manifest_path(&wasm_path, Path::new(target)));
    let wasm_bytes = fs::read(&wasm_path)
        .map_err(|err| ComponentError::Doctor(format!("failed to read wasm: {err}")))?;
    let mut warnings = Vec::new();
    let verified =
        read_and_verify_embedded_component_manifest_section_v1(&wasm_bytes).map_err(|err| {
            ComponentError::Doctor(format!("failed to read embedded manifest: {err}"))
        })?;

    let mut compare_manifest = None;
    let mut compare_describe = None;
    let mut envelope_version = None;
    let mut envelope_kind = None;
    let mut payload_hash_blake3 = None;
    let mut manifest = None;
    let mut external_manifest_summary = None;
    let mut describe_status = None;
    let present = verified.is_some();
    let hash_verified = verified.is_some();

    if let Some(manifest_path) = manifest_path.as_ref() {
        let raw = fs::read_to_string(manifest_path).map_err(|err| {
            ComponentError::Doctor(format!(
                "failed to read manifest {}: {err}",
                manifest_path.display()
            ))
        })?;
        let parsed = parse_manifest(&raw).map_err(|err| {
            ComponentError::Doctor(format!(
                "failed to parse manifest {}: {err}",
                manifest_path.display()
            ))
        })?;
        external_manifest_summary =
            Some((parsed.id.as_str().to_string(), parsed.version.to_string()));
        if let Some(verified) = verified.as_ref() {
            compare_manifest = Some(compare_embedded_with_manifest(&verified.manifest, &parsed));
        }
    }

    if let Some(verified) = verified {
        envelope_version = Some(verified.envelope.version);
        envelope_kind = Some(verified.envelope.kind.clone());
        payload_hash_blake3 = Some(verified.envelope.payload_hash_blake3.clone());
        manifest = Some(verified.manifest.clone());
        match call_describe(&wasm_path) {
            Ok(bytes) => {
                let payload = strip_self_describe_tag(&bytes);
                match canonical::from_cbor::<ComponentDescribe>(payload) {
                    Ok(describe) => {
                        let operation_count = describe.operations.len();
                        let describe_id = describe.info.id.clone();
                        describe_status = Some(ArtifactDescribeStatus {
                            status: "available".to_string(),
                            source: Some("export".to_string()),
                            name: Some(describe_id),
                            schema_id: None,
                            world: None,
                            versions: None,
                            version_count: None,
                            function_count: None,
                            operation_count: Some(operation_count),
                            compare_embedded: None,
                            reason: None,
                        });
                        compare_describe = Some(compare_embedded_with_describe(
                            &verified.manifest,
                            &describe,
                        ));
                    }
                    Err(err) => {
                        let reason = format!("decode failed: {err}");
                        warnings.push(format!("describe {reason}"));
                        describe_status = Some(ArtifactDescribeStatus {
                            status: "unavailable".to_string(),
                            source: Some("export".to_string()),
                            name: None,
                            schema_id: None,
                            world: None,
                            versions: None,
                            version_count: None,
                            function_count: None,
                            operation_count: None,
                            compare_embedded: None,
                            reason: Some(reason),
                        });
                    }
                }
            }
            Err(err) => {
                if err.contains("missing export interface component-descriptor") {
                    match from_wit_world(&wasm_path, "greentic:component/component@0.6.0") {
                        Ok(payload) => {
                            let function_count = payload
                                .versions
                                .first()
                                .and_then(|version| version.schema.get("functions"))
                                .and_then(|functions| functions.as_array())
                                .map(|functions| functions.len());
                            let world = payload
                                .versions
                                .first()
                                .and_then(|version| version.schema.get("world"))
                                .and_then(|world| world.as_str())
                                .map(str::to_string);
                            let versions = payload
                                .versions
                                .iter()
                                .map(|version| version.version.to_string())
                                .collect::<Vec<_>>();
                            describe_status = Some(ArtifactDescribeStatus {
                                status: "available".to_string(),
                                source: Some("wit-world".to_string()),
                                name: Some(payload.name),
                                schema_id: payload.schema_id,
                                world,
                                versions: Some(versions),
                                version_count: Some(payload.versions.len()),
                                function_count,
                                operation_count: None,
                                compare_embedded: None,
                                reason: Some("derived from exported WIT world".to_string()),
                            });
                        }
                        Err(fallback_err) => {
                            describe_status = Some(ArtifactDescribeStatus {
                                status: "unavailable".to_string(),
                                source: Some("wit-world".to_string()),
                                name: None,
                                schema_id: None,
                                world: None,
                                versions: None,
                                version_count: None,
                                function_count: None,
                                operation_count: None,
                                compare_embedded: None,
                                reason: Some(format!(
                                    "missing export interface component-descriptor; WIT fallback failed: {fallback_err}"
                                )),
                            });
                        }
                    }
                } else {
                    warnings.push(format!("describe unavailable: {err}"));
                    describe_status = Some(ArtifactDescribeStatus {
                        status: "unavailable".to_string(),
                        source: Some("export".to_string()),
                        name: None,
                        schema_id: None,
                        world: None,
                        versions: None,
                        version_count: None,
                        function_count: None,
                        operation_count: None,
                        compare_embedded: None,
                        reason: Some(err),
                    });
                }
            }
        }
    }

    if let (Some(compare), Some(status)) = (compare_describe.clone(), describe_status.as_mut()) {
        status.compare_embedded = Some(compare);
    }

    let report = ArtifactInspectReport {
        wasm_path,
        manifest: manifest_path.as_ref().and_then(|path| {
            external_manifest_summary
                .as_ref()
                .map(|(id, version)| ArtifactManifestStatus {
                    path: path.clone(),
                    component_id: id.clone(),
                    version: version.clone(),
                    compare_embedded: compare_manifest.clone(),
                })
        }),
        describe: describe_status,
        embedded: EmbeddedInspectStatus {
            present,
            section_name: EMBEDDED_COMPONENT_MANIFEST_SECTION_V1.to_string(),
            envelope_version,
            envelope_kind,
            payload_hash_blake3,
            hash_verified,
            manifest,
            compare_manifest,
            compare_describe,
            warnings: warnings.clone(),
        },
    };

    if args.json {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|err| ComponentError::Doctor(format!("failed to encode json: {err}")))?;
        println!("{json}");
    } else {
        println!("wasm: {}", report.wasm_path.display());
        if let Some(manifest) = &report.manifest {
            println!("manifest: {}", manifest.path.display());
            println!("  component: {}", manifest.component_id);
            println!("  version: {}", manifest.version);
            if let Some(compare) = &manifest.compare_embedded {
                println!("  embedded vs manifest: {:?}", compare.overall);
            }
        }
        println!(
            "embedded manifest: {}",
            if report.embedded.present {
                "present"
            } else {
                "missing"
            }
        );
        println!("  section: {}", report.embedded.section_name);
        if let Some(version) = report.embedded.envelope_version {
            println!("  envelope version: {version}");
        }
        if let Some(kind) = &report.embedded.envelope_kind {
            println!("  kind: {kind}");
        }
        if let Some(hash) = &report.embedded.payload_hash_blake3 {
            println!("  payload hash: {hash}");
        }
        println!("  hash verified: {}", report.embedded.hash_verified);
        if let Some(manifest) = &report.embedded.manifest {
            println!("  component: {}", manifest.id);
            println!("  name: {}", manifest.name);
            println!("  version: {}", manifest.version);
            println!("  world: {}", manifest.world);
            println!("  operations: {}", manifest.operations.len());
            let operation_names = manifest
                .operations
                .iter()
                .map(|op| op.name.as_str())
                .collect::<Vec<_>>();
            if !operation_names.is_empty() {
                println!("  operation names: {}", operation_names.join(", "));
            }
            if let Some(default_operation) = &manifest.default_operation {
                println!("  default operation: {default_operation}");
            }
            if !manifest.supports.is_empty() {
                println!("  supports: {:?}", manifest.supports);
            }
            println!("  capabilities: {:?}", manifest.capabilities);
            println!(
                "  secret requirements: {}",
                manifest.secret_requirements.len()
            );
            println!("  profiles: {:?}", manifest.profiles);
            if let Some(limits) = &manifest.limits {
                println!(
                    "  limits: memory_mb={} wall_time_ms={} fuel={:?} files={:?}",
                    limits.memory_mb, limits.wall_time_ms, limits.fuel, limits.files
                );
            }
            if let Some(telemetry) = &manifest.telemetry {
                println!("  telemetry span prefix: {}", telemetry.span_prefix);
                println!("  telemetry attributes: {:?}", telemetry.attributes);
                println!("  telemetry emit node spans: {}", telemetry.emit_node_spans);
            }
        }
        if let Some(describe) = &report.describe {
            println!("describe: {}", describe.status);
            if let Some(source) = &describe.source {
                println!("  source: {source}");
            }
            if let Some(name) = &describe.name {
                println!("  name: {name}");
            }
            if let Some(schema_id) = &describe.schema_id {
                println!("  schema id: {schema_id}");
            }
            if let Some(world) = &describe.world {
                println!("  world: {world}");
            }
            if let Some(versions) = &describe.versions {
                println!("  versions: {}", versions.join(", "));
            }
            if let Some(version_count) = describe.version_count {
                println!("  version count: {version_count}");
            }
            if let Some(function_count) = describe.function_count {
                println!("  functions: {function_count}");
            }
            if let Some(operation_count) = describe.operation_count {
                println!("  operations: {operation_count}");
            }
            if let Some(compare) = &describe.compare_embedded {
                println!("  embedded vs describe: {:?}", compare.overall);
            }
            if let Some(reason) = &describe.reason {
                println!("  reason: {reason}");
            }
        }
    }

    Ok(InspectResult { warnings })
}

pub fn emit_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

pub fn build_report(prepared: &PreparedComponent) -> Value {
    let caps = &prepared.manifest.capabilities;
    serde_json::json!({
        "manifest": &prepared.manifest,
        "manifest_path": prepared.manifest_path,
        "wasm_path": prepared.wasm_path,
        "wasm_hash": prepared.wasm_hash,
        "hash_verified": prepared.hash_verified,
        "world": {
            "expected": prepared.manifest.world.as_str(),
            "ok": prepared.world_ok,
        },
        "lifecycle": {
            "init": prepared.lifecycle.init,
            "health": prepared.lifecycle.health,
            "shutdown": prepared.lifecycle.shutdown,
        },
        "describe": prepared.describe,
        "capabilities": prepared.manifest.capabilities,
        "limits": prepared.manifest.limits,
        "telemetry": prepared.manifest.telemetry,
        "redactions": prepared
            .redaction_paths()
            .iter()
            .map(|p| p.as_str().to_string())
            .collect::<Vec<_>>(),
        "defaults_applied": prepared.defaults_applied(),
        "summary": {
            "supports": prepared.manifest.supports,
            "profiles": prepared.manifest.profiles,
            "capabilities": {
                "wasi": {
                    "filesystem": caps.wasi.filesystem.is_some(),
                    "env": caps.wasi.env.is_some(),
                    "random": caps.wasi.random,
                    "clocks": caps.wasi.clocks
                },
                "host": {
                    "secrets": caps.host.secrets.is_some(),
                    "state": caps.host.state.is_some(),
                    "messaging": caps.host.messaging.is_some(),
                    "events": caps.host.events.is_some(),
                    "http": caps.host.http.is_some(),
                    "telemetry": caps.host.telemetry.is_some(),
                    "iac": caps.host.iac.is_some()
                }
            },
        }
    })
}

fn should_inspect_wasm_artifact(args: &InspectArgs) -> bool {
    let Some(target) = args.target.as_ref() else {
        return false;
    };
    let target = strip_file_scheme(Path::new(target));
    target.is_dir()
        || target
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("wasm"))
            .unwrap_or(false)
}

fn discover_manifest_path(wasm_path: &Path, target_path: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if target_path.is_dir() {
        candidates.push(target_path.join("component.manifest.json"));
    }
    if let Some(parent) = wasm_path.parent() {
        candidates.push(parent.join("component.manifest.json"));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("component.manifest.json"));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn inspect_describe(args: &InspectArgs) -> Result<InspectResult, ComponentError> {
    let mut warnings = Vec::new();
    let mut wasm_path = None;
    let bytes = if let Some(path) = args.describe.as_ref() {
        let path = strip_file_scheme(path);
        fs::read(path)
            .map_err(|err| ComponentError::Doctor(format!("failed to read describe file: {err}")))?
    } else {
        let target = args
            .target
            .as_ref()
            .ok_or_else(|| ComponentError::Doctor("inspect target is required".to_string()))?;
        let path = resolve_wasm_path(target).map_err(ComponentError::Doctor)?;
        wasm_path = Some(path.clone());
        call_describe(&path).map_err(ComponentError::Doctor)?
    };

    let payload = strip_self_describe_tag(&bytes);
    if let Err(err) = ensure_canonical_allow_floats(payload) {
        warnings.push(format!("describe payload not canonical: {err}"));
    }
    if let Ok(describe) = canonical::from_cbor::<ComponentDescribe>(payload) {
        let mut report = DescribeReport::from(describe, args.verify)?;
        report.wasm_path = wasm_path;

        if args.json {
            let json = serde_json::to_string_pretty(&report)
                .map_err(|err| ComponentError::Doctor(format!("failed to encode json: {err}")))?;
            println!("{json}");
        } else {
            emit_describe_human(&report);
        }

        let verify_failed = args.verify
            && report
                .operations
                .iter()
                .any(|op| matches!(op.schema_hash_valid, Some(false)));
        if verify_failed {
            return Err(ComponentError::Doctor(
                "schema_hash verification failed".to_string(),
            ));
        }
        return Ok(InspectResult { warnings });
    }

    let derived: DescribePayload = canonical::from_cbor(payload)
        .map_err(|err| ComponentError::Doctor(format!("describe decode failed: {err}")))?;
    if args.verify {
        warnings.push("verify skipped for WIT-derived describe payload".to_string());
    }
    let mut report = DerivedDescribeReport::from(derived);
    report.wasm_path = wasm_path;

    if args.json {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|err| ComponentError::Doctor(format!("failed to encode json: {err}")))?;
        println!("{json}");
    } else {
        emit_derived_describe_human(&report);
    }

    Ok(InspectResult { warnings })
}

fn emit_describe_human(report: &DescribeReport) {
    println!("component: {}", report.info.id);
    println!("  version: {}", report.info.version);
    println!("  role: {}", report.info.role);
    println!("  operations: {}", report.operations.len());
    for op in &report.operations {
        println!("  - {} ({})", op.id, op.schema_hash);
        println!("    input: {}", op.input.summary);
        println!("    output: {}", op.output.summary);
        if let Some(status) = op.schema_hash_valid {
            println!("    schema_hash ok: {status}");
        }
    }
    println!("  config: {}", report.config.summary);
}

#[derive(Debug, Serialize)]
struct DerivedDescribeReport {
    kind: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_id: Option<String>,
    versions: Vec<DerivedDescribeVersionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wasm_path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct DerivedDescribeVersionReport {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    world: Option<String>,
    function_count: usize,
}

impl DerivedDescribeReport {
    fn from(payload: DescribePayload) -> Self {
        Self {
            kind: "wit-derived",
            name: payload.name,
            schema_id: payload.schema_id,
            versions: payload
                .versions
                .into_iter()
                .map(|version| DerivedDescribeVersionReport {
                    version: version.version.to_string(),
                    world: version
                        .schema
                        .get("world")
                        .and_then(|world| world.as_str())
                        .map(str::to_string),
                    function_count: version
                        .schema
                        .get("functions")
                        .and_then(|functions| functions.as_array())
                        .map(|functions| functions.len())
                        .unwrap_or(0),
                })
                .collect(),
            wasm_path: None,
        }
    }
}

fn emit_derived_describe_human(report: &DerivedDescribeReport) {
    println!("describe: wit-derived");
    println!("  name: {}", report.name);
    if let Some(schema_id) = &report.schema_id {
        println!("  schema id: {schema_id}");
    }
    for version in &report.versions {
        println!("  - version: {}", version.version);
        if let Some(world) = &version.world {
            println!("    world: {world}");
        }
        println!("    functions: {}", version.function_count);
    }
}

#[derive(Debug, Serialize)]
struct DescribeReport {
    info: ComponentInfoSummary,
    operations: Vec<OperationSummary>,
    config: SchemaSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    wasm_path: Option<PathBuf>,
}

impl DescribeReport {
    fn from(describe: ComponentDescribe, verify: bool) -> Result<Self, ComponentError> {
        let info = ComponentInfoSummary {
            id: describe.info.id,
            version: describe.info.version,
            role: describe.info.role,
        };
        let config = SchemaSummary::from_schema(&describe.config_schema);
        let mut operations = Vec::new();
        for op in describe.operations {
            let input = SchemaSummary::from_schema(&op.input.schema);
            let output = SchemaSummary::from_schema(&op.output.schema);
            let schema_hash_valid = if verify {
                let expected =
                    schema_hash(&op.input.schema, &op.output.schema, &describe.config_schema)
                        .map_err(|err| {
                            ComponentError::Doctor(format!("schema_hash failed: {err}"))
                        })?;
                Some(expected == op.schema_hash)
            } else {
                None
            };
            operations.push(OperationSummary {
                id: op.id,
                schema_hash: op.schema_hash,
                schema_hash_valid,
                input,
                output,
            });
        }
        Ok(Self {
            info,
            operations,
            config,
            wasm_path: None,
        })
    }
}

#[derive(Debug, Serialize)]
struct ComponentInfoSummary {
    id: String,
    version: String,
    role: String,
}

#[derive(Debug, Serialize)]
struct OperationSummary {
    id: String,
    schema_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_hash_valid: Option<bool>,
    input: SchemaSummary,
    output: SchemaSummary,
}

#[derive(Debug, Serialize)]
struct SchemaSummary {
    kind: String,
    summary: String,
}

impl SchemaSummary {
    fn from_schema(schema: &SchemaIr) -> Self {
        let (kind, summary) = summarize_schema(schema);
        Self { kind, summary }
    }
}

fn summarize_schema(schema: &SchemaIr) -> (String, String) {
    match schema {
        SchemaIr::Object {
            properties,
            required,
            additional,
        } => {
            let add = match additional {
                AdditionalProperties::Allow => "allow",
                AdditionalProperties::Forbid => "forbid",
                AdditionalProperties::Schema(_) => "schema",
            };
            let summary = format!(
                "object{{fields={}, required={}, additional={add}}}",
                properties.len(),
                required.len()
            );
            ("object".to_string(), summary)
        }
        SchemaIr::Array {
            min_items,
            max_items,
            ..
        } => (
            "array".to_string(),
            format!("array{{min={:?}, max={:?}}}", min_items, max_items),
        ),
        SchemaIr::String {
            min_len,
            max_len,
            format,
            ..
        } => (
            "string".to_string(),
            format!(
                "string{{min={:?}, max={:?}, format={:?}}}",
                min_len, max_len, format
            ),
        ),
        SchemaIr::Int { min, max } => (
            "int".to_string(),
            format!("int{{min={:?}, max={:?}}}", min, max),
        ),
        SchemaIr::Float { min, max } => (
            "float".to_string(),
            format!("float{{min={:?}, max={:?}}}", min, max),
        ),
        SchemaIr::Enum { values } => (
            "enum".to_string(),
            format!("enum{{values={}}}", values.len()),
        ),
        SchemaIr::OneOf { variants } => (
            "oneof".to_string(),
            format!("oneof{{variants={}}}", variants.len()),
        ),
        SchemaIr::Bool => ("bool".to_string(), "bool".to_string()),
        SchemaIr::Null => ("null".to_string(), "null".to_string()),
        SchemaIr::Bytes => ("bytes".to_string(), "bytes".to_string()),
        SchemaIr::Ref { id } => ("ref".to_string(), format!("ref{{id={id}}}")),
    }
}

fn resolve_wasm_path(target: &str) -> Result<PathBuf, String> {
    let target_path = strip_file_scheme(Path::new(target));
    if target_path.is_file() {
        return Ok(target_path.to_path_buf());
    }
    if target_path.is_dir()
        && let Some(found) = find_wasm_in_dir(&target_path)?
    {
        return Ok(found);
    }
    Err(format!("inspect: unable to resolve wasm for '{target}'"))
}

fn find_wasm_in_dir(dir: &Path) -> Result<Option<PathBuf>, String> {
    let mut candidates = Vec::new();
    let dist = dir.join("dist");
    if dist.is_dir() {
        collect_wasm_files(&dist, &mut candidates)?;
    }
    let target = dir.join("target").join("wasm32-wasip2");
    for profile in ["release", "debug"] {
        let profile_dir = target.join(profile);
        if profile_dir.is_dir() {
            collect_wasm_files(&profile_dir, &mut candidates)?;
        }
    }
    candidates.sort();
    candidates.dedup();
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(Some(candidates.remove(0))),
        _ => Err(format!(
            "inspect: multiple wasm files found in {}; specify one explicitly",
            dir.display()
        )),
    }
}

fn collect_wasm_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
            out.push(path);
        }
    }
    Ok(())
}

fn call_describe(wasm_path: &Path) -> Result<Vec<u8>, String> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config).map_err(|err| format!("engine init failed: {err}"))?;
    let component = Component::from_file(&engine, wasm_path)
        .map_err(|err| format!("failed to load component: {err}"))?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|err| format!("failed to add wasi: {err}"))?;
    let mut store = Store::new(&engine, InspectWasi::new().map_err(|e| e.to_string())?);
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|err| format!("failed to instantiate: {err}"))?;
    let instance_index = resolve_interface_index(&instance, &mut store, "component-descriptor")
        .ok_or_else(|| "missing export interface component-descriptor".to_string())?;
    let func_index = instance
        .get_export_index(&mut store, Some(&instance_index), "describe")
        .ok_or_else(|| "missing export component-descriptor.describe".to_string())?;
    let func = instance
        .get_func(&mut store, func_index)
        .ok_or_else(|| "describe export is not callable".to_string())?;
    let mut results = vec![Val::Bool(false); func.ty(&mut store).results().len()];
    func.call(&mut store, &[], &mut results)
        .map_err(|err| format!("describe call failed: {err}"))?;
    let val = results
        .first()
        .ok_or_else(|| "describe returned no value".to_string())?;
    val_to_bytes(val)
}

fn resolve_interface_index(
    instance: &wasmtime::component::Instance,
    store: &mut Store<InspectWasi>,
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

fn val_to_bytes(val: &Val) -> Result<Vec<u8>, String> {
    match val {
        Val::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Val::U8(byte) => out.push(*byte),
                    _ => return Err("expected list<u8>".to_string()),
                }
            }
            Ok(out)
        }
        _ => Err("expected list<u8>".to_string()),
    }
}

fn strip_self_describe_tag(bytes: &[u8]) -> &[u8] {
    const SELF_DESCRIBE_TAG: [u8; 3] = [0xd9, 0xd9, 0xf7];
    if bytes.starts_with(&SELF_DESCRIBE_TAG) {
        &bytes[SELF_DESCRIBE_TAG.len()..]
    } else {
        bytes
    }
}

fn ensure_canonical_allow_floats(bytes: &[u8]) -> Result<(), String> {
    let canonicalized = canonical::canonicalize_allow_floats(bytes)
        .map_err(|err| format!("canonicalization failed: {err}"))?;
    if canonicalized.as_slice() != bytes {
        return Err("payload is not canonical".to_string());
    }
    Ok(())
}

struct InspectWasi {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl InspectWasi {
    fn new() -> Result<Self, anyhow::Error> {
        let ctx = WasiCtxBuilder::new().build();
        Ok(Self {
            ctx,
            table: ResourceTable::new(),
        })
    }
}

impl WasiView for InspectWasi {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::Val;

    #[test]
    fn should_inspect_wasm_artifact_detects_wasm_and_dirs_only() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(should_inspect_wasm_artifact(&InspectArgs {
            target: Some("component.wasm".into()),
            manifest: None,
            describe: None,
            json: false,
            verify: false,
            strict: false,
        }));
        assert!(should_inspect_wasm_artifact(&InspectArgs {
            target: Some(dir.path().display().to_string()),
            manifest: None,
            describe: None,
            json: false,
            verify: false,
            strict: false,
        }));
        assert!(!should_inspect_wasm_artifact(&InspectArgs {
            target: Some("component.manifest.json".into()),
            manifest: None,
            describe: None,
            json: false,
            verify: false,
            strict: false,
        }));
    }

    #[test]
    fn discover_manifest_path_checks_target_and_parent_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = dir.path().join("component.manifest.json");
        fs::write(&manifest, "{}").expect("write manifest");

        let from_dir = discover_manifest_path(&dir.path().join("component.wasm"), dir.path());
        assert_eq!(from_dir, Some(manifest.clone()));

        let nested = dir.path().join("dist/component.wasm");
        fs::create_dir_all(nested.parent().expect("parent")).expect("create dist");
        let from_parent = discover_manifest_path(&nested, nested.parent().expect("parent"));
        assert_eq!(from_parent, Some(manifest));
    }

    #[test]
    fn resolve_wasm_path_reports_multiple_candidates_in_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dist = dir.path().join("dist");
        fs::create_dir_all(&dist).expect("create dist");
        fs::write(dist.join("one.wasm"), b"1").expect("write first wasm");
        fs::write(dist.join("two.wasm"), b"2").expect("write second wasm");

        let err = resolve_wasm_path(dir.path().to_str().expect("utf-8"))
            .expect_err("multiple wasm files should fail");

        assert!(err.contains("multiple wasm files found"));
    }

    #[test]
    fn val_to_bytes_rejects_non_byte_lists() {
        let err = val_to_bytes(&Val::List(vec![Val::String("oops".to_string())]))
            .expect_err("non-u8 list should fail");
        assert_eq!(err, "expected list<u8>");
    }

    #[test]
    fn strip_self_describe_tag_removes_only_known_prefix() {
        let tagged = [0xd9, 0xd9, 0xf7, 0x01, 0x02];
        assert_eq!(strip_self_describe_tag(&tagged), &[0x01, 0x02]);
        assert_eq!(strip_self_describe_tag(&[0x01, 0x02]), &[0x01, 0x02]);
    }
}
