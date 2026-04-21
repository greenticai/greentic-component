#![cfg(all(feature = "cli", feature = "prepare"))]

#[path = "support/mod.rs"]
mod support;

use greentic_component::cmd::build;
use greentic_component::cmd::build::BuildArgs;
use greentic_component::embed_and_verify_wasm;
use greentic_component::error::ComponentError;
use greentic_component::scaffold::config_schema::ConfigSchemaInput;
use greentic_component::scaffold::deps::DependencyMode;
use greentic_component::scaffold::engine::{DEFAULT_WIT_WORLD, ScaffoldEngine, ScaffoldRequest};
use greentic_component::scaffold::runtime_capabilities::RuntimeCapabilitiesInput;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use support::TestComponent;

const TEST_WIT: &str = r#"
package greentic:component@0.5.0;
world component {
    export describe: func();
}
 "#;

fn copy_component_v060_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::TempDir::new().unwrap();
    let fixture_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contract/fixtures/component_v0_6_0");
    let workdir = temp.path().join("fixture");
    fs::create_dir_all(&workdir).unwrap();
    fs::copy(
        fixture_dir.join("component.wasm"),
        workdir.join("component.wasm"),
    )
    .unwrap();
    fs::copy(
        fixture_dir.join("component.manifest.json"),
        workdir.join("component.manifest.json"),
    )
    .unwrap();
    (
        temp,
        workdir.join("component.wasm"),
        workdir.join("component.manifest.json"),
    )
}

#[test]
fn inspect_outputs_json() {
    let component = TestComponent::new(TEST_WIT, &["describe"]);
    let manifest_path = component.manifest_path.to_str().unwrap();
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-inspect");
    cmd.arg(manifest_path)
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"manifest\""));
}

#[test]
fn doctor_rejects_non_component_wasm() {
    let component = TestComponent::new(TEST_WIT, &["describe"]);
    let manifest_path = component.manifest_path.to_str().unwrap();
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-doctor");
    cmd.arg(manifest_path)
        .env("GREENTIC_SKIP_NODE_EXPORT_CHECK", "1")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to load component"));
}

#[test]
fn inspect_accepts_manifest_override() {
    let component = TestComponent::new(TEST_WIT, &["describe"]);
    let wasm_path = component.wasm_path.to_str().unwrap();
    let manifest_path = component.manifest_path.to_str().unwrap();
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-inspect");
    cmd.arg(wasm_path)
        .arg("--manifest")
        .arg(manifest_path)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "component: com.greentic.test.component",
        ));
}

#[test]
fn inspect_accepts_describe_fixture() {
    let describe_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/doctor/good_component_describe.cbor");
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-inspect");
    cmd.arg("--describe")
        .arg(describe_path)
        .arg("--json")
        .arg("--verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"operations\""));
}

#[test]
fn inspect_reports_embedded_manifest_from_wasm_json() {
    let (_temp, wasm_path, manifest_path) = copy_component_v060_fixture();
    let manifest_raw = fs::read_to_string(&manifest_path).unwrap();
    let manifest = greentic_component::parse_manifest(&manifest_raw).unwrap();
    embed_and_verify_wasm(&wasm_path, &manifest).unwrap();

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-inspect");
    cmd.arg(wasm_path)
        .arg("--manifest")
        .arg(manifest_path)
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"embedded\""))
        .stdout(predicate::str::contains("\"present\": true"))
        .stdout(predicate::str::contains("\"compare_manifest\""));
}

