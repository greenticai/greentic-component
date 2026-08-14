#![cfg(feature = "cli")]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde_json::Value as JsonValue;
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::abi::{self, AbiError};
use crate::cmd::component_world::{canonical_component_world, is_fallback_world};
use crate::cmd::flow::{
    FlowUpdateResult, manifest_component_id, resolve_operation, update_with_manifest,
};
use crate::cmd::i18n;
use crate::config::{
    ConfigInferenceOptions, ConfigSchemaSource, load_manifest_with_schema, resolve_manifest_path,
};
use crate::describe::{DescribePayload, from_wit_world};
use crate::embedded_descriptor::embed_and_verify_wasm;
use crate::parse_manifest;
use crate::path_safety::normalize_under_root;
use crate::schema_quality::{SchemaQualityMode, validate_operation_schemas};
use greentic_types::cbor::canonical;
use greentic_types::schemas::component::v0_6_0::ComponentDescribe;

const DEFAULT_MANIFEST: &str = "component.manifest.json";

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    /// Path to component.manifest.json (or directory containing it)
    #[arg(long = "manifest", value_name = "PATH", default_value = DEFAULT_MANIFEST)]
    pub manifest: PathBuf,
    /// Path to the cargo binary (fallback: $CARGO, then `cargo` on PATH)
    #[arg(long = "cargo", value_name = "PATH")]
    pub cargo_bin: Option<PathBuf>,
    /// Skip flow regeneration
    #[arg(long = "no-flow")]
    pub no_flow: bool,
    /// Skip config inference; fail if config_schema is missing
    #[arg(long = "no-infer-config")]
    pub no_infer_config: bool,
    /// Do not write inferred config_schema back to the manifest
    #[arg(long = "no-write-schema")]
    pub no_write_schema: bool,
    /// Overwrite existing config_schema with inferred schema
    #[arg(long = "force-write-schema")]
    pub force_write_schema: bool,
    /// Skip schema validation
    #[arg(long = "no-validate")]
    pub no_validate: bool,
    /// Emit machine-readable JSON summary
    #[arg(long = "json")]
    pub json: bool,
    /// Allow empty operation schemas (warnings only)
    #[arg(long)]
    pub permissive: bool,
}

#[derive(Debug, serde::Serialize)]
struct BuildSummary {
    manifest: PathBuf,
    wasm_path: PathBuf,
    wasm_hash: String,
    config_source: ConfigSchemaSource,
    schema_written: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    flows: Option<FlowUpdateResult>,
}

