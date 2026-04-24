use std::fmt::Write;

use super::report::{InfoReport, ManifestSource};

/// Render an [`InfoReport`] as a human-readable block of text.
pub fn render(r: &InfoReport) -> String {
    let mut s = String::new();

    let header = match (&r.component_id, &r.version) {
        (Some(id), Some(v)) => format!("{id} {v}"),
        (Some(id), None) => id.clone(),
        (None, _) => "component/wasm".to_string(),
    };
    let _ = writeln!(s, "{header}");

    if let Some(d) = &r.description {
        let _ = writeln!(s, "{d}");
    }
    let _ = writeln!(s);

    kv(&mut s, "Artifact type", &r.artifact_type);
    kv(&mut s, "Size", &format_size(r.size_bytes));
    if let Some(w) = &r.wit_world {
        kv(&mut s, "WIT world", w);
    }

    if r.manifest_source == ManifestSource::None {
        let _ = writeln!(
            s,
            "\n(manifest not found, showing binary-derived info only)"
        );
    }

    if !r.exports.is_empty() {
        let _ = writeln!(s, "\nExports ({})", r.exports.len());
        for e in &r.exports {
            let _ = writeln!(s, "  {e}");
        }
    }
    if !r.imports.is_empty() {
        let _ = writeln!(s, "\nImports ({})", r.imports.len());
        for i in &r.imports {
            let _ = writeln!(s, "  {i}");
        }
    }
    if let Some(c) = &r.capabilities
        && (!c.host.is_empty() || !c.wasi.is_empty())
    {
        let _ = writeln!(s, "\nCapabilities");
        if !c.host.is_empty() {
            let _ = writeln!(s, "  host         {}", c.host.join(", "));
        }
        if !c.wasi.is_empty() {
            let _ = writeln!(s, "  wasi         {}", c.wasi.join(", "));
        }
    }
    s
}

fn kv(s: &mut String, k: &str, v: &str) {
    if v.is_empty() {
        return;
    }
    let _ = writeln!(s, "{:<14} {}", k, v);
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::info::report::{Capabilities, InfoReport, ManifestSource};

    #[test]
    fn shows_manifest_missing_line_when_no_manifest() {
        let r = InfoReport {
            info_schema_version: 1,
            component_id: None,
            name: None,
            version: None,
            description: None,
            artifact_type: "component/wasm".into(),
            size_bytes: 4096,
            wit_world: None,
            exports: vec!["greentic:component/descriptor@0.6.0".into()],
            imports: vec!["wasi:io/streams@0.2.0".into()],
            capabilities: None,
            manifest_source: ManifestSource::None,
        };
        let out = render(&r);
        assert!(out.contains("component/wasm"));
        assert!(out.contains("manifest not found"));
        assert!(out.contains("Exports (1)"));
        assert!(out.contains("4.0 KiB"));
    }

    #[test]
    fn renders_embedded_manifest_info() {
        let r = InfoReport {
            info_schema_version: 1,
            component_id: Some("greentic:component/adaptive-card".into()),
            name: Some("adaptive-card".into()),
            version: Some("1.6.0".into()),
            description: Some("AC renderer".into()),
            artifact_type: "component/wasm".into(),
            size_bytes: 4_405_248,
            wit_world: Some("greentic:component/world@0.6.0".into()),
            exports: vec!["greentic:component/descriptor@0.6.0".into()],
            imports: vec!["wasi:io/streams@0.2.0".into()],
            capabilities: Some(Capabilities {
                host: vec!["messaging.inbound".into(), "state.read".into()],
                wasi: vec!["clocks".into()],
            }),
            manifest_source: ManifestSource::Embedded,
        };
        let out = render(&r);
        assert!(out.contains("greentic:component/adaptive-card 1.6.0"));
        assert!(out.contains("messaging.inbound"));
        assert!(!out.contains("manifest not found"));
    }

    #[test]
    fn format_size_tiers() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MiB");
    }
}