#[test]
fn inspect_human_output_includes_manifest_and_describe_sections_for_embedded_wasm() {
    let (_temp, wasm_path, manifest_path) = copy_component_v060_fixture();
    let manifest_raw = fs::read_to_string(&manifest_path).unwrap();
    let manifest = greentic_component::parse_manifest(&manifest_raw).unwrap();
    embed_and_verify_wasm(&wasm_path, &manifest).unwrap();

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-inspect");
    cmd.arg(wasm_path)
        .arg("--manifest")
        .arg(manifest_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("manifest: "))
        .stdout(predicate::str::contains("embedded vs manifest: Match"))
        .stdout(predicate::str::contains("embedded manifest: present"))
        .stdout(predicate::str::contains(
            "world: greentic:component/component@0.6.0",
        ))
        .stdout(predicate::str::contains("operation names: handle_message"))
        .stdout(predicate::str::contains(
            "default operation: handle_message",
        ))
        .stdout(predicate::str::contains("supports: [Messaging]"))
        .stdout(predicate::str::contains("capabilities:"))
        .stdout(predicate::str::contains("secret requirements:"))
        .stdout(predicate::str::contains("profiles:"))
        .stdout(predicate::str::contains(
            "limits: memory_mb=128 wall_time_ms=1000",
        ))
        .stdout(predicate::str::contains("describe: available"))
        .stdout(predicate::str::contains("source: wit-world"))
        .stdout(predicate::str::contains("name: component"))
        .stdout(predicate::str::contains(
            "schema id: greentic:component/component@0.6.0",
        ))
        .stdout(predicate::str::contains(
            "world: greentic:component/component@0.6.0",
        ))
        .stdout(predicate::str::contains("versions: 0.6.0"))
        .stdout(predicate::str::contains("version count: 1"))
        .stdout(predicate::str::contains("functions: 2"))
        .stdout(predicate::str::contains(
            "reason: derived from exported WIT world",
        ));
}

#[test]
fn doctor_detects_scaffold_directory() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("demo-detect");
    let engine = ScaffoldEngine::new();
    let request = ScaffoldRequest {
        name: "demo-detect".into(),
        path: root.clone(),
        template_id: "rust-wasi-p2-min".into(),
        org: "ai.greentic".into(),
        version: "0.1.0".into(),
        license: "MIT".into(),
        wit_world: DEFAULT_WIT_WORLD.into(),
        user_operations: vec!["handle_message".into()],
        default_operation: "handle_message".into(),
        runtime_capabilities: RuntimeCapabilitiesInput::default(),
        config_schema: ConfigSchemaInput::default(),
        non_interactive: true,
        year_override: Some(2030),
        dependency_mode: DependencyMode::Local,
    };
    engine.scaffold(request).unwrap();
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-doctor");
    cmd.arg(&root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unable to resolve wasm"));
}

#[test]
fn doctor_fails_when_built_wasm_is_missing_embedded_manifest() {
    let (_temp, wasm_path, _manifest_path) = copy_component_v060_fixture();
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-doctor");
    cmd.arg(wasm_path)
        .assert()
        .failure()
        .stdout(predicate::str::contains("doctor.embedded.missing"));
}

#[test]
fn doctor_no_longer_reports_missing_embedded_when_section_is_present() {
    let (_temp, wasm_path, manifest_path) = copy_component_v060_fixture();
    let manifest_raw = fs::read_to_string(&manifest_path).unwrap();
    let manifest = greentic_component::parse_manifest(&manifest_raw).unwrap();
    embed_and_verify_wasm(&wasm_path, &manifest).unwrap();

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("component-doctor");
    cmd.arg(wasm_path)
        .assert()
        .failure()
        .stdout(predicate::str::contains("doctor.embedded.missing").not());
}

#[test]
fn scaffold_makefile_uses_greentic_dev_commands() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("demo-dev");
    let engine = ScaffoldEngine::new();
    let request = ScaffoldRequest {
        name: "demo-dev".into(),
        path: root.clone(),
        template_id: "rust-wasi-p2-min".into(),
        org: "ai.greentic".into(),
        version: "0.1.0".into(),
        license: "MIT".into(),
        wit_world: DEFAULT_WIT_WORLD.into(),
        user_operations: vec!["handle_message".into()],
        default_operation: "handle_message".into(),
        runtime_capabilities: RuntimeCapabilitiesInput::default(),
        config_schema: ConfigSchemaInput::default(),
        non_interactive: true,
        year_override: Some(2030),
        dependency_mode: DependencyMode::Local,
    };
    engine.scaffold(request).unwrap();

    let makefile =
        fs::read_to_string(root.join("Makefile")).expect("Makefile should be scaffolded");
    assert!(makefile.contains("greentic-dev component build --manifest ./component.manifest.json"));
    assert!(makefile.contains(
        "greentic-dev component doctor $(WASM_OUT) --manifest ./component.manifest.json"
    ));
}

