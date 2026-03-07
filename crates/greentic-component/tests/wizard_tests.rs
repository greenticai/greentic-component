#![cfg(feature = "cli")]

use assert_cmd::prelude::*;
use greentic_component::cmd::wizard::{
    ExecutionMode, RunMode, WizardArgs, WizardCliArgs, WizardSubcommand, run, run_cli,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;

fn create_answers(path: &std::path::Path, name: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0"
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_operations(path: &std::path::Path, name: &str, operations: &[&str]) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0",
            "operations": operations,
            "default_operation": operations.first().copied().unwrap_or("handle_message")
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_operation_names(path: &std::path::Path, name: &str, operation_names: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0",
            "operation_names": operation_names
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_runtime_capabilities(path: &std::path::Path, name: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0",
            "operation_names": "handle_message,render",
            "filesystem_mode": "read_only",
            "filesystem_mounts": "assets:assets:/assets",
            "messaging_inbound": true,
            "messaging_outbound": false,
            "events_inbound": false,
            "events_outbound": true,
            "http_client": true,
            "state_read": true,
            "telemetry_scope": "pack",
            "telemetry_span_prefix": "component.demo",
            "telemetry_attributes": "component=demo",
            "secret_keys": "API_TOKEN",
            "secret_env": "prod",
            "secret_tenant": "acme",
            "secret_format": "text"
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_no_filesystem_mounts(path: &std::path::Path, name: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0",
            "filesystem_mode": "none",
            "filesystem_mounts": "assets:assets:/assets"
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_config_fields(path: &std::path::Path, name: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": name,
            "output_dir": root.join(name),
            "abi_version": "0.6.0",
            "config_fields": "enabled:bool:required,api_key:string"
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_all_fields(path: &std::path::Path, name: &str) -> Value {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let output_dir = root.join(name);
    let fields = json!({
        "component_name": name,
        "output_dir": output_dir,
        "abi_version": "0.6.0",
        "operation_names": "handle_message,render,notify",
        "filesystem_mode": "sandbox",
        "filesystem_mounts": "assets:assets:/assets,data:data:/data",
        "http_client": true,
        "messaging_inbound": true,
        "messaging_outbound": true,
        "events_inbound": true,
        "events_outbound": false,
        "http_server": false,
        "state_read": true,
        "state_write": true,
        "state_delete": false,
        "telemetry_scope": "tenant",
        "telemetry_span_prefix": "wizard.smoke",
        "telemetry_attributes": "component=wizard-smoke,mode=advanced",
        "secrets_enabled": true,
        "secret_keys": "API_TOKEN,WEBHOOK_SECRET",
        "secret_env": "prod",
        "secret_tenant": "acme",
        "secret_format": "text",
        "config_fields": "enabled:bool:required,api_key:string,timeout_ms:integer"
    });
    let payload = json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": fields
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
    payload
}

fn install_cargo_wrapper(root: &Path) -> std::path::PathBuf {
    let bin_dir = root.join("test-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let wrapper_path = bin_dir.join("cargo");
    let real_cargo = std::process::Command::new("bash")
        .arg("-lc")
        .arg("command -v cargo")
        .output()
        .expect("locate cargo");
    assert!(real_cargo.status.success(), "cargo should be available");
    let real_cargo = String::from_utf8(real_cargo.stdout)
        .unwrap()
        .trim()
        .to_string();
    let real_component = std::process::Command::new("bash")
        .arg("-lc")
        .arg("command -v cargo-component")
        .output()
        .expect("locate cargo-component");
    assert!(
        real_component.status.success(),
        "cargo-component should be available for wizard smoke test"
    );
    let real_component = String::from_utf8(real_component.stdout)
        .unwrap()
        .trim()
        .to_string();
    let script = format!(
        "#!/bin/sh\nset -eu\nif [ \"${{1:-}}\" = \"component\" ]; then\n  shift\n  exec \"{real_component}\" \"$@\"\nfi\nexec \"{real_cargo}\" \"$@\"\n"
    );
    fs::write(&wrapper_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper_path, perms).unwrap();
    }
    bin_dir
}

fn create_add_operation_answers(
    path: &std::path::Path,
    project_root: &std::path::Path,
    operation: &str,
) {
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "add-operation",
        "fields": {
            "project_root": project_root,
            "operation_name": operation,
            "set_default_operation": true
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_update_operation_answers(
    path: &std::path::Path,
    project_root: &std::path::Path,
    operation: &str,
    new_operation: &str,
) {
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "update-operation",
        "fields": {
            "project_root": project_root,
            "operation_name": operation,
            "new_operation_name": new_operation,
            "set_default_operation": true
        }
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answers_with_mode(path: &std::path::Path, mode: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let fields = match mode {
        "build-test" => serde_json::json!({
            "project_root": root,
            "full_tests": false
        }),
        "doctor" => serde_json::json!({
            "project_root": root
        }),
        _ => serde_json::json!({
            "component_name": "component",
            "output_dir": root.join("component"),
            "abi_version": "0.6.0"
        }),
    };
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": mode,
        "fields": fields
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

fn create_answer_document(path: &std::path::Path, name: &str, schema_version: &str) {
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let payload = serde_json::json!({
        "wizard_id": "greentic-component.wizard.run",
        "schema_id": "greentic-component.wizard.run",
        "schema_version": schema_version,
        "locale": "en",
        "answers": {
            "mode": "create",
            "fields": {
                "component_name": name,
                "output_dir": root.join(name),
                "abi_version": "0.6.0"
            }
        },
        "locks": {}
    });
    fs::write(path, serde_json::to_string_pretty(&payload).unwrap()).unwrap();
}

#[test]
fn wizard_create_execute_creates_template_files() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answers(&answers_path, "demo-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should succeed");

    let root = temp.path().join("demo-component");
    assert!(root.join("Cargo.toml").exists());
    assert!(root.join("build.rs").exists());
    assert!(root.join("src/lib.rs").exists());
    assert!(root.join("Makefile").exists());
    assert!(root.join("src/qa.rs").exists());
    assert!(root.join("src/i18n.rs").exists());
    assert!(root.join("src/i18n_bundle.rs").exists());
    assert!(root.join("assets/i18n/en.json").exists());
    assert!(root.join("assets/i18n/locales.json").exists());
    assert!(root.join("tools/i18n.sh").exists());

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("name = \"demo-component\""));
    assert!(cargo_toml.contains("[package.metadata.greentic]"));
    assert!(cargo_toml.contains("abi_version = \"0.6.0\""));
}

#[test]
fn wizard_create_supports_multiple_user_operations() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answers_with_operations(
        &answers_path,
        "multi-op-component",
        &["render", "summarize"],
    );

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should support authored operations");

    let root = temp.path().join("multi-op-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"name\": \"render\""));
    assert!(manifest.contains("\"name\": \"summarize\""));
    assert!(manifest.contains("\"default_operation\": \"render\""));

    let lib_rs = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(lib_rs.contains("name: \"render\".to_string()"));
    assert!(lib_rs.contains("name: \"summarize\".to_string()"));
}

#[test]
fn wizard_create_supports_comma_separated_operation_names() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.csv.json");
    create_answers_with_operation_names(&answers_path, "csv-op-component", "render, summarize");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should parse comma-separated operation names");

    let root = temp.path().join("csv-op-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"name\": \"render\""));
    assert!(manifest.contains("\"name\": \"summarize\""));
    assert!(manifest.contains("\"default_operation\": \"render\""));
}

#[test]
fn wizard_create_writes_runtime_capability_fields() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.runtime.json");
    create_answers_with_runtime_capabilities(&answers_path, "capability-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create with runtime capability fields should succeed");

    let root = temp.path().join("capability-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"mode\": \"read_only\""));
    assert!(manifest.contains("\"guest_path\": \"/assets\""));
    assert!(manifest.contains("\"messaging\""));
    assert!(manifest.contains("\"events\""));
    assert!(manifest.contains("\"inbound\": true"));
    assert!(manifest.contains("\"outbound\": true"));
    assert!(manifest.contains("\"client\": true"));
    assert!(manifest.contains("\"read\": true"));
    assert!(manifest.contains("\"scope\": \"pack\""));
    assert!(manifest.contains("\"span_prefix\": \"component.demo\""));
    assert!(manifest.contains("\"key\": \"API_TOKEN\""));
}

#[test]
fn wizard_create_writes_concrete_qa_operation_schemas() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.qa-schemas.json");
    create_answers(&answers_path, "qa-schema-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should succeed");

    let root = temp.path().join("qa-schema-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let operations = manifest_json["operations"].as_array().unwrap();
    let qa_spec = operations
        .iter()
        .find(|op| op["name"] == "qa-spec")
        .expect("qa-spec operation");
    let apply_answers = operations
        .iter()
        .find(|op| op["name"] == "apply-answers")
        .expect("apply-answers operation");
    let i18n_keys = operations
        .iter()
        .find(|op| op["name"] == "i18n-keys")
        .expect("i18n-keys operation");

    assert_eq!(
        qa_spec["output_schema"]["required"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        qa_spec["output_schema"]["properties"]["mode"]["type"].as_str(),
        Some("string")
    );
    assert_eq!(
        apply_answers["output_schema"]["properties"]["warnings"]["items"]["type"].as_str(),
        Some("string")
    );
    assert_eq!(
        i18n_keys["output_schema"]["items"]["type"].as_str(),
        Some("string")
    );
}

#[test]
fn wizard_create_ignores_filesystem_mounts_when_mode_is_none() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.no-fs.json");
    create_answers_with_no_filesystem_mounts(&answers_path, "no-fs-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should succeed");

    let root = temp.path().join("no-fs-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
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
fn wizard_create_writes_config_schema_fields() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.config.json");
    create_answers_with_config_fields(&answers_path, "config-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard create should succeed");

    let root = temp.path().join("config-component");
    let manifest = fs::read_to_string(root.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"enabled\""));
    assert!(manifest.contains("\"boolean\""));
    assert!(manifest.contains("\"api_key\""));

    let lib_rs = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(lib_rs.contains("\"enabled\".to_string()"));
    assert!(lib_rs.contains("SchemaIr::Bool"));
    assert!(lib_rs.contains("\"api_key\".to_string()"));

    let schema_file = fs::read_to_string(root.join("schemas/component.schema.json")).unwrap();
    assert!(schema_file.contains("\"enabled\""));
    assert!(schema_file.contains("\"api_key\""));
}

#[test]
fn wizard_create_writes_answers_out_when_requested() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    let answers_out = temp.path().join("out/answers.out.json");
    create_answers(&answers_path, "answers-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: Some(answers_out.clone()),
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("out/plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard dry-run should succeed");
    assert!(answers_out.exists());
}

#[test]
fn wizard_create_dry_run_does_not_write_files() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answers(&answers_path, "component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard dry-run should succeed");
    let root = temp.path().join("component");
    assert!(
        !root.exists(),
        "dry-run mode should not execute file writes"
    );
}

#[test]
fn wizard_validate_flag_behaves_like_dry_run() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answers(&answers_path, "component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: true,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard validate should succeed");
    let root = temp.path().join("component");
    assert!(
        !root.exists(),
        "validate mode should not execute file writes"
    );
}

