use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use thiserror::Error;

use crate::manifest::{ComponentManifest, parse_manifest};
use crate::signing::{SigningError, verify_manifest_hash};

const MANIFEST_NAME: &str = "component.manifest.json";

#[derive(Debug, Clone)]
pub struct ComponentHandle {
    pub manifest: ComponentManifest,
    pub wasm_path: PathBuf,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error(
        "component not found for `{0}`; if pointing at a wasm file, pass --manifest <path/to/component.manifest.json>"
    )]
    NotFound(String),
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest parse failed at {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: crate::manifest::ManifestError,
    },
    #[error("missing artifact `{path}` declared in manifest")]
    MissingArtifact { path: PathBuf },
    #[error("hash verification failed: {0}")]
    Signing(#[from] SigningError),
}

pub fn discover(path_or_id: &str) -> Result<ComponentHandle, LoadError> {
    discover_with_manifest(path_or_id, None)
}

pub fn discover_with_manifest(
    path_or_id: &str,
    manifest_override: Option<&Path>,
) -> Result<ComponentHandle, LoadError> {
    if let Some(manifest_path) = manifest_override {
        return load_from_manifest(manifest_path);
    }
    let normalized = normalize_path_or_id(path_or_id);
    let normalized_str = normalized.as_ref();
    if let Some(handle) = try_explicit(normalized_str)? {
        return Ok(handle);
    }
    if let Some(handle) = try_workspace(normalized_str)? {
        return Ok(handle);
    }
    if let Some(handle) = try_registry(path_or_id)? {
        return Ok(handle);
    }
    Err(LoadError::NotFound(path_or_id.to_string()))
}

fn try_explicit(arg: &str) -> Result<Option<ComponentHandle>, LoadError> {
    let path = Path::new(arg);
    if !path.exists() {
        return Ok(None);
    }

    let target = if path.is_dir() {
        path.join(MANIFEST_NAME)
    } else if path.extension().and_then(OsStr::to_str) == Some("json") {
        path.to_path_buf()
    } else if path.extension().and_then(OsStr::to_str) == Some("wasm") {
        path.parent()
            .map(|dir| dir.join(MANIFEST_NAME))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.join(MANIFEST_NAME)
    };

    if target.exists() {
        return load_from_manifest(&target).map(Some);
    }

    Ok(None)
}

fn try_workspace(id: &str) -> Result<Option<ComponentHandle>, LoadError> {
    let cwd = std::env::current_dir().map_err(|e| LoadError::Io {
        path: PathBuf::from("."),
        source: e,
    })?;
    let target = cwd.join("target").join("wasm32-wasip2");
    let file_name = format!("{id}.wasm");

    for profile in ["release", "debug"] {
        let candidate = target.join(profile).join(&file_name);
        if candidate.exists() {
            let manifest_path = candidate
                .parent()
                .map(|dir| dir.join(MANIFEST_NAME))
                .unwrap_or_else(|| candidate.with_extension("manifest.json"));
            if manifest_path.exists() {
                return load_from_manifest(&manifest_path).map(Some);
            }
        }
    }

    Ok(None)
}

