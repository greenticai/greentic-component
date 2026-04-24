#![cfg(all(feature = "cli", feature = "prepare"))]

use assert_cmd::prelude::*;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::cargo_bin("greentic-component").expect("cargo_bin")
}

fn fixture_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contract/fixtures/component_v0_6_0/component.wasm")
}

#[test]
fn human_output_on_fixture() {
    let path = fixture_wasm();
    if !path.exists() {
        eprintln!("skipping: fixture missing at {}", path.display());
        return;
    }
    let out = bin()
        .args(["info", path.to_str().unwrap()])
        .output()
        .expect("run info");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("Artifact type"),
        "missing Artifact type row:\n{s}"
    );
    assert!(s.contains("Size"), "missing Size row:\n{s}");
    // One of Exports or "manifest not found" must appear.
    assert!(
        s.contains("Exports") || s.contains("manifest not found"),
        "missing Exports section and no manifest-missing notice:\n{s}"
    );
}

#[test]
fn json_output_has_schema_version() {
    let path = fixture_wasm();
    if !path.exists() {
        eprintln!("skipping: fixture missing at {}", path.display());
        return;
    }
    let out = bin()
        .args(["info", path.to_str().unwrap(), "--json"])
        .output()
        .expect("run info --json");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["info_schema_version"], 1);
    assert_eq!(v["artifact_type"], "component/wasm");
    assert!(v["exports"].is_array());
    assert!(v["imports"].is_array());
}

#[test]
fn missing_file_exits_2() {
    let out = bin()
        .args(["info", "/nope/does-not-exist.wasm"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn wrong_extension_exits_2() {
    let out = bin().args(["info", "Cargo.toml"]).output().expect("run");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn not_a_component_exits_5() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("bad.wasm");
    std::fs::write(&path, b"this is not wasm").unwrap();
    let out = bin()
        .args(["info", path.to_str().unwrap()])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(5));
}