#[test]
#[ignore = "requires cargo-component on PATH; runs in nightly CI with full tooling"]
fn build_logs_resolved_component_world_version() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("build-log-world");
    let engine = ScaffoldEngine::new();
    let request = ScaffoldRequest {
        name: "build-log-world".into(),
        path: root.clone(),
        template_id: "rust-wasi-p2-min".into(),
        org: "ai.greentic".into(),
        version: "0.1.0".into(),
        license: "MIT".into(),
        wit_world: DEFAULT_WIT_WORLD.into(),
        user_operations: vec!["handle_message".into()],
        default_operation: "handle_message".into(),
        runtime_capabilities: RuntimeCapabilitiesInput::default(),
        config_schema: ConfigSchemaInput::default(),
        non_interactive: true,
        year_override: Some(2030),
        dependency_mode: DependencyMode::Local,
    };
    engine.scaffold(request).unwrap();
    let fixture_wasm =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests/bin/component.wasm");

    let cargo_wrapper = root.join("fake_cargo.sh");
    std::fs::write(
        &cargo_wrapper,
        format!(
            r#"#!/bin/sh
set -e
if [ "${{1:-}}" = "component" ] && [ "${{2:-}}" = "--version" ]; then
  echo "cargo-component-component 0.21.1"
  exit 0
fi

wasm_path=$(python3 - <<'PY'
import json, os
path=os.path.join(os.getcwd(),"component.manifest.json")
try:
    with open(path, "r") as f:
        data=json.load(f)
    print(data.get("artifacts", {{}}).get("component_wasm") or "target/wasm32-wasip2/release/component.wasm")
except Exception:
    print("target/wasm32-wasip2/release/component.wasm")
PY
)
mkdir -p "$(dirname "$wasm_path")"
cp "{fixture_wasm}" "$wasm_path"

if [ "${{1:-}}" = "component" ] && [ "${{2:-}}" = "build" ]; then
  exit 0
fi

if [ "${{1:-}}" = "build" ]; then
  exit 0
fi

REAL_CARGO="$(command -v cargo)"
"$REAL_CARGO" "$@"
"#,
            fixture_wasm = fixture_wasm.display()
        ),
    )
    .expect("write cargo wrapper");
    let mut perms = std::fs::metadata(&cargo_wrapper)
        .expect("metadata")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&cargo_wrapper, perms).expect("chmod");
    }

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    cmd.current_dir(&root)
        .env("CARGO", &cargo_wrapper)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GREENTIC_SKIP_NODE_EXPORT_CHECK", "1")
        .arg("build")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Resolved manifest world: greentic:component/component@0.6.0")
                .and(predicate::str::contains("component@0.5.0").not()),
        );
}

#[test]
fn new_outputs_template_metadata_in_json() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = temp.path().join("json-demo");
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    let assert = cmd
        .arg("new")
        .arg("--name")
        .arg("json-demo")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&project)
        .arg("--no-check")
        .arg("--no-git")
        .arg("--json")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .assert()
        .success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let value: Value = serde_json::from_str(&output).expect("json");
    assert_eq!(
        value["scaffold"]["template"].as_str().unwrap(),
        "rust-wasi-p2-min"
    );
    assert_eq!(
        value["scaffold"]["template_description"].as_str().unwrap(),
        "Minimal Rust + WASI-P2 component starter"
    );
    assert_eq!(
        value["post_init"]["git"]["status"].as_str().unwrap(),
        "skipped"
    );
    assert!(
        value["post_init"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["stage"] == "git-init")
    );
}

