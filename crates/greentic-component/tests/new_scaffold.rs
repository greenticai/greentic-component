#![cfg(feature = "cli")]

use assert_cmd::prelude::*;
use greentic_component::cmd::component_world::canonical_component_world;
use greentic_types::component::ComponentManifest as TypesManifest;
use insta::assert_snapshot;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[path = "snapshot_util.rs"]
mod snapshot_util;

use snapshot_util::normalize_text;

fn cargo_component_available() -> bool {
    Command::new("cargo")
        .args(["component", "--version"])
        .status()
        .is_ok_and(|status| status.success())
}

fn wasm_target_available() -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("wasm32-wasip2")
        })
}

fn cratesio_canonical_aux_exports_available() -> bool {
    std::env::var_os("GREENTIC_CANONICAL_AUX_EXPORTS_CRATESIO").is_some()
}

fn install_cargo_wrapper(root: &Path) -> std::path::PathBuf {
    let bin_dir = root.join("test-bin");
    fs::create_dir_all(&bin_dir).expect("create test bin");
    let wrapper_path = bin_dir.join("cargo");
    let greentic_component_path = bin_dir.join("greentic-component");
    let real_cargo = Command::new("rustup")
        .args(["which", "cargo"])
        .output()
        .or_else(|_| {
            Command::new("bash")
                .args(["-lc", "command -v cargo"])
                .output()
        })
        .expect("locate cargo");
    assert!(real_cargo.status.success(), "cargo should be available");
    let real_cargo = String::from_utf8(real_cargo.stdout)
        .expect("cargo path utf8")
        .trim()
        .to_string();
    let real_component = Command::new("bash")
        .args(["-lc", "command -v cargo-component"])
        .output()
        .expect("locate cargo-component");
    assert!(
        real_component.status.success(),
        "cargo-component should be available for scaffold smoke test"
    );
    let real_component = String::from_utf8(real_component.stdout)
        .expect("cargo-component path utf8")
        .trim()
        .to_string();
    fs::write(
        &wrapper_path,
        format!(
            "#!/bin/sh\nset -eu\nif [ \"${{1:-}}\" = \"component\" ]; then\n  shift\n  exec \"{real_component}\" \"$@\"\nfi\nexec \"{real_cargo}\" \"$@\"\n"
        ),
    )
    .expect("write cargo wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper_path)
            .expect("cargo wrapper metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper_path, perms).expect("chmod cargo wrapper");
    }
    fs::write(
        &greentic_component_path,
        format!(
            "#!/bin/sh\nset -eu\nexec \"{}\" \"$@\"\n",
            assert_cmd::cargo::cargo_bin!("greentic-component").display()
        ),
    )
    .expect("write greentic-component wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&greentic_component_path)
            .expect("greentic-component wrapper metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&greentic_component_path, perms)
            .expect("chmod greentic-component wrapper");
    }
    bin_dir
}

#[test]
fn scaffold_rust_wasi_template() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("demo-component");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    cmd.arg("new")
        .arg("--name")
        .arg("demo-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--no-check")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio")
        .env("GIT_AUTHOR_NAME", "Greentic Labs")
        .env("GIT_AUTHOR_EMAIL", "greentic-labs@example.com")
        .env("GIT_COMMITTER_NAME", "Greentic Labs")
        .env("GIT_COMMITTER_EMAIL", "greentic-labs@example.com")
        .env_remove("USER")
        .env_remove("USERNAME");
    cmd.assert().success();

    let cargo = fs::read_to_string(component_dir.join("Cargo.toml")).expect("Cargo.toml");
    let manifest =
        fs::read_to_string(component_dir.join("component.manifest.json")).expect("manifest");
    let lib_rs = fs::read_to_string(component_dir.join("src/lib.rs")).expect("lib.rs");
    let qa_rs = fs::read_to_string(component_dir.join("src/qa.rs")).expect("qa.rs");
    let locales_json =
        fs::read_to_string(component_dir.join("assets/i18n/locales.json")).expect("locales.json");
    let i18n_tool = component_dir.join("tools/i18n.sh");
    let i18n_tool_contents = fs::read_to_string(&i18n_tool).expect("read scaffolded tools/i18n.sh");
    let manifest_json: JsonValue = serde_json::from_str(&manifest).expect("manifest json");
    let operations = manifest_json["operations"]
        .as_array()
        .expect("operations array in scaffold");
    assert!(
        !operations.is_empty(),
        "scaffolded manifest should include at least one operation"
    );
    let first_op = operations[0].as_object().expect("operation object");
    assert!(first_op["input_schema"].is_object());
    assert!(first_op["output_schema"].is_object());
    let first_op_name = first_op["name"].as_str().expect("operation name");
    assert_eq!(
        manifest_json["default_operation"].as_str(),
        Some(first_op_name),
        "default_operation should be set for scaffolds"
    );
    let manifest_parsed: TypesManifest =
        serde_json::from_str(&manifest).expect("manifest parses as greentic-types");
    assert!(
        !manifest_parsed.operations.is_empty(),
        "operations should deserialize"
    );
    assert_eq!(manifest_parsed.operations[0].name, "handle_message");
    assert!(
        operations.iter().any(|op| op["name"] == "qa-spec")
            && operations.iter().any(|op| op["name"] == "apply-answers")
            && operations.iter().any(|op| op["name"] == "i18n-keys"),
        "scaffold should include QA operation names"
    );
    assert!(component_dir.join("assets/i18n/en.json").exists());
    assert!(component_dir.join("assets/i18n/locales.json").exists());
    assert!(component_dir.join("tools/i18n.sh").exists());
    assert!(
        i18n_tool_contents.contains("--skip-git-repo-check"),
        "tools/i18n.sh should wrap codex exec with --skip-git-repo-check"
    );
    assert!(
        i18n_tool_contents.contains("greentic-i18n-translator"),
        "tools/i18n.sh should call greentic-i18n-translator directly"
    );
    assert!(
        i18n_tool_contents.contains("cargo binstall -y greentic-i18n-translator"),
        "tools/i18n.sh should install greentic-i18n-translator with cargo-binstall when missing"
    );
    assert!(
        qa_rs.contains("\"qa.field.api_key.label\"")
            && !qa_rs.contains("Provide values for initial provider setup."),
        "QA code path should use i18n keys, not raw user-facing strings"
    );
    let locales: JsonValue = serde_json::from_str(&locales_json).expect("locales json");
    assert_eq!(
        locales,
        serde_json::json!([
            "ar", "ar-AE", "ar-DZ", "ar-EG", "ar-IQ", "ar-MA", "ar-SA", "ar-SD", "ar-SY", "ar-TN",
            "ay", "bg", "bn", "cs", "da", "de", "el", "en-GB", "es", "et", "fa", "fi", "fr",
            "fr-FR", "gn", "gu", "hi", "hr", "ht", "hu", "id", "it", "ja", "km", "kn", "ko", "lo",
            "lt", "lv", "ml", "mr", "ms", "my", "nah", "ne", "nl", "nl-NL", "no", "pa", "pl", "pt",
            "qu", "ro", "ru", "si", "sk", "sr", "sv", "ta", "te", "th", "tl", "tr", "uk", "ur",
            "vi", "zh"
        ])
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&i18n_tool)
            .expect("i18n tool metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "tools/i18n.sh should be executable");
    }

    assert_snapshot!("scaffold_cargo_toml", normalize_text(cargo.trim()));
    assert_snapshot!("scaffold_manifest", normalize_text(manifest.trim()));
    assert_snapshot!("scaffold_lib", normalize_text(lib_rs.trim()));
    assert_eq!(
        first_op["input_schema"]["properties"]["input"]["default"]
            .as_str()
            .expect("input default"),
        "Hello from demo-component!"
    );
    let status = Command::new("cargo")
        .arg("test")
        .current_dir(&component_dir)
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_NET_OFFLINE", "true")
        .status()
        .expect("run cargo test");
    assert!(
        status.success(),
        "scaffolded project should pass host tests"
    );
    // The scaffold emits the dev-lane pre-release range
    // (`>=1.1.0-dev, <1.2.0-0`) which resolves to the published
    // `1.1.0-dev.<run_id>` artifacts. Keep this smoke test on host-only
    // checks until the wasm/cargo-component path is wired into CI.
    if cargo_component_available()
        && wasm_target_available()
        && cratesio_canonical_aux_exports_available()
    {
        let cargo_wrapper_dir = install_cargo_wrapper(temp.path());
        let path_env = format!(
            "{}:{}",
            cargo_wrapper_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut build = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
        build
            .current_dir(&component_dir)
            .env("PATH", &path_env)
            .env("CARGO_NET_OFFLINE", "true")
            .arg("build");
        build.assert().success();

        let mut doctor = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
        doctor
            .current_dir(&component_dir)
            .env("PATH", &path_env)
            .arg("doctor")
            .arg("--manifest")
            .arg("component.manifest.json")
            .arg(".");
        doctor.assert().success();
    }
    assert!(
        component_dir.join(".git").exists(),
        "post-render hook should initialize git"
    );
}

#[test]
fn doctor_validates_canonical_worlds_for_scaffold() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("canonical-component");
    let reg = assert_cmd::cargo::cargo_bin!("greentic-component");
    let mut cmd = Command::new(reg);
    cmd.arg("new")
        .arg("--name")
        .arg("canonical-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--no-check")
        .env("HOME", temp.path())
        .env("CARGO_NET_OFFLINE", "true")
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio")
        .env("GIT_AUTHOR_NAME", "Greentic Labs")
        .env("GIT_AUTHOR_EMAIL", "greentic-labs@example.com")
        .env("GIT_COMMITTER_NAME", "Greentic Labs")
        .env("GIT_COMMITTER_EMAIL", "greentic-labs@example.com")
        .env_remove("USER")
        .env_remove("USERNAME");
    cmd.assert().success();

    let manifest_path = component_dir.join("component.manifest.json");
    let manifest = fs::read_to_string(&manifest_path).expect("read scaffold manifest after build");
    let manifest_json: JsonValue =
        serde_json::from_str(&manifest).expect("manifest parses as JSON after build");
    let manifest_world = manifest_json["world"]
        .as_str()
        .expect("manifest world should be a string");
    let canonical_world = canonical_component_world();
    assert_eq!(
        canonical_world, manifest_world,
        "scaffold uses the canonical component world"
    );

    let mut doctor = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    doctor.current_dir(&component_dir).arg("doctor").arg(".");
    doctor.assert().failure().stderr(
        predicates::str::contains("unable to resolve wasm")
            .or(predicates::str::contains("failed to load component")),
    );
}

#[test]
fn doctor_accepts_built_scaffold_artifact() {
    if !cargo_component_available() || !wasm_target_available() {
        eprintln!(
            "skipping built scaffold doctor smoke test: cargo-component or wasm32-wasip2 missing"
        );
        return;
    }
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("artifact-component");
    let cargo_wrapper_dir = install_cargo_wrapper(temp.path());
    let path_env = format!(
        "{}:{}",
        cargo_wrapper_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut new_cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    new_cmd
        .arg("new")
        .arg("--name")
        .arg("artifact-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--no-check")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "local")
        .env("GIT_AUTHOR_NAME", "Greentic Labs")
        .env("GIT_AUTHOR_EMAIL", "greentic-labs@example.com")
        .env("GIT_COMMITTER_NAME", "Greentic Labs")
        .env("GIT_COMMITTER_EMAIL", "greentic-labs@example.com")
        .env("PATH", &path_env)
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("USER")
        .env_remove("USERNAME");
    new_cmd.assert().success();

    let mut build_cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    build_cmd
        .current_dir(&component_dir)
        .env("PATH", &path_env)
        .env("CARGO_NET_OFFLINE", "true")
        .env("GREENTIC_SKIP_NODE_EXPORT_CHECK", "1")
        .arg("build")
        .arg("--no-flow")
        .arg("--no-infer-config")
        .arg("--no-validate");
    build_cmd.assert().success();

    let manifest_path = component_dir.join("component.manifest.json");
    let manifest = fs::read_to_string(&manifest_path).expect("read built manifest");
    let manifest_json: JsonValue =
        serde_json::from_str(&manifest).expect("manifest parses as JSON after build");
    assert_eq!(
        manifest_json["world"]
            .as_str()
            .expect("manifest world should be a string"),
        canonical_component_world()
    );

    let wasm_path = component_dir.join(
        manifest_json["artifacts"]["component_wasm"]
            .as_str()
            .expect("artifact path"),
    );
    let wasm_uri = format!("file://{}", wasm_path.display());
    let mut doctor = Command::new(assert_cmd::cargo::cargo_bin!("component-doctor"));
    doctor
        .current_dir(&component_dir)
        .arg(wasm_uri)
        .arg("--manifest")
        .arg("component.manifest.json")
        .env("PATH", &path_env)
        .env("CARGO_NET_OFFLINE", "true");
    doctor.assert().success();
}

#[test]
fn scaffold_new_accepts_custom_user_operations() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("custom-ops-component");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    cmd.arg("new")
        .arg("--name")
        .arg("custom-ops-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--operation")
        .arg("render,sync-state")
        .arg("--default-operation")
        .arg("sync-state")
        .arg("--no-check")
        .arg("--no-git")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio");
    cmd.assert().success();

    let manifest = fs::read_to_string(component_dir.join("component.manifest.json"))
        .expect("custom ops manifest");
    let manifest_json: JsonValue = serde_json::from_str(&manifest).expect("manifest json");
    let operations = manifest_json["operations"]
        .as_array()
        .expect("operations array in scaffold");
    let user_operation_names = operations
        .iter()
        .filter_map(|op| op["name"].as_str())
        .filter(|name| !matches!(*name, "qa-spec" | "apply-answers" | "i18n-keys"))
        .collect::<Vec<_>>();
    assert_eq!(user_operation_names, vec!["render", "sync-state"]);
    assert_eq!(
        manifest_json["default_operation"].as_str(),
        Some("sync-state")
    );

    let lib_rs = fs::read_to_string(component_dir.join("src/lib.rs")).expect("lib.rs");
    assert!(lib_rs.contains("name: \"render\".to_string()"));
    assert!(lib_rs.contains("name: \"sync-state\".to_string()"));
}

