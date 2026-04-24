use serde::{Deserialize, Serialize};

/// Structured info about a compiled component `.wasm` artifact.
///
/// Serialized as JSON when the `info --json` flag is set. The
/// `info_schema_version` field lets downstream consumers detect
/// breaking changes to this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoReport {
    pub info_schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub artifact_type: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wit_world: Option<String>,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
    pub manifest_source: ManifestSource,
}

/// Non-empty capability surfaces extracted from the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub host: Vec<String>,
    pub wasi: Vec<String>,
}

/// Where the manifest-derived fields on [`InfoReport`] came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ManifestSource {
    /// Manifest read from the `greentic.component.manifest.v1` custom section
    /// inside the `.wasm`.
    Embedded,
    /// Manifest read from a `component.manifest.json` sibling next to the `.wasm`.
    Sibling,
    /// No manifest was found; info fields are binary-derived only.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_has_schema_version_one() {
        let r = InfoReport {
            info_schema_version: 1,
            component_id: None,
            name: None,
            version: None,
            description: None,
            artifact_type: "component/wasm".into(),
            size_bytes: 0,
            wit_world: None,
            exports: vec![],
            imports: vec![],
            capabilities: None,
            manifest_source: ManifestSource::None,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["info_schema_version"], 1);
        assert_eq!(v["manifest_source"], "none");
    }

    #[test]
    fn manifest_source_serializes_lowercase() {
        let v = serde_json::to_value(ManifestSource::Embedded).unwrap();
        assert_eq!(v, serde_json::Value::String("embedded".into()));
        let v = serde_json::to_value(ManifestSource::Sibling).unwrap();
        assert_eq!(v, serde_json::Value::String("sibling".into()));
    }
}