#[test]
#[cfg(feature = "store")]
fn store_fetch_accepts_source_and_out_dir() {
    let temp = tempfile::TempDir::new().unwrap();
    let source_path = temp.path().join("component.wasm");
    fs::write(&source_path, b"fake-wasm").unwrap();

    let out_dir = temp.path().join("out");
    let cache_dir = temp.path().join("cache");
    let source_ref = format!("file://{}", source_path.display());

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    cmd.arg("store")
        .arg("fetch")
        .arg("--out")
        .arg(&out_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg(&source_ref)
        .assert()
        .success();

    let fetched = fs::read(out_dir.join("component.wasm")).expect("fetched component");
    assert_eq!(fetched, b"fake-wasm");
}

#[test]
#[cfg(feature = "store")]
fn store_fetch_accepts_wasm_output_path() {
    let temp = tempfile::TempDir::new().unwrap();
    let source_path = temp.path().join("component.wasm");
    fs::write(&source_path, b"fake-wasm").unwrap();

    let out_file = temp.path().join("offline_comp.wasm");
    let cache_dir = temp.path().join("cache");
    let source_ref = format!("file://{}", source_path.display());

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    cmd.arg("store")
        .arg("fetch")
        .arg("--out")
        .arg(&out_file)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg(&source_ref)
        .assert()
        .success();

    let fetched = fs::read(&out_file).expect("fetched component");
    assert_eq!(fetched, b"fake-wasm");
}

#[test]
#[cfg(feature = "store")]
fn store_fetch_accepts_directory_source() {
    let temp = tempfile::TempDir::new().unwrap();
    let source_dir = temp.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("component.wasm"), b"fake-wasm").unwrap();
    fs::write(
        source_dir.join("component.manifest.json"),
        r#"{"artifacts":{"component_wasm":"component.wasm"}}"#,
    )
    .unwrap();

    let out_dir = temp.path().join("out");
    let cache_dir = temp.path().join("cache");
    let source_ref = source_dir.to_string_lossy().to_string();

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    cmd.arg("store")
        .arg("fetch")
        .arg("--out")
        .arg(&out_dir)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg(&source_ref)
        .assert()
        .success();

    let fetched = fs::read(out_dir.join("component.wasm")).expect("fetched component");
    assert_eq!(fetched, b"fake-wasm");
}

#[test]
fn test_command_writes_trace_on_failure() {
    let temp = tempfile::TempDir::new().unwrap();
    let trace_path = temp.path().join("trace.json");
    let input_path = temp.path().join("input.json");
    fs::write(&input_path, "{}").unwrap();

    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests/valid.component.json");
    let wasm_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests/bin/component.wasm");

    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("greentic-component");
    cmd.arg("test")
        .arg("--wasm")
        .arg(&wasm_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--op")
        .arg("invalid_op")
        .arg("--input")
        .arg(&input_path)
        .arg("--trace-out")
        .arg(&trace_path)
        .assert()
        .failure();

    let trace = fs::read_to_string(&trace_path).expect("trace should be written");
    let value: Value = serde_json::from_str(&trace).expect("trace JSON");
    assert_eq!(value["trace_version"].as_u64(), Some(1));
    assert!(value["error"]["code"].as_str().is_some());
}

#[test]
fn build_fails_on_empty_operation_schemas() {
    let component = TestComponent::new(TEST_WIT, &["describe"]);
    rewrite_operation_schemas_to_empty(&component.manifest_path);

    let args = BuildArgs {
        manifest: component.manifest_path.clone(),
        cargo_bin: Some(true_bin()),
        no_flow: true,
        no_infer_config: true,
        no_write_schema: true,
        force_write_schema: false,
        no_validate: true,
        json: false,
        permissive: false,
    };

    let err = build::run(args).expect_err("build should fail when schemas are empty");
    let component_err = err
        .downcast_ref::<ComponentError>()
        .expect("expected a ComponentError");
    assert_eq!(component_err.code(), "E_OP_SCHEMA_EMPTY");
}

#[test]
fn build_permissive_allows_empty_operation_schemas() {
    let component = TestComponent::new(TEST_WIT, &["describe"]);
    rewrite_operation_schemas_to_empty(&component.manifest_path);

    let args = BuildArgs {
        manifest: component.manifest_path.clone(),
        cargo_bin: Some(true_bin()),
        no_flow: true,
        no_infer_config: true,
        no_write_schema: true,
        force_write_schema: false,
        no_validate: true,
        json: false,
        permissive: true,
    };

    build::run(args).expect("permissive build should succeed");
}

fn true_bin() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("TRUE_BIN") {
        return std::path::PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("true");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from("true")
}

fn rewrite_operation_schemas_to_empty(manifest_path: &Path) {
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(manifest_path).expect("read manifest")).unwrap();
    if let Some(operations) = manifest
        .get_mut("operations")
        .and_then(|value| value.as_array_mut())
    {
        for operation in operations {
            operation["input_schema"] = json!({});
            operation["output_schema"] = json!({});
        }
    }
    fs::write(
        manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