#[test]
fn wizard_validate_command_alias_behaves_like_dry_run() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answers(&answers_path, "component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run_cli(WizardCliArgs {
        command: Some(WizardSubcommand::Validate(args)),
        args: WizardArgs {
            mode: RunMode::Create,
            execution: ExecutionMode::Execute,
            dry_run: false,
            validate: false,
            apply: false,
            qa_answers: None,
            answers: None,
            qa_answers_out: None,
            emit_answers: None,
            schema_version: None,
            migrate: false,
            plan_out: None,
            project_root: temp.path().to_path_buf(),
            template: None,
            full_tests: false,
            json: false,
        },
    })
    .expect("wizard validate alias should succeed");
    let root = temp.path().join("component");
    assert!(
        !root.exists(),
        "validate alias mode should not execute file writes"
    );
}

#[test]
fn wizard_answers_aliases_work() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    let answers_out = temp.path().join("out/answers.out.json");
    create_answers(&answers_path, "answers-alias-component");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: Some(answers_out.clone()),
        schema_version: Some("1.0.1".to_string()),
        migrate: true,
        plan_out: Some(temp.path().join("out/plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard dry-run with alias flags should succeed");
    assert!(answers_out.exists());
    let out = fs::read_to_string(answers_out).unwrap();
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        out.get("wizard_id").and_then(serde_json::Value::as_str),
        Some("greentic-component.wizard.run")
    );
    assert_eq!(
        out.get("schema_version")
            .and_then(serde_json::Value::as_str),
        Some("1.0.1")
    );
}

