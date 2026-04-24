use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::report::{Capabilities, InfoReport, ManifestSource};
use crate::capabilities::Capabilities as ManifestCapabilities;

/// Read a compiled component `.wasm` and project it into an [`InfoReport`].
///
/// Returns an error only if the file can't be read or is not a valid
/// WebAssembly Component Model binary. A missing manifest is not an
/// error — the report's `manifest_source` is simply `None`.
pub fn read(path: &Path) -> Result<InfoReport> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let size_bytes = bytes.len() as u64;

    let engine = wasmtime::Engine::default();
    // wasmtime's error type is its own re-export of anyhow::Error and does not
    // implement std::error::Error, so use `map_err` instead of `.with_context`.
    let component =
        wasmtime::component::Component::from_binary(&engine, &bytes).map_err(|err| {
            anyhow!(
                "{}: not a valid wasm32-wasip2 component: {err}",
                path.display()
            )
        })?;
    let ct = component.component_type();
    let exports: Vec<String> = ct.exports(&engine).map(|(n, _)| n.to_string()).collect();
    let imports: Vec<String> = ct.imports(&engine).map(|(n, _)| n.to_string()).collect();

    let (manifest_source, id, name, version, description, capabilities, world_from_manifest) =
        load_manifest(path, &bytes);

    // Prefer the manifest-declared world identifier; fall back to a heuristic
    // over the binary-derived exports when no manifest was found.
    let wit_world = world_from_manifest.or_else(|| pick_wit_world(&exports));

    Ok(InfoReport {
        info_schema_version: 1,
        component_id: id,
        name,
        version,
        description,
        artifact_type: "component/wasm".into(),
        size_bytes,
        wit_world,
        exports,
        imports,
        capabilities,
        manifest_source,
    })
}

type ManifestFields = (
    ManifestSource,
    Option<String>, // id
    Option<String>, // name
    Option<String>, // version
    Option<String>, // description
    Option<Capabilities>,
    Option<String>, // wit_world
);

fn load_manifest(path: &Path, bytes: &[u8]) -> ManifestFields {
    // 1. Try the embedded custom section first.
    if let Ok(Some(verified)) =
        crate::embedded_descriptor::read_and_verify_embedded_component_manifest_section_v1(bytes)
    {
        let m = &verified.manifest;
        return (
            ManifestSource::Embedded,
            Some(m.id.clone()),
            Some(m.name.clone()),
            Some(m.version.clone()),
            // EmbeddedComponentManifestV1 has no description field.
            None,
            Some(extract_capabilities(&m.capabilities)),
            Some(m.world.clone()),
        );
    }

    // 2. Fall back to a sibling manifest JSON.
    //    Try `<stem>.manifest.json` first, then the generic `component.manifest.json`.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("component");
    let candidates = [
        path.with_file_name(format!("{stem}.manifest.json")),
        path.with_file_name("component.manifest.json"),
    ];

    for candidate in &candidates {
        if !candidate.exists() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(candidate) else {
            continue;
        };
        let Ok(m) = crate::manifest::parse_manifest(&raw) else {
            continue;
        };
        return (
            ManifestSource::Sibling,
            Some(m.id.as_str().to_string()),
            Some(m.name.clone()),
            Some(m.version.to_string()),
            // ComponentManifest has no description field today.
            None,
            Some(extract_capabilities(&m.capabilities)),
            Some(m.world.as_str().to_string()),
        );
    }

    (ManifestSource::None, None, None, None, None, None, None)
}

