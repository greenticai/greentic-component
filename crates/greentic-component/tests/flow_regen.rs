#![cfg(feature = "cli")]

use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::TempDir;
use serde_json::Value as JsonValue;

fn write_stub_manifest(dir: &TempDir, dev_flows: bool) {
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "input": { "type": "string", "default": "hello" }
        },
        "required": ["input"]
    });
    let manifest = serde_json::json!({
        "id": "ai.greentic.example",
        "name": "example",
        "operations": [
            { "name": "handle_message", "input_schema": input_schema, "output_schema": {} }
        ],
        "default_operation": "handle_message",
        "config_schema": { "type": "object", "properties": {}, "required": [] },
        "supports": ["messaging"],
        "profiles": { "default": "stateless", "supported": ["stateless"] },
        "capabilities": {
            "wasi": { "filesystem": { "mode": "none", "mounts": [] }, "random": true, "clocks": true },
            "host": { "messaging": { "inbound": true, "outbound": true }, "telemetry": { "scope": "node" }, "secrets": { "required": [] } }
        },
        "limits": { "memory_mb": 64, "wall_time_ms": 1000 },
        "artifacts": { "component_wasm": "component.wasm" },
        "hashes": { "component_wasm": "blake3:0000000000000000000000000000000000000000000000000000000000000000" },
        "dev_flows": if dev_flows {
            serde_json::json!({
                "default": { "format": "flow-ir-json", "graph": { "nodes": [ { "id": "start", "type": "start" } ], "edges": [] } }
            })
        } else {
            serde_json::Value::Null
        }
    });
    fs::write(
        dir.path().join("component.manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");
}

#[test]
fn flow_update_regenerates_dev_flows_and_sets_operation() {
    let temp = TempDir::new().expect("tempdir");
    write_stub_manifest(&temp, true);

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert().success();

    let manifest_after =
        fs::read_to_string(temp.path().join("component.manifest.json")).expect("manifest");
    let json: JsonValue = serde_json::from_str(&manifest_after).expect("json");
    let default_template =
        json["dev_flows"]["default"]["graph"]["nodes"]["emit_config"]["template"]
            .as_str()
            .expect("template");
    assert!(
        default_template.contains("\"handle_message\""),
        "must emit operation key"
    );
    assert!(
        default_template.contains(r#""node_id": "example""#),
        "node_id should use manifest name"
    );
    assert!(
        default_template.contains("NEXT_NODE_PLACEHOLDER"),
        "routing placeholder should be present"
    );
    assert!(
        !default_template.contains("\"tool\""),
        "tool should not be emitted"
    );
    let parsed: JsonValue = serde_json::from_str(default_template).expect("template json");
    assert_eq!(
        parsed["node"]["handle_message"]["input"]["input"], "hello",
        "required defaults must be injected"
    );
}

#[test]
fn flow_update_errors_on_missing_required_defaults() {
    let temp = TempDir::new().expect("tempdir");
    write_stub_manifest(&temp, false);
    let manifest = serde_json::json!({
        "id": "ai.greentic.example",
        "name": "example",
        "operations": [
            {
                "name": "handle_message",
                "input_schema": {
                    "type": "object",
                    "properties": { "input": { "type": "string" } },
                    "required": ["input"]
                },
                "output_schema": {}
            }
        ],
        "default_operation": "handle_message",
        "config_schema": { "type": "object", "properties": {}, "required": [] },
        "supports": ["messaging"],
        "profiles": { "default": "stateless", "supported": ["stateless"] },
        "capabilities": {
            "wasi": { "filesystem": { "mode": "none", "mounts": [] }, "random": true, "clocks": true },
            "host": { "messaging": { "inbound": true, "outbound": true }, "telemetry": { "scope": "node" }, "secrets": { "required": [] } }
        },
        "limits": { "memory_mb": 64, "wall_time_ms": 1000 },
        "artifacts": { "component_wasm": "component.wasm" },
        "hashes": { "component_wasm": "blake3:0000000000000000000000000000000000000000000000000000000000000000" }
    });
    fs::write(
        temp.path().join("component.manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert().failure().stderr(predicates::str::contains(
        "Required field input has no default; cannot generate default dev_flow",
    ));
}

#[test]
fn flow_update_errors_when_operation_ambiguous() {
    let temp = TempDir::new().expect("tempdir");
    let manifest = serde_json::json!({
        "id": "ai.greentic.example",
        "name": "example",
        "operations": [
            { "name": "op1", "input_schema": {}, "output_schema": {} },
            { "name": "op2", "input_schema": {}, "output_schema": {} }
        ],
        "config_schema": { "type": "object", "properties": {}, "required": [] },
        "supports": ["messaging"],
        "profiles": { "default": "stateless", "supported": ["stateless"] },
        "capabilities": {
            "wasi": { "filesystem": { "mode": "none", "mounts": [] }, "random": true, "clocks": true },
            "host": { "messaging": { "inbound": true, "outbound": true }, "telemetry": { "scope": "node" }, "secrets": { "required": [] } }
        },
        "limits": { "memory_mb": 64, "wall_time_ms": 1000 },
        "artifacts": { "component_wasm": "component.wasm" },
        "hashes": { "component_wasm": "blake3:0000000000000000000000000000000000000000000000000000000000000000" }
    });
    fs::write(
        temp.path().join("component.manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path()).arg("flow").arg("update");
    cmd.assert()
        .failure()
        .stderr(predicates::str::contains("declares multiple operations"));
}