#[test]
fn wizard_answer_document_requires_migrate_for_schema_version_change() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answer_document(&answers_path, "doc-component", "0.9.0");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    let err = run(args).expect_err("expected schema version mismatch without --migrate");
    assert!(
        err.to_string().contains("rerun with --migrate"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn wizard_answer_document_migrates_with_flag() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    let answers_out = temp.path().join("answers.out.json");
    create_answer_document(&answers_path, "doc-component", "0.9.0");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: Some(answers_out.clone()),
        schema_version: None,
        migrate: true,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard should migrate and continue");
    let out = fs::read_to_string(answers_out).unwrap();
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        out.get("schema_version")
            .and_then(serde_json::Value::as_str),
        Some("1.0.0")
    );
}

#[test]
fn wizard_apply_command_alias_with_migrate_executes_side_effects() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    create_answer_document(&answers_path, "apply-doc-component", "0.9.0");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: true,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run_cli(WizardCliArgs {
        command: Some(WizardSubcommand::Apply(args)),
        args: WizardArgs {
            mode: RunMode::Create,
            execution: ExecutionMode::DryRun,
            dry_run: false,
            validate: false,
            apply: false,
            qa_answers: None,
            answers: None,
            qa_answers_out: None,
            emit_answers: None,
            schema_version: None,
            migrate: false,
            plan_out: None,
            project_root: temp.path().to_path_buf(),
            template: None,
            full_tests: false,
            json: false,
        },
    })
    .expect("wizard apply alias should execute scaffold");
    assert!(temp.path().join("apply-doc-component/Cargo.toml").exists());
}