#[test]
fn scaffold_new_writes_runtime_capability_fields() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("runtime-capability-component");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    cmd.arg("new")
        .arg("--name")
        .arg("runtime-capability-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--filesystem-mode")
        .arg("read_only")
        .arg("--filesystem-mount")
        .arg("assets:assets:/assets")
        .arg("--http-client")
        .arg("--state-read")
        .arg("--telemetry-scope")
        .arg("pack")
        .arg("--telemetry-span-prefix")
        .arg("component.runtime")
        .arg("--telemetry-attribute")
        .arg("component=runtime")
        .arg("--secret-key")
        .arg("API_TOKEN")
        .arg("--secret-env")
        .arg("prod")
        .arg("--secret-tenant")
        .arg("acme")
        .arg("--secret-format")
        .arg("text")
        .arg("--no-check")
        .arg("--no-git")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio");
    cmd.assert().success();

    let manifest = fs::read_to_string(component_dir.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"mode\": \"read_only\""));
    assert!(manifest.contains("\"guest_path\": \"/assets\""));
    assert!(manifest.contains("\"client\": true"));
    assert!(manifest.contains("\"read\": true"));
    assert!(manifest.contains("\"scope\": \"pack\""));
    assert!(manifest.contains("\"span_prefix\": \"component.runtime\""));
    assert!(manifest.contains("\"component\": \"runtime\""));
    assert!(manifest.contains("\"key\": \"API_TOKEN\""));
}

