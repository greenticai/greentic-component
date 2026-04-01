#![cfg(feature = "cli")]

use greentic_component::cmd::doctor::{DoctorArgs, DoctorFormat, run as doctor_run};
use greentic_component::cmd::wizard::{ExecutionMode, RunMode, WizardArgs, run as wizard_run};

#[test]
fn doctor_rejects_unbuilt_wizard_scaffold() {
    let temp = tempfile::TempDir::new().unwrap();
    let answers_path = temp.path().join("answers.json");
    let payload = serde_json::json!({
        "schema": "component-wizard-run/v1",
        "mode": "create",
        "fields": {
            "component_name": "component",
            "output_dir": temp.path().join("component"),
            "abi_version": "0.6.0"
        }
    });
    std::fs::write(
        &answers_path,
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();

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
    wizard_run(args).unwrap();

    let root = temp.path().join("component");
    let doctor_args = DoctorArgs {
        target: root.to_string_lossy().to_string(),
        manifest: None,
        format: DoctorFormat::Human,
    };
    let err = doctor_run(doctor_args).expect_err("doctor should require a wasm artifact");
    assert!(err.to_string().contains("unable to resolve wasm"));
}