#[test]
fn wizard_replay_answers_mode_build_test_overrides_default_create_mode() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.build-test.json");
    let plan_out = temp.path().join("plan.json");
    create_answers_with_mode(&answers_path, "build-test");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(plan_out.clone()),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard replay should adopt build-test mode from answers");
    let plan: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(plan_out).expect("plan should exist"))
            .expect("plan JSON");
    assert_eq!(
        plan.pointer("/plan/meta/id")
            .and_then(serde_json::Value::as_str),
        Some("greentic.component.build_test")
    );
}

#[test]
fn wizard_replay_answers_mode_doctor_overrides_default_create_mode() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.doctor.json");
    let plan_out = temp.path().join("plan.json");
    create_answers_with_mode(&answers_path, "doctor");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(plan_out.clone()),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard replay should adopt doctor mode from answers");
    let plan: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(plan_out).expect("plan should exist"))
            .expect("plan JSON");
    assert_eq!(
        plan.pointer("/plan/meta/id")
            .and_then(serde_json::Value::as_str),
        Some("greentic.component.doctor")
    );
}

#[test]
fn wizard_emit_answers_preserves_replayed_mode() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.build-test.json");
    let answers_out = temp.path().join("answers.out.json");
    create_answers_with_mode(&answers_path, "build-test");

    let args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::DryRun,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_path),
        qa_answers_out: None,
        emit_answers: Some(answers_out.clone()),
        schema_version: None,
        migrate: false,
        plan_out: Some(temp.path().join("plan.json")),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(args).expect("wizard replay should emit answers");
    let out: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(answers_out).expect("answers out")).unwrap();
    assert_eq!(
        out.pointer("/answers/mode")
            .and_then(serde_json::Value::as_str),
        Some("build-test")
    );
}

#[test]
fn wizard_add_operation_updates_manifest_and_lib() {
    let temp = tempfile::TempDir::new().unwrap();
    let create_answers_path = temp.path().join("create.answers.json");
    create_answers(&create_answers_path, "op-edit-component");

    run(WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(create_answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    })
    .unwrap();

    let project_root = temp.path().join("op-edit-component");
    let add_answers = temp.path().join("add.answers.json");
    create_add_operation_answers(&add_answers, &project_root, "render");

    run(WizardArgs {
        mode: RunMode::AddOperation,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(add_answers),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    })
    .unwrap();

    let manifest = fs::read_to_string(project_root.join("component.manifest.json")).unwrap();
    assert!(manifest.contains("\"name\": \"render\""));
    assert!(manifest.contains("\"default_operation\": \"render\""));

    let lib_rs = fs::read_to_string(project_root.join("src/lib.rs")).unwrap();
    assert!(lib_rs.contains("name: \"render\".to_string()"));
    assert!(lib_rs.contains("name: \"qa-spec\".to_string()"));
}