#[test]
fn scaffold_new_ignores_filesystem_mounts_when_mode_is_none() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("no-fs-component");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    cmd.arg("new")
        .arg("--name")
        .arg("no-fs-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--filesystem-mode")
        .arg("none")
        .arg("--filesystem-mount")
        .arg("assets:assets:/assets")
        .arg("--no-check")
        .arg("--no-git")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio");
    cmd.assert().success();

    let manifest = fs::read_to_string(component_dir.join("component.manifest.json")).unwrap();
    let manifest_json: JsonValue = serde_json::from_str(&manifest).expect("manifest json");
    assert_eq!(
        manifest_json["capabilities"]["wasi"]["filesystem"]["mode"].as_str(),
        Some("none")
    );
    assert_eq!(
        manifest_json["capabilities"]["wasi"]["filesystem"]["mounts"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
}

#[test]
fn scaffold_new_writes_config_schema_fields() {
    let temp = TempDir::new().expect("temp dir");
    let component_dir = temp.path().join("config-component");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    cmd.arg("new")
        .arg("--name")
        .arg("config-component")
        .arg("--org")
        .arg("ai.greentic")
        .arg("--path")
        .arg(&component_dir)
        .arg("--config-field")
        .arg("enabled:bool:required")
        .arg("--config-field")
        .arg("api_key:string")
        .arg("--no-check")
        .arg("--no-git")
        .env("HOME", temp.path())
        .env("GREENTIC_TEMPLATE_YEAR", "2030")
        .env("GREENTIC_TEMPLATE_ROOT", temp.path().join("templates"))
        .env("GREENTIC_DEP_MODE", "cratesio");
    cmd.assert().success();

    let manifest = fs::read_to_string(component_dir.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"enabled\""));
    assert!(manifest.contains("\"boolean\""));
    assert!(manifest.contains("\"api_key\""));

    let lib_rs = fs::read_to_string(component_dir.join("src/lib.rs")).unwrap();
    assert!(lib_rs.contains("\"enabled\".to_string()"));
    assert!(lib_rs.contains("SchemaIr::Bool"));
    assert!(lib_rs.contains("\"api_key\".to_string()"));

    let schema_file =
        fs::read_to_string(component_dir.join("schemas/component.schema.json")).unwrap();
    assert!(schema_file.contains("\"enabled\""));
    assert!(schema_file.contains("\"api_key\""));
}