fn try_registry(id: &str) -> Result<Option<ComponentHandle>, LoadError> {
    let Some(base) = BaseDirs::new() else {
        return Ok(None);
    };
    let registry_root = base.home_dir().join(".greentic").join("components");
    if !registry_root.exists() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(&registry_root).map_err(|err| LoadError::Io {
        path: registry_root.clone(),
        source: err,
    })? {
        let entry = entry.map_err(|err| LoadError::Io {
            path: registry_root.clone(),
            source: err,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == id || (!id.contains('@') && name.starts_with(id)) {
            candidates.push(entry.path());
        }
    }

    candidates.sort();
    candidates.reverse();

    for dir in candidates {
        let manifest_path = dir.join(MANIFEST_NAME);
        if manifest_path.exists() {
            return load_from_manifest(&manifest_path).map(Some);
        }
    }

    Ok(None)
}

fn load_from_manifest(path: &Path) -> Result<ComponentHandle, LoadError> {
    let contents = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = parse_manifest(&contents).map_err(|source| LoadError::Manifest {
        path: path.to_path_buf(),
        source,
    })?;
    let root = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let wasm_path = root.join(manifest.artifacts.component_wasm());
    if !wasm_path.exists() {
        return Err(LoadError::MissingArtifact { path: wasm_path });
    }
    verify_manifest_hash(&manifest, &root)?;
    Ok(ComponentHandle {
        manifest,
        wasm_path,
        root,
        manifest_path: path.to_path_buf(),
    })
}

fn normalize_path_or_id(input: &str) -> Cow<'_, str> {
    if let Some(rest) = input.strip_prefix("file://") {
        Cow::Owned(rest.to_string())
    } else {
        Cow::Borrowed(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn manifest_json(artifact: &str, hash: &str) -> String {
        format!(
            r#"{{
  "id": "com.greentic.test.component",
  "name": "Test Component",
  "version": "0.1.0",
  "world": "greentic:component/component@0.6.0",
  "describe_export": "describe",
  "operations": [{{
    "name": "run",
    "input_schema": {{"type":"object","properties":{{}},"required":[],"additionalProperties":false}},
    "output_schema": {{"type":"object","properties":{{}},"required":[],"additionalProperties":false}}
  }}],
  "default_operation": "run",
  "supports": ["messaging"],
  "profiles": {{"default": "stateless", "supported": ["stateless"]}},
  "secret_requirements": [],
  "capabilities": {{
    "wasi": {{
      "filesystem": {{"mode":"none","mounts":[]}},
      "random": true,
      "clocks": true
    }},
    "host": {{
      "messaging": {{"inbound": true, "outbound": true}},
      "telemetry": {{"scope": "tenant"}}
    }}
  }},
  "config_schema": {{"type":"object","properties":{{}},"required":[],"additionalProperties":false}},
  "limits": {{"memory_mb": 64, "wall_time_ms": 1000}},
  "artifacts": {{"component_wasm": "{artifact}"}},
  "hashes": {{"component_wasm": "{hash}"}},
  "dev_flows": {{
    "default": {{
      "format": "flow-ir-json",
      "graph": {{
        "nodes": [{{"id":"start","type":"start"}}, {{"id":"end","type":"end"}}],
        "edges": [{{"from":"start","to":"end"}}]
      }}
    }}
  }}
}}"#
        )
    }

    fn write_component_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("fixture dir");
        let wasm_path = dir.path().join("component.wasm");
        fs::write(&wasm_path, b"fixture-wasm").expect("write wasm");
        let hash = format!("blake3:{}", blake3::hash(b"fixture-wasm").to_hex());
        let manifest_path = dir.path().join(MANIFEST_NAME);
        fs::write(&manifest_path, manifest_json("component.wasm", &hash)).expect("write manifest");
        (dir, manifest_path, wasm_path)
    }

    #[test]
    fn normalize_path_or_id_strips_file_scheme_only() {
        assert_eq!(
            normalize_path_or_id("file:///tmp/component"),
            "/tmp/component"
        );
        assert_eq!(normalize_path_or_id("component-id"), "component-id");
    }

    #[test]
    fn discover_with_manifest_uses_override_before_searching_by_id() {
        let (_dir, manifest_path, wasm_path) = write_component_fixture();

        let handle =
            discover_with_manifest("not-a-real-component", Some(&manifest_path)).expect("load");

        assert_eq!(handle.manifest_path, manifest_path);
        assert_eq!(handle.wasm_path, wasm_path);
    }

    #[test]
    fn load_from_manifest_reports_missing_artifact() {
        let dir = tempfile::tempdir().expect("fixture dir");
        let manifest_path = dir.path().join(MANIFEST_NAME);
        fs::write(
            &manifest_path,
            manifest_json(
                "missing/component.wasm",
                "blake3:0000000000000000000000000000000000000000000000000000000000000000",
            ),
        )
        .expect("write manifest");

        let err = load_from_manifest(&manifest_path).expect_err("artifact should be missing");
        assert!(
            matches!(err, LoadError::MissingArtifact { path } if path.ends_with("missing/component.wasm"))
        );
    }

    #[test]
    fn try_explicit_discovers_manifest_next_to_wasm() {
        let (_dir, manifest_path, wasm_path) = write_component_fixture();

        let handle = try_explicit(wasm_path.to_str().expect("utf-8 path"))
            .expect("try_explicit succeeds")
            .expect("fixture should resolve");

        assert_eq!(handle.manifest_path, manifest_path);
        assert_eq!(handle.wasm_path, wasm_path);
    }

    #[test]
    fn try_explicit_returns_none_for_missing_paths() {
        let missing = tempfile::tempdir()
            .expect("tempdir")
            .path()
            .join("missing-component");

        let resolved = try_explicit(missing.to_str().expect("utf-8")).expect("lookup succeeds");

        assert!(resolved.is_none());
    }

    #[test]
    fn try_explicit_discovers_manifest_inside_directory() {
        let (_dir, manifest_path, wasm_path) = write_component_fixture();
        let component_dir = manifest_path.parent().expect("manifest parent");

        let handle = try_explicit(component_dir.to_str().expect("utf-8"))
            .expect("try_explicit succeeds")
            .expect("fixture should resolve");

        assert_eq!(handle.manifest_path, manifest_path);
        assert_eq!(handle.wasm_path, wasm_path);
    }

    #[test]
    fn discover_reports_not_found_when_no_locations_match() {
        let err = discover("com.greentic.missing.component").expect_err("missing component");

        assert!(matches!(err, LoadError::NotFound(id) if id == "com.greentic.missing.component"));
    }

    #[test]
    fn load_from_manifest_reports_parse_errors() {
        let dir = tempfile::tempdir().expect("fixture dir");
        let manifest_path = dir.path().join(MANIFEST_NAME);
        fs::write(&manifest_path, "{not valid json").expect("write invalid manifest");

        let err = load_from_manifest(&manifest_path).expect_err("invalid manifest should fail");

        assert!(matches!(err, LoadError::Manifest { path, .. } if path == manifest_path));
    }

    #[test]
    fn load_from_manifest_returns_handle_for_valid_fixture() {
        let (_dir, manifest_path, wasm_path) = write_component_fixture();

        let handle = load_from_manifest(&manifest_path).expect("valid manifest should load");

        assert_eq!(
            handle.root,
            manifest_path.parent().expect("manifest parent")
        );
        assert_eq!(handle.manifest_path, manifest_path);
        assert_eq!(handle.wasm_path, wasm_path);
    }

    #[test]
    fn try_explicit_accepts_manifest_json_path_directly() {
        let (_dir, manifest_path, wasm_path) = write_component_fixture();

        let handle = try_explicit(manifest_path.to_str().expect("utf-8 path"))
            .expect("lookup succeeds")
            .expect("fixture should resolve");

        assert_eq!(handle.manifest_path, manifest_path);
        assert_eq!(handle.wasm_path, wasm_path);
    }

    #[test]
    fn try_explicit_returns_none_for_existing_non_manifest_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("notes.txt");
        fs::write(&existing, b"notes").expect("write note");

        let handle = try_explicit(existing.to_str().expect("utf-8")).expect("lookup succeeds");

        assert!(handle.is_none());
    }

    #[test]
    fn try_workspace_discovers_component_from_target_directory() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("cwd");
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(dir.path()).expect("set cwd");

        let profile_dir = dir.path().join("target/wasm32-wasip2/release");
        fs::create_dir_all(&profile_dir).expect("create target dir");
        let wasm_path = profile_dir.join("com.greentic.test.component.wasm");
        fs::write(&wasm_path, b"fixture-wasm").expect("write wasm");
        let hash = format!("blake3:{}", blake3::hash(b"fixture-wasm").to_hex());
        let manifest_path = profile_dir.join(MANIFEST_NAME);
        fs::write(
            &manifest_path,
            manifest_json("com.greentic.test.component.wasm", &hash),
        )
        .expect("write manifest");

        let handle = try_workspace("com.greentic.test.component")
            .expect("workspace lookup")
            .expect("fixture should resolve");

        assert_eq!(
            handle.manifest_path,
            manifest_path.canonicalize().expect("canonical manifest")
        );
        assert_eq!(
            handle.wasm_path,
            wasm_path.canonicalize().expect("canonical wasm")
        );

        std::env::set_current_dir(original_cwd).expect("restore cwd");
    }

    #[test]
    fn try_workspace_ignores_wasm_without_adjacent_manifest() {
        let _guard = cwd_lock().lock().expect("cwd lock");
        let original_cwd = std::env::current_dir().expect("cwd");
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_current_dir(dir.path()).expect("set cwd");

        let profile_dir = dir.path().join("target/wasm32-wasip2/release");
        fs::create_dir_all(&profile_dir).expect("create target dir");
        fs::write(
            profile_dir.join("com.greentic.test.component.wasm"),
            b"fixture-wasm",
        )
        .expect("write wasm");

        let handle = try_workspace("com.greentic.test.component").expect("workspace lookup");

        assert!(handle.is_none());

        std::env::set_current_dir(original_cwd).expect("restore cwd");
    }
}
