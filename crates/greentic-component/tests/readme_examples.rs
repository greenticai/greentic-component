#![cfg(feature = "cli")]

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value as JsonValue;
use std::fs;
use tempfile::TempDir;

#[test]
fn readme_quickstart_example_stays_fresh() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("hello-world");

    let mut new_cmd = cargo_bin_cmd!("greentic-component");
    new_cmd
        .arg("new")
        .arg("--name")
        .arg("hello-world")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--non-interactive")
        .arg("--no-check")
        .env("HOME", temp.path())
        .env("CARGO_NET_OFFLINE", "true")
        .env("GIT_AUTHOR_NAME", "Greentic Labs")
        .env("GIT_AUTHOR_EMAIL", "greentic-labs@example.com")
        .env("GIT_COMMITTER_NAME", "Greentic Labs")
        .env("GIT_COMMITTER_EMAIL", "greentic-labs@example.com")
        .env_remove("USER")
        .env_remove("USERNAME");
    new_cmd.assert().success();

    let mut flow_cmd = cargo_bin_cmd!("greentic-component");
    flow_cmd
        .current_dir(&component_dir)
        .arg("flow")
        .arg("update")
        .env("CARGO_NET_OFFLINE", "true");
    flow_cmd.assert().success();

    let manifest_raw =
        fs::read_to_string(component_dir.join("component.manifest.json")).expect("manifest");
    let manifest_json: JsonValue =
        serde_json::from_str(&manifest_raw).expect("manifest json parses");
    assert_eq!(
        manifest_json["id"].as_str(),
        Some("ai.greentic.hello-world"),
        "scaffold id should match name/org"
    );
    let operations = manifest_json["operations"]
        .as_array()
        .expect("operations array");
    assert!(
        operations.iter().any(|op| op["name"] == "handle_message"),
        "scaffold should include handle_message operation"
    );

    let default_template =
        manifest_json["dev_flows"]["default"]["graph"]["nodes"]["emit_config"]["template"]
            .as_str()
            .expect("default template string");
    let default_flow_json: JsonValue =
        serde_json::from_str(default_template).expect("template json");
    assert_eq!(default_flow_json["node_id"], "hello-world");
    assert!(
        default_flow_json["node"].get("component.exec").is_none(),
        "YGTc v2 should not emit component.exec wrapper"
    );
    let op_node = default_flow_json["node"]["handle_message"]
        .as_object()
        .expect("operation node");
    assert!(
        op_node.contains_key("input"),
        "operation node should include input"
    );
    assert_eq!(
        default_flow_json["node"]["routing"][0]["to"],
        "NEXT_NODE_PLACEHOLDER"
    );

    let lib_rs = fs::read_to_string(component_dir.join("src/lib.rs")).expect("lib.rs");
    assert!(
        lib_rs.contains("impl node::Guest")
            && lib_rs.contains("impl component_qa::Guest")
            && lib_rs.contains("impl component_i18n::Guest")
            && lib_rs.contains("export_component_v060!("),
        "scaffold should expose canonical 0.6 node, qa, and i18n exports"
    );
}
