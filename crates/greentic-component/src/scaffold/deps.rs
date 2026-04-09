use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use pathdiff::diff_paths;
use serde::Serialize;
use thiserror::Error;
use toml::{Table as TomlTable, Value as TomlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyMode {
    Local,
    CratesIo,
}

impl DependencyMode {
    pub fn from_env() -> Self {
        match env::var("GREENTIC_DEP_MODE") {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "cratesio" | "crates-io" | "crates_io" => DependencyMode::CratesIo,
                "local" => DependencyMode::Local,
                "" => DependencyMode::CratesIo,
                _ => {
                    eprintln!("Unknown GREENTIC_DEP_MODE='{value}', defaulting to cratesio mode");
                    DependencyMode::CratesIo
                }
            },
            Err(_) => DependencyMode::CratesIo,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DependencyMode::Local => "local",
            DependencyMode::CratesIo => "cratesio",
        }
    }
}

const GREENTIC_TYPES_VERSION: &str = "0.4";
const GREENTIC_INTERFACES_GUEST_VERSION: &str = "0.4";
const GREENTIC_INTERFACES_VERSION: &str = "0.4";

#[derive(Debug, Clone)]
pub struct DependencyTemplates {
    pub greentic_interfaces: String,
    pub greentic_interfaces_guest: String,
    pub greentic_types: String,
    pub relative_patch_path: Option<String>,
}

