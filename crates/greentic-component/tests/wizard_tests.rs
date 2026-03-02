#![cfg(feature = "cli")]

use greentic_component::cmd::wizard::{
    ExecutionMode, RunMode, WizardArgs, WizardCliArgs, WizardSubcommand, run, run_cli,
};
use std::fs;

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