pub fn run(args: BuildArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest);
    let cwd = env::current_dir().context("failed to read current directory")?;
    let manifest_path = if manifest_path.is_absolute() {
        manifest_path
    } else {
        cwd.join(manifest_path)
    };
    if !manifest_path.exists() {
        bail!(
            "{}",
            i18n::tr_lit("manifest not found at {}").replacen(
                "{}",
                &manifest_path.display().to_string(),
                1
            )
        );
    }
    let cargo_bin = args
        .cargo_bin
        .clone()
        .or_else(|| env::var_os("CARGO").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let inference_opts = ConfigInferenceOptions {
        allow_infer: !args.no_infer_config,
        write_schema: !args.no_write_schema,
        force_write_schema: args.force_write_schema,
        validate: !args.no_validate,
    };
    println!(
        "Using manifest at {} (cargo: {})",
        manifest_path.display(),
        cargo_bin.display()
    );

    let config = load_manifest_with_schema(&manifest_path, &inference_opts)?;
    let mode = if args.permissive {
        SchemaQualityMode::Permissive
    } else {
        SchemaQualityMode::Strict
    };
    let manifest_component = parse_manifest(
        &serde_json::to_string(&config.manifest)
            .context("failed to serialize manifest for schema validation")?,
    )
    .context("failed to parse manifest for schema validation")?;
    let schema_warnings = validate_operation_schemas(&manifest_component, mode)?;
    for warning in schema_warnings {
        eprintln!("warning[W_OP_SCHEMA_EMPTY]: {}", warning.message);
    }
    let component_id = manifest_component_id(&config.manifest)?;
    let _operation = resolve_operation(&config.manifest, component_id)?;
    let flow_outcome = if args.no_flow {
        None
    } else {
        Some(update_with_manifest(&config)?)
    };

    let mut manifest_to_write = flow_outcome
        .as_ref()
        .map(|outcome| outcome.manifest.clone())
        .unwrap_or_else(|| config.manifest.clone());
    let canonical_manifest = parse_manifest(
        &serde_json::to_string(&manifest_to_write)
            .context("failed to serialize manifest for embedded descriptor")?,
    )
    .context("failed to parse canonical manifest for embedded descriptor")?;

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    build_wasm(manifest_dir, &cargo_bin, &manifest_to_write)?;
    check_canonical_world_export(manifest_dir, &manifest_to_write)?;
    let wasm_path_for_embedding = resolve_wasm_path(manifest_dir, &manifest_to_write)?;
    embed_and_verify_wasm(&wasm_path_for_embedding, &canonical_manifest)
        .context("failed to embed canonical manifest into built wasm")?;

    if !config.persist_schema {
        manifest_to_write
            .as_object_mut()
            .map(|obj| obj.remove("config_schema"));
    }
    let (wasm_path, wasm_hash) = update_manifest_hashes(manifest_dir, &mut manifest_to_write)?;
    emit_describe_artifacts(manifest_dir, &manifest_to_write, &wasm_path)?;
    write_manifest(&manifest_path, &manifest_to_write)?;

    if args.json {
        let payload = BuildSummary {
            manifest: manifest_path.clone(),
            wasm_path,
            wasm_hash,
            config_source: config.source,
            schema_written: config.schema_written && config.persist_schema,
            flows: flow_outcome.as_ref().map(|outcome| outcome.result),
        };
        serde_json::to_writer_pretty(std::io::stdout(), &payload)?;
        println!();
    } else {
        println!("Built wasm artifact at {}", wasm_path.display());
        println!("Updated {} hashes (blake3)", manifest_path.display());
        if config.schema_written && config.persist_schema {
            println!(
                "Updated {} with inferred config_schema ({:?})",
                manifest_path.display(),
                config.source
            );
        }
        if let Some(outcome) = flow_outcome {
            let flows = outcome.result;
            println!(
                "Flows updated (default: {}, custom: {})",
                flows.default_updated, flows.custom_updated
            );
        } else {
            println!("Flow regeneration skipped (--no-flow)");
        }
    }

    Ok(())
}

