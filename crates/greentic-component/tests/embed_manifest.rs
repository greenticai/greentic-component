#![cfg(feature = "cli")]

use assert_cmd::cargo::cargo_bin_cmd;
use greentic_component::embedded_descriptor::{
    build_embedded_manifest_projection, decode_embedded_component_descriptor_v1,
    encode_embedded_component_descriptor_v1, read_embedded_component_manifest_section_v1,
};
use serde_json::Value as JsonValue;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;
use wasm_encoder::{CustomSection, Encode, Section};
use wit_component::encode as encode_wit;
use wit_parser::Resolve;

const TEST_WIT: &str = r#"
package greentic:component@0.5.0;
world node {
    export describe: func();
}
"#;

fn write_fake_cargo(dir: &Path) -> std::path::PathBuf {
    let script = "#!/bin/sh\nset -e\nexit 0\n".to_string();
    let path = dir.join("fake_cargo.sh");
    fs::write(&path, script).expect("write fake cargo");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod");
    path
}

fn write_component_wasm(dir: &Path, wasm_name: &str) {
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_str("component.wit", TEST_WIT)
        .expect("push wit");
    let mut wasm = encode_wit(&resolve, pkg).expect("encode component");
    let section = CustomSection {
        name: "producers".into(),
        data: "wasm32-wasip2".as_bytes().into(),
    };
    wasm.push(section.id());
    section.encode(&mut wasm);
    fs::write(dir.join(wasm_name), wasm).expect("write wasm");
}

fn minimal_manifest() -> JsonValue {
    serde_json::json!({
        "id": "ai.greentic.example",
        "name": "example",
        "version": "0.1.0",
        "world": "greentic:component/node@0.5.0",
        "describe_export": "get-manifest",
        "operations": [
            {
                "name": "handle_message",
                "input_schema": {
                    "type": "object",
                    "properties": { "payload": { "type": "string", "default": "ping" } },
                    "required": ["payload"]
                },
                "output_schema": {
                    "type": "object",
                    "properties": { "result": { "type": "string", "default": "ok" } },
                    "required": ["result"]
                }
            }
        ],
        "default_operation": "handle_message",
        "config_schema": {
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        },
        "supports": ["messaging"],
        "profiles": {
            "default": "stateless",
            "supported": ["stateless"]
        },
        "secret_requirements": [],
        "capabilities": {
            "wasi": {
                "filesystem": { "mode": "none", "mounts": [] },
                "random": true,
                "clocks": true
            },
            "host": {
                "messaging": { "inbound": true, "outbound": true },
                "telemetry": { "scope": "node" },
                "secrets": { "required": [] }
            }
        },
        "limits": { "memory_mb": 64, "wall_time_ms": 1000 },
        "artifacts": { "component_wasm": "component.wasm" },
        "hashes": { "component_wasm": "blake3:0000000000000000000000000000000000000000000000000000000000000000" }
    })
}

#[test]
fn embedded_manifest_envelope_round_trips_deterministically() {
    let manifest_json = minimal_manifest();
    let manifest = greentic_component::parse_manifest(&manifest_json.to_string()).unwrap();
    let projection = build_embedded_manifest_projection(&manifest);

    let (bytes_a, envelope_a, _payload_a) =
        encode_embedded_component_descriptor_v1(&projection).unwrap();
    let (bytes_b, envelope_b, _payload_b) =
        encode_embedded_component_descriptor_v1(&projection).unwrap();

    assert_eq!(bytes_a, bytes_b);
    assert_eq!(
        envelope_a.payload_hash_blake3,
        envelope_b.payload_hash_blake3
    );

    let decoded = decode_embedded_component_descriptor_v1(&bytes_a).unwrap();
    assert_eq!(decoded.envelope.kind, "greentic.component.manifest");
    assert_eq!(decoded.manifest, projection);
}

#[test]
fn build_embeds_manifest_custom_section_into_wasm() {
    let temp = TempDir::new().expect("tempdir");
    let manifest_path = temp.path().join("component.manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&minimal_manifest()).unwrap(),
    )
    .expect("write manifest");

    write_component_wasm(temp.path(), "component.wasm");
    let fake_cargo = write_fake_cargo(temp.path());

    let mut cmd = cargo_bin_cmd!("greentic-component");
    cmd.current_dir(temp.path())
        .env("CARGO", &fake_cargo)
        .env("GREENTIC_SKIP_NODE_EXPORT_CHECK", "1")
        .arg("build")
        .arg("--no-flow");
    cmd.assert().success();

    let wasm_bytes = fs::read(temp.path().join("component.wasm")).expect("read wasm");
    let section = read_embedded_component_manifest_section_v1(&wasm_bytes)
        .expect("read embedded section")
        .expect("embedded section should exist");
    let decoded = decode_embedded_component_descriptor_v1(&section).expect("decode section");
    assert_eq!(decoded.envelope.version, 1);
    assert_eq!(
        decoded.envelope.payload_schema.as_deref(),
        Some("greentic.component.manifest.v1")
    );
    assert_eq!(decoded.manifest.name, "example");
    assert_eq!(
        decoded.manifest.default_operation.as_deref(),
        Some("handle_message")
    );
}

#[test]
fn decode_fails_on_payload_hash_mismatch() {
    let manifest_json = minimal_manifest();
    let manifest = greentic_component::parse_manifest(&manifest_json.to_string()).unwrap();
    let projection = build_embedded_manifest_projection(&manifest);
    let (bytes, mut envelope, _payload) =
        encode_embedded_component_descriptor_v1(&projection).unwrap();
    let _ = bytes;
    envelope.payload_hash_blake3 =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    let tampered =
        greentic_types::cbor::canonical::to_canonical_cbor_allow_floats(&envelope).unwrap();
    let err = decode_embedded_component_descriptor_v1(&tampered).unwrap_err();
    assert!(err.to_string().contains("payload hash mismatch"));
}