#[derive(Debug, Error)]
pub enum DependencyError {
    #[error("crates.io dependency mode forbids `path =` entries in {manifest}")]
    PathDependency { manifest: PathBuf },
    #[error("failed to read manifest {manifest}: {source}")]
    Io {
        manifest: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn resolve_dependency_templates(
    mode: DependencyMode,
    target_path: &Path,
) -> DependencyTemplates {
    match mode {
        DependencyMode::Local => resolve_local_templates(target_path),
        DependencyMode::CratesIo => DependencyTemplates {
            greentic_interfaces: format!("version = \"{GREENTIC_INTERFACES_VERSION}\""),
            greentic_interfaces_guest: format!("version = \"{GREENTIC_INTERFACES_GUEST_VERSION}\""),
            greentic_types: format!("version = \"{GREENTIC_TYPES_VERSION}\""),
            relative_patch_path: None,
        },
    }
}

fn resolve_local_templates(target_path: &Path) -> DependencyTemplates {
    let repo_root = workspace_root();
    let interfaces_root = repo_root
        .parent()
        .map(|parent| parent.join("greentic-interfaces"));
    let types_root = repo_root
        .parent()
        .map(|parent| parent.join("greentic-types"));

    let greentic_interfaces = interfaces_root
        .as_ref()
        .map(|root| root.join("crates/greentic-interfaces"))
        .filter(|path| path.exists())
        .map(|path| format!(r#"path = "{}""#, absolute_path_string(&path)))
        .unwrap_or_else(|| format!("version = \"{GREENTIC_INTERFACES_VERSION}\""));

    let greentic_interfaces_guest = interfaces_root
        .as_ref()
        .map(|root| root.join("crates/greentic-interfaces-guest"))
        .filter(|path| path.exists())
        .map(|path| format!(r#"path = "{}""#, absolute_path_string(&path)))
        .unwrap_or_else(|| format!("version = \"{GREENTIC_INTERFACES_GUEST_VERSION}\""));

    let greentic_types = types_root
        .filter(|path| path.exists())
        .map(|path| format!(r#"path = "{}""#, absolute_path_string(&path)))
        .unwrap_or_else(|| format!("version = \"{GREENTIC_TYPES_VERSION}\""));

    DependencyTemplates {
        greentic_interfaces,
        greentic_interfaces_guest,
        greentic_types,
        relative_patch_path: local_patch_path(target_path),
    }
}

fn local_patch_path(scaffold_root: &Path) -> Option<String> {
    let repo_root = workspace_root();
    let crate_root = repo_root.join("crates/greentic-component");
    if !crate_root.exists() {
        return None;
    }
    Some(greentic_component_patch_path(scaffold_root, &repo_root))
}

fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

fn greentic_component_patch_path(scaffold_root: &Path, repo_root: &Path) -> String {
    let abs = repo_root.join("crates/greentic-component");
    format!(r#"path = "{}""#, relative_path_string(scaffold_root, &abs))
}

fn relative_path_string(from: &Path, to: &Path) -> String {
    diff_paths(to, from)
        .unwrap_or_else(|| to.to_path_buf())
        .display()
        .to_string()
}

fn absolute_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

pub fn ensure_cratesio_manifest_clean(root: &Path) -> Result<(), DependencyError> {
    let manifest = root.join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).map_err(|source| DependencyError::Io {
        manifest: manifest.clone(),
        source,
    })?;
    let parsed: TomlTable = toml::from_str(&contents).map_err(|source| DependencyError::Io {
        manifest: manifest.clone(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })?;
    if manifest_has_path_dependency(&parsed) {
        return Err(DependencyError::PathDependency { manifest });
    }
    Ok(())
}

fn manifest_has_path_dependency(doc: &TomlTable) -> bool {
    has_path_dep_table(doc.get("dependencies").and_then(TomlValue::as_table))
        || has_path_dep_table(doc.get("dev-dependencies").and_then(TomlValue::as_table))
        || has_path_dep_table(doc.get("build-dependencies").and_then(TomlValue::as_table))
        || has_path_dep_workspace(doc.get("workspace").and_then(TomlValue::as_table))
        || has_path_dep_patch(doc.get("patch").and_then(TomlValue::as_table))
        || has_path_dep_target(doc.get("target").and_then(TomlValue::as_table))
}

fn has_path_dep_workspace(workspace: Option<&toml::Table>) -> bool {
    let Some(workspace) = workspace else {
        return false;
    };
    has_path_dep_table(workspace.get("dependencies").and_then(TomlValue::as_table))
}

fn has_path_dep_patch(patch: Option<&toml::Table>) -> bool {
    let Some(patch) = patch else {
        return false;
    };
    patch
        .values()
        .filter_map(TomlValue::as_table)
        .any(|registry| has_path_dep_table(Some(registry)))
}

fn has_path_dep_target(target: Option<&toml::Table>) -> bool {
    let Some(target) = target else {
        return false;
    };
    target.values().filter_map(TomlValue::as_table).any(|cfg| {
        has_path_dep_table(cfg.get("dependencies").and_then(TomlValue::as_table))
            || has_path_dep_table(cfg.get("dev-dependencies").and_then(TomlValue::as_table))
            || has_path_dep_table(cfg.get("build-dependencies").and_then(TomlValue::as_table))
    })
}

fn has_path_dep_table(table: Option<&toml::Table>) -> bool {
    let Some(table) = table else {
        return false;
    };
    table.values().any(value_has_path_key)
}

fn value_has_path_key(value: &TomlValue) -> bool {
    matches!(value, TomlValue::Table(dep) if dep.contains_key("path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;

    #[test]
    fn cratesio_manifest_rejects_path_dependencies() {
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("Cargo.toml");
        std::fs::write(&manifest, "[dependencies]\nfoo = { path = \"../foo\" }\n").unwrap();
        let err = ensure_cratesio_manifest_clean(temp.path()).unwrap_err();
        match err {
            DependencyError::PathDependency { manifest: path } => assert_eq!(path, manifest),
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn cratesio_manifest_accepts_version_dependencies() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[dependencies]\nfoo = \"0.1\"\n",
        )
        .unwrap();
        ensure_cratesio_manifest_clean(temp.path()).unwrap();
    }

    #[test]
    fn cratesio_manifest_allows_component_metadata_target() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"

[package.metadata.component.target]
world = "greentic:component/component@0.6.0"
"#,
        )
        .unwrap();
        ensure_cratesio_manifest_clean(temp.path()).unwrap();
    }
}