fn build_wasm(manifest_dir: &Path, cargo_bin: &Path, manifest: &JsonValue) -> Result<()> {
    let resolved_world = manifest.get("world").and_then(|v| v.as_str()).unwrap_or("");
    if resolved_world.is_empty() {
        println!("Resolved manifest world: <missing>");
    } else {
        println!("Resolved manifest world: {resolved_world}");
    }
    let require_component = resolved_world.contains("component@0.6.0");

    if require_component {
        if cargo_component_available(cargo_bin) {
            println!(
                "Running cargo component build via {} in {}",
                cargo_bin.display(),
                manifest_dir.display()
            );
            // The executable is the resolved Cargo binary; arguments are passed directly without a shell.
            // foxguard: ignore[rs/no-command-injection]
            let mut cmd = Command::new(cargo_bin);
            if let Some(flags) = resolved_wasm_rustflags() {
                cmd.env("RUSTFLAGS", sanitize_wasm_rustflags(&flags));
            }
            cmd.arg("component").arg("build");
            maybe_add_offline_flag(&mut cmd);
            let status = cmd
                .arg("--target")
                .arg("wasm32-wasip2")
                .arg("--release")
                .current_dir(manifest_dir)
                .status()
                .with_context(|| {
                    format!(
                        "failed to run cargo component build via {}",
                        cargo_bin.display()
                    )
                })?;
            if !status.success() {
                bail!(
                    "cargo component build --target wasm32-wasip2 --release failed with status {}",
                    status
                );
            }
            return Ok(());
        }
        bail!(
            "component@0.6.0 manifests require cargo-component; install it with `cargo install cargo-component --locked`"
        );
    }

    println!(
        "Running cargo build via {} in {}",
        cargo_bin.display(),
        manifest_dir.display()
    );
    // The executable is the resolved Cargo binary; arguments are passed directly without a shell.
    // foxguard: ignore[rs/no-command-injection]
    let mut cmd = Command::new(cargo_bin);
    if let Some(flags) = resolved_wasm_rustflags() {
        cmd.env("RUSTFLAGS", sanitize_wasm_rustflags(&flags));
    }
    cmd.arg("build");
    maybe_add_offline_flag(&mut cmd);
    let status = cmd
        .arg("--target")
        .arg("wasm32-wasip2")
        .arg("--release")
        .current_dir(manifest_dir)
        .status()
        .with_context(|| format!("failed to run cargo build via {}", cargo_bin.display()))?;

    if !status.success() {
        bail!(
            "cargo build --target wasm32-wasip2 --release failed with status {}",
            status
        );
    }
    Ok(())
}

