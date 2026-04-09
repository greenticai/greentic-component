#![cfg(feature = "cli")]

use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::TempDir;
use serde_json::Value as JsonValue;

#[test]
fn updates_dev_flows_from_manifest_schema() {
    let temp = TempDir::new().expect("tempdir");
    let manifest = r#"
{
  "id": "component-demo",
  "name": "component-demo",
  "operations": [
    {
      "name": "handle_message",
      "input_schema": {
        "type": "object",
        "properties": {
          "title": { "type": "string", "default": "Hello world" },
          "threshold": { "type": "number", "default": 0.42 },
          "kind": { "enum": ["Text", "Number"], "default": "Text" },
          "internal": { "type": "string", "default": "skip-me", "x_flow_hidden": true }
        },
        "required": ["title", "threshold"]
      },
      "output_schema": {}
    }
  ],
  "default_operation": "handle_message",
  "config_schema": {
    "type": "object",
    "properties": {
      "title": {
        "type": "string",
        "description": "Greeting shown to users",
        "default": "Hello world"
      },
      "threshold": {
        "type": "number",
        "default": 0.42
      },
      "kind": {
        "enum": ["Text", "Number"],
        "description": "Answer type",
        "default": "Text"
      },
      "internal": {
        "type": "string",
        "x_flow_hidden": true,
        "default": "skip-me"
      }
    },
    "required": ["title", "threshold"]
  }
}
"#;
    fs::write(temp.path().join("component.manifest.json"), manifest).expect("write manifest");

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert().success();

    let manifest_after =
        fs::read_to_string(temp.path().join("component.manifest.json")).expect("manifest");
    let value: JsonValue = serde_json::from_str(&manifest_after).expect("json manifest");

    let default_flow = &value["dev_flows"]["default"];
    assert_eq!(default_flow["format"], "flow-ir-json");
    let default_graph = &default_flow["graph"];
    assert_eq!(default_graph["id"], "component-demo.default");
    assert_eq!(default_graph["kind"], "component-config");
    let default_template = default_graph["nodes"]["emit_config"]["template"]
        .as_str()
        .expect("default template");
    let default_payload: JsonValue =
        serde_json::from_str(default_template).expect("default template json");
    assert!(
        default_template.contains("NEXT_NODE_PLACEHOLDER")
            && !default_template.contains("COMPONENT_STEP")
            && !default_template.contains("\"tool\""),
        "template should be add-step ready"
    );
    assert_eq!(default_payload["node_id"], "component-demo");
    let exec_node = &default_payload["node"]["handle_message"];
    assert_eq!(exec_node["input"]["title"], "Hello world");
    assert_eq!(exec_node["input"]["threshold"], 0.42);
    assert!(
        default_payload["node"]["handle_message"]["input"]
            .get("kind")
            .is_none(),
        "optional fields should be omitted in default flow"
    );

    let custom_flow = &value["dev_flows"]["custom"];
    assert_eq!(custom_flow["format"], "flow-ir-json");
    let custom_graph = &custom_flow["graph"];
    assert_eq!(custom_graph["id"], "component-demo.custom");
    let question_fields = custom_graph["nodes"]["ask_config"]["questions"]["fields"]
        .as_array()
        .expect("question fields");
    let field_ids: Vec<String> = question_fields
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(field_ids, vec!["kind", "threshold", "title"]);
    let kind_field = question_fields
        .iter()
        .find(|entry| entry["id"] == "kind")
        .expect("kind field");
    let options = kind_field["options"].as_array().expect("enum options");
    assert_eq!(
        options
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        vec!["Text", "Number"]
    );
    let custom_template = custom_graph["nodes"]["emit_config"]["template"]
        .as_str()
        .expect("template string");
    assert!(
        custom_template.contains(r#""handle_message": {"#)
            && custom_template.contains(r#""node_id": "component-demo""#),
        "operation node should be emitted"
    );
    assert!(
        custom_template.contains(r#""title": "{{state.title}}""#),
        "string fields should be quoted state values"
    );
    assert!(
        custom_template.contains(r#""threshold": {{state.threshold}}"#),
        "number fields should be raw state values"
    );
    assert!(
        !custom_template.contains("internal"),
        "hidden fields should be skipped"
    );
}

#[test]
fn flow_update_is_idempotent() {
    let temp = TempDir::new().expect("tempdir");
    let manifest = r#"{"id":"component-demo","name":"component-demo","operations":[{"name":"handle_message","input_schema":{"type":"object","properties":{"input":{"type":"string","default":"hi"}},"required":["input"]},"output_schema":{}}],"config_schema":{"type":"object","properties":{},"required":[]}}"#;
    fs::write(temp.path().join("component.manifest.json"), manifest).expect("write manifest");

    let mut first = cargo_bin_cmd!("greentic-component");
    first.current_dir(temp.path()).arg("flow").arg("update");
    first.assert().success();
    let initial = fs::read_to_string(temp.path().join("component.manifest.json")).unwrap();

    let mut second = cargo_bin_cmd!("greentic-component");
    second.current_dir(temp.path()).arg("flow").arg("update");
    second.assert().success();
    let after = fs::read_to_string(temp.path().join("component.manifest.json")).unwrap();

    assert_eq!(initial, after, "running update twice should be stable");
}

#[test]
fn infers_schema_from_wit_when_missing() {
    let temp = TempDir::new().expect("tempdir");
    let manifest = r#"{"id":"component-demo","name":"component-demo","world":"demo:component/component@0.1.0","operations":[{"name":"handle_message","input_schema":{"type":"object","properties":{"input":{"type":"string","default":"hi"}},"required":["input"]},"output_schema":{}}]}"#;
    fs::write(temp.path().join("component.manifest.json"), manifest).expect("write manifest");
    let wit_dir = temp.path().join("wit");
    fs::create_dir_all(&wit_dir).expect("create wit dir");
    fs::write(
        wit_dir.join("world.wit"),
        r#"
package demo:component;

world component {
    import component: interface {
        @config
        record config {
            /// Demo title
            title: string,
            /// @default(5)
            max-items: u32,
        }
    }
}
"#,
    )
    .expect("write wit");

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert().success();

    let manifest_after =
        fs::read_to_string(temp.path().join("component.manifest.json")).expect("manifest");
    let manifest_json: JsonValue = serde_json::from_str(&manifest_after).unwrap();
    assert!(
        manifest_json.get("config_schema").is_some(),
        "inferred schema should be written by default"
    );
    assert!(
        manifest_json["dev_flows"].get("default").is_some(),
        "default dev flow should be generated"
    );
}

#[test]
fn fails_when_required_defaults_missing() {
    let temp = TempDir::new().expect("tempdir");
    let manifest = r#"{"id":"component-demo","name":"component-demo","operations":[{"name":"handle_message","input_schema":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]},"output_schema":{}}],"config_schema":{"type":"object","properties":{},"required":[]}}"#;
    fs::write(temp.path().join("component.manifest.json"), manifest).expect("write manifest");

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert().failure().stderr(predicates::str::contains(
        "Required field input has no default; cannot generate default dev_flow",
    ));
}
