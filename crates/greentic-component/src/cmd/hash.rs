use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser};
use serde_json::Value;

use crate::path_safety::normalize_under_root;

#[derive(Args, Debug, Clone)]
#[command(about = "Recompute the wasm hash inside component.manifest.json")]
pub struct HashArgs {
    /// Path to component.manifest.json
    #[arg(default_value = "component.manifest.json")]
    pub manifest: PathBuf,
    /// Optional override for the wasm artifact path
    #[arg(long)]
    pub wasm: Option<PathBuf>,
}

#[derive(Parser, Debug)]
struct HashCli {
    #[command(flatten)]
    args: HashArgs,
}

pub fn parse_from_cli() -> HashArgs {
    HashCli::parse().args
}

pub fn run(args: HashArgs) -> Result<()> {
    let workspace_root = std::env::current_dir()
        .context("failed to read current directory")?
        .canonicalize()
        .context("failed to canonicalize workspace root")?;
    let manifest_path =
        normalize_or_canonicalize(&workspace_root, &args.manifest).with_context(|| {
            format!(
                "manifest path escapes workspace root: {}",
                args.manifest.display()
            )
        })?;
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let mut manifest: Value = serde_json::from_str(&manifest_text)
        .with_context(|| format!("invalid json: {}", manifest_path.display()))?;
    let manifest_root = manifest_path
        .parent()
        .unwrap_or(workspace_root.as_path())
        .canonicalize()
        .with_context(|| {
            format!(
                "failed to canonicalize manifest directory {}",
                manifest_path.display()
            )
        })?;
    let wasm_candidate = resolve_wasm_path(&manifest, args.wasm.as_deref())?;
    let wasm_path =
        normalize_or_canonicalize(&manifest_root, &wasm_candidate).with_context(|| {
            format!(
                "wasm path escapes manifest root {}",
                manifest_root.display()
            )
        })?;
    let wasm_bytes = fs::read(&wasm_path)
        .with_context(|| format!("failed to read wasm at {}", wasm_path.display()))?;
    let digest = blake3::hash(&wasm_bytes).to_hex().to_string();
    manifest["hashes"]["component_wasm"] = Value::String(format!("blake3:{digest}"));
    let formatted = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, formatted + "\n")
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    println!(
        "Updated {} with hash of {}",
        manifest_path.display(),
        wasm_path.display()
    );
    Ok(())
}

fn resolve_wasm_path(manifest: &Value, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    let artifact = manifest
        .get("artifacts")
        .and_then(|art| art.get("component_wasm"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("manifest is missing artifacts.component_wasm"))?;
    Ok(PathBuf::from(artifact))
}

fn normalize_or_canonicalize(root: &Path, candidate: &Path) -> Result<PathBuf> {
    if candidate.is_absolute() {
        return candidate
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", candidate.display()));
    }
    normalize_under_root(root, candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_wasm_path_prefers_explicit_override() {
        let manifest = json!({
            "artifacts": { "component_wasm": "component.wasm" }
        });
        let path = resolve_wasm_path(&manifest, Some(Path::new("override.wasm"))).unwrap();
        assert_eq!(path, PathBuf::from("override.wasm"));
    }

    #[test]
    fn resolve_wasm_path_errors_when_manifest_is_missing_artifact() {
        let err = resolve_wasm_path(&json!({}), None).expect_err("artifact path required");
        assert!(err.to_string().contains("artifacts.component_wasm"));
    }

    #[test]
    fn normalize_or_canonicalize_allows_relative_paths_under_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let wasm = root.path().join("component.wasm");
        fs::write(&wasm, b"wasm").expect("write wasm");

        let normalized =
            normalize_or_canonicalize(root.path(), Path::new("component.wasm")).unwrap();
        assert_eq!(normalized, wasm);
    }

    #[test]
    fn resolve_wasm_path_uses_manifest_artifact_when_present() {
        let manifest = json!({
            "artifacts": { "component_wasm": "dist/component.wasm" }
        });

        let path = resolve_wasm_path(&manifest, None).expect("artifact path");

        assert_eq!(path, PathBuf::from("dist/component.wasm"));
    }

    #[test]
    fn normalize_or_canonicalize_canonicalizes_absolute_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        let wasm = root.path().join("component.wasm");
        fs::write(&wasm, b"wasm").expect("write wasm");

        let normalized = normalize_or_canonicalize(root.path(), &wasm).expect("absolute path");

        assert_eq!(normalized, wasm.canonicalize().expect("canonical wasm"));
    }

    #[test]
    fn run_updates_manifest_hash_using_manifest_artifact() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("cwd");
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(dir.path()).expect("set cwd");

        let wasm = dir.path().join("component.wasm");
        fs::write(&wasm, b"updated-wasm").expect("write wasm");
        let manifest_path = dir.path().join("component.manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json!({
                "artifacts": { "component_wasm": "component.wasm" },
                "hashes": { "component_wasm": "blake3:old" }
            }))
            .expect("manifest json"),
        )
        .expect("write manifest");

        run(HashArgs {
            manifest: PathBuf::from("component.manifest.json"),
            wasm: None,
        })
        .expect("run hash command");

        let updated: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("read manifest"))
                .expect("parse updated manifest");
        let expected = format!("blake3:{}", blake3::hash(b"updated-wasm").to_hex());
        assert_eq!(updated["hashes"]["component_wasm"], Value::String(expected));

        std::env::set_current_dir(original_cwd).expect("restore cwd");
    }

    #[test]
    fn run_prefers_explicit_wasm_override() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("cwd");
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(dir.path()).expect("set cwd");

        let manifest_wasm = dir.path().join("component.wasm");
        let override_wasm = dir.path().join("override.wasm");
        fs::write(&manifest_wasm, b"manifest-wasm").expect("write manifest wasm");
        fs::write(&override_wasm, b"override-wasm").expect("write override wasm");
        let manifest_path = dir.path().join("component.manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json!({
                "artifacts": { "component_wasm": "component.wasm" },
                "hashes": { "component_wasm": "blake3:old" }
            }))
            .expect("manifest json"),
        )
        .expect("write manifest");

        run(HashArgs {
            manifest: PathBuf::from("component.manifest.json"),
            wasm: Some(PathBuf::from("override.wasm")),
        })
        .expect("run hash command");

        let updated: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("read manifest"))
                .expect("parse updated manifest");
        let expected = format!("blake3:{}", blake3::hash(b"override-wasm").to_hex());
        assert_eq!(updated["hashes"]["component_wasm"], Value::String(expected));

        std::env::set_current_dir(original_cwd).expect("restore cwd");
    }
}