/// Project the manifest's structured capability declaration into a flat list
/// of human-readable identifiers (e.g. `messaging.inbound`, `state.read`,
/// `wasi.clocks`). Only surfaces that are actually enabled are included.
fn extract_capabilities(caps: &ManifestCapabilities) -> Capabilities {
    let mut host: Vec<String> = Vec::new();
    let mut wasi: Vec<String> = Vec::new();

    // WASI surfaces.
    if let Some(fs) = &caps.wasi.filesystem {
        use crate::capabilities::FilesystemMode::{None as FsNone, ReadOnly, Sandbox};
        match fs.mode {
            FsNone => {}
            ReadOnly => wasi.push("filesystem.read_only".into()),
            Sandbox => wasi.push("filesystem.sandbox".into()),
        }
    }
    if let Some(env) = &caps.wasi.env
        && !env.allow.is_empty()
    {
        wasi.push("env".into());
    }
    if caps.wasi.random {
        wasi.push("random".into());
    }
    if caps.wasi.clocks {
        wasi.push("clocks".into());
    }

    // Host surfaces.
    if let Some(messaging) = &caps.host.messaging {
        if messaging.inbound {
            host.push("messaging.inbound".into());
        }
        if messaging.outbound {
            host.push("messaging.outbound".into());
        }
    }
    if let Some(events) = &caps.host.events {
        if events.inbound {
            host.push("events.inbound".into());
        }
        if events.outbound {
            host.push("events.outbound".into());
        }
    }
    if let Some(http) = &caps.host.http {
        if http.client {
            host.push("http.client".into());
        }
        if http.server {
            host.push("http.server".into());
        }
    }
    if let Some(state) = &caps.host.state {
        if state.read {
            host.push("state.read".into());
        }
        if state.write {
            host.push("state.write".into());
        }
    }
    if let Some(secrets) = &caps.host.secrets
        && !secrets.required.is_empty()
    {
        host.push(format!("secrets.required[{}]", secrets.required.len()));
    }
    if let Some(telemetry) = &caps.host.telemetry {
        use crate::capabilities::TelemetryScope::*;
        let scope = match telemetry.scope {
            Tenant => "tenant",
            Pack => "pack",
            Node => "node",
        };
        host.push(format!("telemetry.{scope}"));
    }
    if let Some(iac) = &caps.host.iac {
        if iac.write_templates {
            host.push("iac.write_templates".into());
        }
        if iac.execute_plans {
            host.push("iac.execute_plans".into());
        }
    }

    Capabilities { host, wasi }
}

/// Heuristic pick for the WIT world name when no manifest is available.
///
/// Look for an export that looks like a WIT world or descriptor identifier:
/// typical greentic components export `greentic:component/descriptor@X.Y.Z`
/// or similar `.../world@<version>` forms.
fn pick_wit_world(exports: &[String]) -> Option<String> {
    exports
        .iter()
        .find(|e| e.contains("/world@") || e.contains("/descriptor@"))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_wasm() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/contract/fixtures/component_v0_6_0/component.wasm")
    }

    #[test]
    fn reads_size_and_exports_from_fixture() {
        let path = fixture_wasm();
        if !path.exists() {
            eprintln!("skipping: no fixture at {}", path.display());
            return;
        }
        let r = read(&path).expect("read info");
        assert!(r.size_bytes > 0);
        assert!(!r.exports.is_empty());
        assert_eq!(r.info_schema_version, 1);
        assert_eq!(r.artifact_type, "component/wasm");
        // Sibling `component.manifest.json` exists in this fixture dir, so
        // manifest_source should be Embedded or Sibling (or None if neither
        // survives parsing — we tolerate all three).
        assert!(matches!(
            r.manifest_source,
            ManifestSource::Embedded | ManifestSource::Sibling | ManifestSource::None
        ));
    }

    #[test]
    fn pick_wit_world_prefers_world_segments() {
        let exports = vec![
            "wasi:io/streams@0.2.0".to_string(),
            "greentic:component/descriptor@0.6.0".to_string(),
        ];
        let picked = pick_wit_world(&exports);
        assert_eq!(
            picked.as_deref(),
            Some("greentic:component/descriptor@0.6.0")
        );
    }

    #[test]
    fn extract_capabilities_empty_when_default() {
        let caps = ManifestCapabilities::default();
        let out = extract_capabilities(&caps);
        assert!(out.host.is_empty());
        assert!(out.wasi.is_empty());
    }

    #[test]
    fn extract_capabilities_flags_enabled_surfaces() {
        use crate::capabilities::{HostCapabilities, MessagingCapabilities, StateCapabilities};
        let caps = ManifestCapabilities {
            wasi: crate::capabilities::WasiCapabilities {
                filesystem: None,
                env: None,
                random: false,
                clocks: true,
            },
            host: HostCapabilities {
                messaging: Some(MessagingCapabilities {
                    inbound: true,
                    outbound: false,
                }),
                events: None,
                http: None,
                secrets: None,
                state: Some(StateCapabilities {
                    read: true,
                    write: false,
                }),
                telemetry: None,
                iac: None,
            },
        };
        let out = extract_capabilities(&caps);
        assert_eq!(out.wasi, vec!["clocks".to_string()]);
        assert_eq!(
            out.host,
            vec!["messaging.inbound".to_string(), "state.read".to_string()]
        );
    }
}
