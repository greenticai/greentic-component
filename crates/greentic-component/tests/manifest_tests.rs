use std::fs;
use std::path::Path;

use greentic_component::manifest::{
    DescribeKind, ManifestError, parse_manifest, validate_manifest,
};
use greentic_types::flow::FlowKind;
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifests")
        .join(name);
    fs::read_to_string(path).expect("fixture should exist")
}

#[test]
fn round_trip_manifest_parse() {
    let raw = fixture("valid.component.json");
    let manifest = parse_manifest(&raw).expect("manifest parses");
    assert_eq!(manifest.id.as_str(), "com.greentic.demo.echo");
    assert_eq!(manifest.version.to_string(), "0.3.0");
    assert_eq!(manifest.describe_export.kind(), DescribeKind::Export);
    assert_eq!(
        manifest.supports,
        vec![FlowKind::Messaging, FlowKind::Event]
    );
    assert_eq!(manifest.operations.len(), 1);
    assert_eq!(manifest.operations[0].name, "handle_message".to_string());
    assert!(manifest.operations[0].input_schema.is_object());
    assert!(manifest.operations[0].output_schema.is_object());
    assert_eq!(
        manifest.default_operation.as_deref(),
        Some("handle_message")
    );
    assert_eq!(
        manifest.profiles.supported,
        vec!["stateless".to_string(), "cached".to_string()]
    );
    assert_eq!(manifest.secret_requirements.len(), 1);
    assert_eq!(manifest.secret_requirements[0].key.as_str(), "KV_API_TOKEN");
    assert!(manifest.telemetry.is_some());
}

#[test]
fn schema_validation_fails_for_missing_fields() {
    let raw = fixture("invalid.component.json");
    match parse_manifest(&raw).unwrap_err() {
        ManifestError::Schema(_) => {}
        err => panic!("expected schema error, got {err:?}"),
    }
}

#[test]
fn schema_validation_fails_for_string_operations() {
    let mut value: Value = serde_json::from_str(&fixture("valid.component.json")).unwrap();
    value["operations"] = serde_json::json!(["handle_message"]);
    let raw = serde_json::to_string(&value).unwrap();
    match parse_manifest(&raw).unwrap_err() {
        ManifestError::Schema(_) => {}
        err => panic!("expected schema error, got {err:?}"),
    }
}

#[test]
fn semver_validation_reports_leading_zero() {
    let raw = fixture("valid.component.json");
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["version"] = Value::String("01.0.0".into());
    let raw_with_bad_version = serde_json::to_string(&value).unwrap();
    match parse_manifest(&raw_with_bad_version).unwrap_err() {
        ManifestError::InvalidVersion { .. } => {}
        err => panic!("expected InvalidVersion, got {err:?}"),
    }
}

#[test]
fn relative_artifact_path_required() {
    let raw = fixture("valid.component.json");
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["artifacts"]["component_wasm"] = Value::String("/abs/component.wasm".into());
    let serialized = serde_json::to_string(&value).unwrap();
    match parse_manifest(&serialized).unwrap_err() {
        ManifestError::InvalidArtifactPath { .. } => {}
        err => panic!("expected InvalidArtifactPath, got {err:?}"),
    }
}

#[test]
fn manifest_schema_helper_exposes_json() {
    assert!(greentic_component::manifest_schema().contains("$schema"));
}

#[test]
fn validate_manifest_round_trip() {
    let raw = fixture("valid.component.json");
    validate_manifest(&raw).expect("schema-valid manifest");
}

#[test]
fn state_delete_requires_write() {
    let mut value: Value = serde_json::from_str(&fixture("valid.component.json")).unwrap();
    value["capabilities"]["host"]["state"]["delete"] = Value::Bool(true);
    value["capabilities"]["host"]["state"]["write"] = Value::Bool(false);
    let raw = serde_json::to_string(&value).unwrap();
    let manifest = parse_manifest(&raw).expect("manifest parses");
    let state = manifest.capabilities.host.state.expect("state caps");
    assert!(state.write, "delete should imply write");
}

#[test]
fn manifest_preserves_dev_flows() {
    let mut value: Value = serde_json::from_str(&fixture("valid.component.json")).unwrap();
    value["dev_flows"] = serde_json::json!({
        "default": {
            "format": "flow-ir-json",
            "graph": {
                "nodes": [
                    { "id": "start", "type": "start" },
                    { "id": "end", "type": "end" }
                ],
                "edges": [
                    { "from": "start", "to": "end" }
                ]
            }
        }
    });
    let serialized = serde_json::to_string(&value).unwrap();
    parse_manifest(&serialized).expect("manifest with dev_flows parses");
    validate_manifest(&serialized).expect("schema-valid manifest with dev_flows");
}

#[test]
fn parse_manifest_uses_host_secret_requirements_when_top_level_is_empty() {
    let mut value: Value = serde_json::from_str(&fixture("valid.component.json")).unwrap();
    value["secret_requirements"] = serde_json::json!([]);
    let raw = serde_json::to_string(&value).unwrap();
    let manifest = parse_manifest(&raw).expect("manifest parses");
    assert_eq!(manifest.secret_requirements.len(), 1);
    assert_eq!(manifest.secret_requirements[0].key.as_str(), "KV_API_TOKEN");
}

#[test]
fn parse_manifest_rejects_mismatched_secret_surfaces() {
    let mut value: Value = serde_json::from_str(&fixture("valid.component.json")).unwrap();
    value["capabilities"]["host"]["secrets"]["required"][0]["key"] =
        Value::String("OTHER_TOKEN".into());
    let raw = serde_json::to_string(&value).unwrap();
    match parse_manifest(&raw).unwrap_err() {
        ManifestError::InconsistentSecretRequirements => {}
        err => panic!("expected InconsistentSecretRequirements, got {err:?}"),
    }
}