fn cargo_component_available(cargo_bin: &Path) -> bool {
    // The executable is the resolved Cargo binary; arguments are passed directly without a shell.
    // foxguard: ignore[rs/no-command-injection]
    Command::new(cargo_bin)
        .arg("component")
        .arg("--version")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn maybe_add_offline_flag(cmd: &mut Command) {
    if cargo_offline_requested() {
        cmd.arg("--offline");
    }
}

fn cargo_offline_requested() -> bool {
    env_truthy(env::var_os("CARGO_NET_OFFLINE").as_deref())
}

fn env_truthy(value: Option<&std::ffi::OsStr>) -> bool {
    value
        .and_then(|raw| raw.to_str())
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Reads the wasm-specific rustflags that CI exports for wasm builds.
fn resolved_wasm_rustflags() -> Option<String> {
    env::var("WASM_RUSTFLAGS")
        .ok()
        .or_else(|| env::var("RUSTFLAGS").ok())
}

/// Drops linker arguments that `wasm-component-ld` rejects and normalizes whitespace.
fn sanitize_wasm_rustflags(flags: &str) -> String {
    flags
        .replace("-Wl,", "")
        .replace("-C link-arg=--no-keep-memory", "")
        .replace("-C link-arg=--threads=1", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_canonical_world_export(manifest_dir: &Path, manifest: &JsonValue) -> Result<()> {
    if env::var_os("GREENTIC_SKIP_NODE_EXPORT_CHECK").is_some() {
        println!("World export check skipped (GREENTIC_SKIP_NODE_EXPORT_CHECK=1)");
        return Ok(());
    }
    let wasm_path = resolve_wasm_path(manifest_dir, manifest)?;
    let canonical_world = canonical_component_world();
    match abi::check_world_base(&wasm_path, canonical_world) {
        Ok(exported) => println!("Exported world: {exported}"),
        Err(err) => match err {
            AbiError::WorldMismatch { expected, found } if is_fallback_world(&found) => {
                println!("Exported world: {expected} (compatible fallback export: {found})");
            }
            err => {
                return Err(err)
                    .with_context(|| format!("component must export world {canonical_world}"));
            }
        },
    }
    Ok(())
}

fn update_manifest_hashes(
    manifest_dir: &Path,
    manifest: &mut JsonValue,
) -> Result<(PathBuf, String)> {
    let artifact_path = resolve_wasm_path(manifest_dir, manifest)?;
    let wasm_bytes = fs::read(&artifact_path)
        .with_context(|| format!("failed to read wasm at {}", artifact_path.display()))?;
    let digest = blake3::hash(&wasm_bytes).to_hex().to_string();

    manifest["artifacts"]["component_wasm"] =
        JsonValue::String(path_string_relative(manifest_dir, &artifact_path)?);
    manifest["hashes"]["component_wasm"] = JsonValue::String(format!("blake3:{digest}"));

    Ok((artifact_path, format!("blake3:{digest}")))
}

fn path_string_relative(base: &Path, target: &Path) -> Result<String> {
    let rel = pathdiff::diff_paths(target, base).unwrap_or_else(|| target.to_path_buf());
    rel.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("failed to stringify path {}", target.display()))
}

fn resolve_wasm_path(manifest_dir: &Path, manifest: &JsonValue) -> Result<PathBuf> {
    let manifest_root = manifest_dir
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", manifest_dir.display()))?;
    let candidate = manifest
        .get("artifacts")
        .and_then(|a| a.get("component_wasm"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let raw_name = manifest
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| manifest.get("id").and_then(|v| v.as_str()))
                .unwrap_or("component");
            let sanitized = raw_name.replace(['-', '.'], "_");
            manifest_dir.join(format!("target/wasm32-wasip2/release/{sanitized}.wasm"))
        });
    if candidate.exists() {
        let normalized = normalize_under_root(&manifest_root, &candidate).or_else(|_| {
            if candidate.is_absolute() {
                candidate
                    .canonicalize()
                    .with_context(|| format!("failed to canonicalize {}", candidate.display()))
            } else {
                normalize_under_root(&manifest_root, &candidate)
            }
        })?;
        return Ok(normalized);
    }

    if let Some(cargo_target_dir) = env::var_os("CARGO_TARGET_DIR") {
        let relative = candidate
            .strip_prefix(manifest_dir)
            .unwrap_or(&candidate)
            .to_path_buf();
        if relative.starts_with("target") {
            let alt =
                PathBuf::from(cargo_target_dir).join(relative.strip_prefix("target").unwrap());
            if alt.exists() {
                return alt
                    .canonicalize()
                    .with_context(|| format!("failed to canonicalize {}", alt.display()));
            }
        }
    }

    let normalized = normalize_under_root(&manifest_root, &candidate).or_else(|_| {
        if candidate.is_absolute() {
            candidate
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", candidate.display()))
        } else {
            normalize_under_root(&manifest_root, &candidate)
        }
    })?;
    Ok(normalized)
}

fn write_manifest(manifest_path: &Path, manifest: &JsonValue) -> Result<()> {
    let formatted = serde_json::to_string_pretty(manifest)?;
    fs::write(manifest_path, formatted + "\n")
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn emit_describe_artifacts(
    manifest_dir: &Path,
    manifest: &JsonValue,
    wasm_path: &Path,
) -> Result<()> {
    let abi_version = read_abi_version(manifest_dir);
    let require_describe = abi_version.as_deref() == Some("0.6.0");
    let manifest_model = parse_manifest(
        &serde_json::to_string(manifest).context("failed to serialize manifest for describe")?,
    )
    .context("failed to parse manifest for describe")?;

    let describe_bytes = match call_describe(wasm_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            if require_describe {
                match from_wit_world(wasm_path, manifest_model.world.as_str()) {
                    Ok(payload) => {
                        write_wit_describe_artifacts(
                            manifest_dir,
                            manifest,
                            wasm_path,
                            abi_version.as_deref(),
                            &payload,
                        )?;
                        eprintln!(
                            "warning: describe export unavailable, emitted WIT-derived describe.json instead ({err})"
                        );
                        return Ok(());
                    }
                    Err(wit_err) => {
                        return Err(anyhow!(
                            "describe failed: {err}; WIT fallback failed: {wit_err}"
                        ));
                    }
                }
            }
            eprintln!("warning: skipping describe artifacts ({err})");
            return Ok(());
        }
    };

    let payload = strip_self_describe_tag(&describe_bytes);
    let canonical_bytes = canonical::canonicalize_allow_floats(payload)
        .map_err(|err| anyhow!("describe canonicalization failed: {err}"))?;
    let describe: ComponentDescribe = canonical::from_cbor(&canonical_bytes)
        .map_err(|err| anyhow!("describe decode failed: {err}"))?;

    let dist_dir = manifest_dir.join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;

    let (name, abi_underscore) = artifact_basename(manifest, wasm_path, abi_version.as_deref());
    let base = format!("{name}__{abi_underscore}");
    let describe_cbor_path = dist_dir.join(format!("{base}.describe.cbor"));
    fs::write(&describe_cbor_path, &canonical_bytes)
        .with_context(|| format!("failed to write {}", describe_cbor_path.display()))?;

    let describe_json_path = dist_dir.join(format!("{base}.describe.json"));
    let json = serde_json::to_string_pretty(&describe)?;
    fs::write(&describe_json_path, json + "\n")
        .with_context(|| format!("failed to write {}", describe_json_path.display()))?;

    let wasm_out = dist_dir.join(format!("{base}.wasm"));
    if wasm_out != wasm_path {
        let _ = fs::copy(wasm_path, &wasm_out);
    }

    Ok(())
}

fn write_wit_describe_artifacts(
    manifest_dir: &Path,
    manifest: &JsonValue,
    wasm_path: &Path,
    abi_version: Option<&str>,
    payload: &DescribePayload,
) -> Result<()> {
    let dist_dir = manifest_dir.join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;

    let (name, abi_underscore) = artifact_basename(manifest, wasm_path, abi_version);
    let base = format!("{name}__{abi_underscore}");
    let describe_cbor_path = dist_dir.join(format!("{base}.describe.cbor"));
    let cbor = canonical::to_canonical_cbor_allow_floats(payload)
        .map_err(|err| anyhow!("describe fallback canonicalization failed: {err}"))?;
    fs::write(&describe_cbor_path, cbor)
        .with_context(|| format!("failed to write {}", describe_cbor_path.display()))?;

    let describe_json_path = dist_dir.join(format!("{base}.describe.json"));
    let json = serde_json::to_string_pretty(payload)?;
    fs::write(&describe_json_path, json + "\n")
        .with_context(|| format!("failed to write {}", describe_json_path.display()))?;

    let wasm_out = dist_dir.join(format!("{base}.wasm"));
    if wasm_out != wasm_path {
        let _ = fs::copy(wasm_path, &wasm_out);
    }

    Ok(())
}

fn read_abi_version(manifest_dir: &Path) -> Option<String> {
    let cargo_path = manifest_dir.join("Cargo.toml");
    let contents = fs::read_to_string(cargo_path).ok()?;
    let doc: toml::Value = toml::from_str(&contents).ok()?;
    doc.get("package")
        .and_then(|pkg| pkg.get("metadata"))
        .and_then(|meta| meta.get("greentic"))
        .and_then(|g| g.get("abi_version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn artifact_basename(
    manifest: &JsonValue,
    wasm_path: &Path,
    abi_version: Option<&str>,
) -> (String, String) {
    let name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| manifest.get("id").and_then(|v| v.as_str()))
        .map(sanitize_name)
        .unwrap_or_else(|| {
            wasm_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(sanitize_name)
                .unwrap_or_else(|| "component".to_string())
        });
    let abi = abi_version.unwrap_or("0.6.0").replace('.', "_");
    (name, abi)
}

fn sanitize_name(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn call_describe(wasm_path: &Path) -> Result<Vec<u8>> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config).map_err(|err| anyhow!("failed to create engine: {err}"))?;
    let component = Component::from_file(&engine, wasm_path)
        .map_err(|err| anyhow!("failed to load component {}: {err}", wasm_path.display()))?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|err| anyhow!("failed to add wasi: {err}"))?;
    let mut store = Store::new(&engine, BuildWasi::new()?);
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|err| anyhow!("failed to instantiate component: {err}"))?;
    let instance_index = resolve_interface_index(&instance, &mut store, "component-descriptor")
        .ok_or_else(|| anyhow!("missing export interface component-descriptor"))?;
    let func_index = instance
        .get_export_index(&mut store, Some(&instance_index), "describe")
        .ok_or_else(|| anyhow!("missing export component-descriptor.describe"))?;
    let func = instance
        .get_func(&mut store, func_index)
        .ok_or_else(|| anyhow!("describe export is not callable"))?;
    let mut results = vec![Val::Bool(false); func.ty(&mut store).results().len()];
    func.call(&mut store, &[], &mut results)
        .map_err(|err| anyhow!("describe call failed: {err}"))?;
    let val = results
        .first()
        .ok_or_else(|| anyhow!("describe returned no value"))?;
    val_to_bytes(val).map_err(|err| anyhow!(err))
}