#[test]
fn wizard_update_operation_renames_manifest_and_lib() {
    let temp = tempfile::TempDir::new().unwrap();
    let create_answers_path = temp.path().join("create.answers.json");
    create_answers(&create_answers_path, "rename-op-component");

    run(WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: Some(create_answers_path),
        answers: None,
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    })
    .unwrap();

    let project_root = temp.path().join("rename-op-component");
    let update_answers = temp.path().join("update.answers.json");
    create_update_operation_answers(&update_answers, &project_root, "handle_message", "render");

    run(WizardArgs {
        mode: RunMode::UpdateOperation,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(update_answers),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    })
    .unwrap();

    let manifest = fs::read_to_string(project_root.join("component.manifest.json")).unwrap();
    assert!(!manifest.contains("\"name\": \"handle_message\""));
    assert!(manifest.contains("\"name\": \"render\""));
    assert!(manifest.contains("\"default_operation\": \"render\""));

    let lib_rs = fs::read_to_string(project_root.join("src/lib.rs")).unwrap();
    assert!(!lib_rs.contains("name: \"handle_message\".to_string()"));
    assert!(lib_rs.contains("name: \"render\".to_string()"));
}

#[test]
fn wizard_full_chain_dry_run_emit_validate_replay_execute() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_in = temp.path().join("answers.in.json");
    let plan_out = temp.path().join("plan.validate.json");
    let answers_out = temp.path().join("answers.out.json");
    let replay_plan = temp.path().join("plan.replay.json");
    let component_name = "full-chain-component";
    let component_root = temp.path().join(component_name);
    create_answers(&answers_in, component_name);

    let validate_args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: true,
        apply: false,
        qa_answers: None,
        answers: Some(answers_in),
        qa_answers_out: None,
        emit_answers: Some(answers_out.clone()),
        schema_version: None,
        migrate: false,
        plan_out: Some(plan_out.clone()),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };

    run(validate_args).expect("validate pass should succeed");
    assert!(plan_out.exists(), "validate should emit a plan file");
    assert!(
        answers_out.exists(),
        "validate should emit an answers document"
    );
    assert!(
        !component_root.exists(),
        "validate/dry-run path should not create scaffold files"
    );

    let emitted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&answers_out).expect("answers out")).unwrap();
    assert_eq!(
        emitted
            .pointer("/schema_id")
            .and_then(serde_json::Value::as_str),
        Some("greentic-component.wizard.run")
    );
    assert_eq!(
        emitted
            .pointer("/answers/mode")
            .and_then(serde_json::Value::as_str),
        Some("create")
    );
    assert_eq!(
        emitted
            .pointer("/answers/fields/component_name")
            .and_then(serde_json::Value::as_str),
        Some(component_name)
    );

    let replay_validate_args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: true,
        apply: false,
        qa_answers: None,
        answers: Some(answers_out.clone()),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: Some(replay_plan.clone()),
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };
    run(replay_validate_args).expect("replay validate should succeed");
    assert!(
        replay_plan.exists(),
        "replay validate should emit a second plan"
    );

    let execute_args = WizardArgs {
        mode: RunMode::Create,
        execution: ExecutionMode::Execute,
        dry_run: false,
        validate: false,
        apply: false,
        qa_answers: None,
        answers: Some(answers_out),
        qa_answers_out: None,
        emit_answers: None,
        schema_version: None,
        migrate: false,
        plan_out: None,
        project_root: temp.path().to_path_buf(),
        template: None,
        full_tests: false,
        json: false,
    };
    run(execute_args).expect("execute from emitted answers should succeed");

    assert!(component_root.join("Cargo.toml").exists());
    assert!(component_root.join("component.manifest.json").exists());
    assert!(component_root.join("src/lib.rs").exists());
    let cargo_toml = fs::read_to_string(component_root.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("name = \"full-chain-component\""));
    assert!(cargo_toml.contains("abi_version = \"0.6.0\""));
}