fn resolve_interface_index(
    instance: &wasmtime::component::Instance,
    store: &mut Store<BuildWasi>,
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

struct BuildWasi {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl BuildWasi {
    fn new() -> Result<Self> {
        let ctx = WasiCtxBuilder::new().build();
        Ok(Self {
            ctx,
            table: ResourceTable::new(),
        })
    }
}

impl WasiView for BuildWasi {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use serde_json::json;
    use wasmtime::component::Val;

    use super::{
        env_truthy, path_string_relative, resolve_wasm_path, sanitize_name,
        sanitize_wasm_rustflags, strip_self_describe_tag, val_to_bytes,
    };

    #[test]
    fn sanitize_name_preserves_hyphens_for_dist_artifacts() {
        assert_eq!(
            sanitize_name("wizard-smoke-advanced"),
            "wizard-smoke-advanced"
        );
        assert_eq!(
            sanitize_name("wizard_smoke_advanced"),
            "wizard_smoke_advanced"
        );
    }

    #[test]
    fn env_truthy_accepts_common_true_spellings() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(
                env_truthy(Some(OsStr::new(value))),
                "{value} should be truthy"
            );
        }
    }

    #[test]
    fn env_truthy_rejects_falsey_and_missing_values() {
        for value in [
            None,
            Some(OsStr::new("0")),
            Some(OsStr::new("false")),
            Some(OsStr::new("")),
        ] {
            assert!(!env_truthy(value));
        }
    }

    #[test]
    fn sanitize_wasm_rustflags_drops_unsupported_linker_args() {
        let sanitized = sanitize_wasm_rustflags(
            "-C opt-level=z -Wl,--export-table -C link-arg=--no-keep-memory -C link-arg=--threads=1",
        );

        assert_eq!(sanitized, "-C opt-level=z --export-table");
    }

    #[test]
    fn path_string_relative_prefers_relative_path() {
        let base = Path::new("/tmp/project");
        let target = Path::new("/tmp/project/dist/component.wasm");

        let relative = path_string_relative(base, target).expect("relative path");

        assert_eq!(relative, "dist/component.wasm");
    }

    #[test]
    fn resolve_wasm_path_uses_default_target_location_when_manifest_omits_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir
            .path()
            .join("target/wasm32-wasip2/release/com_greentic_demo.wasm");
        std::fs::create_dir_all(target.parent().expect("target parent"))
            .expect("create target dir");
        std::fs::write(&target, b"wasm").expect("write wasm");

        let manifest = json!({
            "id": "com.greentic.demo"
        });

        let resolved = resolve_wasm_path(dir.path(), &manifest).expect("resolve default wasm path");
        assert_eq!(resolved, target.canonicalize().expect("canonical target"));
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