#[test]
fn wizard_emit_answers_round_trips_all_fields_and_replay_builds_component() {
    let temp = tempfile::TempDir::new().unwrap();
    let component_name = "wizard-smoke-advanced";
    let answers_in = temp.path().join("answers.in.json");
    let answers_out = temp.path().join("answers.out.json");
    let plan_out = temp.path().join("plan.json");
    let component_root = temp.path().join(component_name);
    let cargo_wrapper_dir = install_cargo_wrapper(temp.path());
    let path_env = format!(
        "{}:{}",
        cargo_wrapper_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let expected_payload = create_answers_with_all_fields(&answers_in, component_name);
    let expected_fields = expected_payload
        .get("fields")
        .cloned()
        .expect("expected fields");

    let mut dry_run = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    dry_run
        .arg("wizard")
        .arg("--mode")
        .arg("create")
        .arg("--dry-run")
        .arg("--qa-answers")
        .arg(&answers_in)
        .arg("--emit-answers")
        .arg(&answers_out)
        .arg("--plan-out")
        .arg(&plan_out)
        .env("HOME", temp.path())
        .env("GREENTIC_DEP_MODE", "local")
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", &path_env);
    dry_run.assert().success();

    let emitted: Value = serde_json::from_str(&fs::read_to_string(&answers_out).unwrap()).unwrap();
    assert_eq!(
        emitted.get("wizard_id").and_then(Value::as_str),
        Some("greentic-component.wizard.run")
    );
    assert_eq!(
        emitted.get("schema_id").and_then(Value::as_str),
        Some("greentic-component.wizard.run")
    );
    assert_eq!(
        emitted.get("schema_version").and_then(Value::as_str),
        Some("1.0.0")
    );
    assert_eq!(
        emitted.pointer("/answers/mode").and_then(Value::as_str),
        Some("create")
    );
    assert_eq!(emitted.pointer("/answers/fields"), Some(&expected_fields));
    assert!(plan_out.exists(), "dry-run should emit a plan file");
    assert!(
        !component_root.exists(),
        "dry-run should not create component files"
    );

    let mut replay = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    replay
        .arg("wizard")
        .arg("--mode")
        .arg("create")
        .arg("--answers")
        .arg(&answers_out)
        .env("HOME", temp.path())
        .env("GREENTIC_DEP_MODE", "local")
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", &path_env);
    replay.assert().success();

    assert!(component_root.join("Cargo.toml").exists());
    assert!(component_root.join("component.manifest.json").exists());

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(component_root.join("component.manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest.get("default_operation").and_then(Value::as_str),
        Some("handle_message")
    );
    assert_eq!(
        manifest
            .pointer("/capabilities/wasi/filesystem/mode")
            .and_then(Value::as_str),
        Some("sandbox")
    );
    assert_eq!(
        manifest
            .pointer("/capabilities/host/messaging/inbound")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        manifest
            .pointer("/capabilities/host/messaging/outbound")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        manifest
            .pointer("/capabilities/host/events/inbound")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        manifest
            .pointer("/capabilities/host/events/outbound")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        manifest
            .pointer("/telemetry/span_prefix")
            .and_then(Value::as_str),
        Some("wizard.smoke")
    );
    assert_eq!(
        manifest
            .pointer("/config_schema/required/0")
            .and_then(Value::as_str),
        Some("enabled")
    );

    let mut cargo_test = Command::new("cargo");
    cargo_test
        .arg("test")
        .arg("--manifest-path")
        .arg(component_root.join("Cargo.toml"))
        .arg("--offline")
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", &path_env);
    cargo_test.assert().success();

    let mut wasm_build = Command::new("make");
    wasm_build
        .current_dir(&component_root)
        .arg("wasm")
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", &path_env);
    wasm_build.assert().success();

    let wasm_path = component_root.join("dist/wizard-smoke-advanced__0_6_0.wasm");
    assert!(
        wasm_path.exists(),
        "wasm build should produce a dist artifact"
    );

    let mut doctor = Command::new(assert_cmd::cargo::cargo_bin!("greentic-component"));
    doctor
        .arg("doctor")
        .arg(&wasm_path)
        .arg("--manifest")
        .arg(component_root.join("component.manifest.json"))
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", &path_env);
    doctor.assert().success();
}
