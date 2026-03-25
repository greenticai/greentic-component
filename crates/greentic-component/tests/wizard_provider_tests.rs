#![cfg(feature = "cli")]

use std::path::PathBuf;

use greentic_component::scaffold::config_schema::ConfigSchemaInput;
use greentic_component::scaffold::runtime_capabilities::RuntimeCapabilitiesInput;
use greentic_component::wizard::{WizardRequest, WizardStep, apply_scaffold, execute_plan};
use insta::assert_json_snapshot;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct PlanSnapshot<'a> {
    plan_version: u32,
    generator: &'a str,
    template_version: &'a str,
    template_digest_blake3: &'a str,
    requested_abi_version: &'a str,
    step_count: usize,
    steps: Vec<StepSnapshot>,
}

#[derive(Debug, Serialize)]
struct StepSnapshot {
    kind: &'static str,
    path: String,
    size: Option<usize>,
    blake3: Option<String>,
}

#[test]
fn scaffold_plan_snapshot_is_deterministic() {
    let request = WizardRequest {
        name: "demo-component".to_string(),
        abi_version: "0.6.0".to_string(),
        mode: greentic_component::wizard::WizardMode::Default,
        target: PathBuf::from("/tmp/wizard-provider-plan/demo-component"),
        answers: None,
        required_capabilities: vec!["host.http.client".to_string()],
        provided_capabilities: vec!["telemetry.emit".to_string()],
        user_operations: vec!["handle_message".to_string()],
        default_operation: Some("handle_message".to_string()),
        runtime_capabilities: RuntimeCapabilitiesInput::default(),
        config_schema: ConfigSchemaInput::default(),
    };

    let result = apply_scaffold(request, true).expect("plan should build");
    let envelope = &result.plan;
    let plan = &envelope.plan;

    let steps = plan
        .steps
        .iter()
        .map(|step| match step {
            WizardStep::EnsureDir { paths } => StepSnapshot {
                kind: "ensure_dir",
                path: paths.join(","),
                size: None,
                blake3: None,
            },
            WizardStep::WriteFiles { files } => {
                let joined = files.keys().cloned().collect::<Vec<_>>().join(",");
                let mut hasher = blake3::Hasher::new();
                for (path, content) in files {
                    hasher.update(path.as_bytes());
                    hasher.update(&[0]);
                    hasher.update(content.as_bytes());
                    hasher.update(&[0xff]);
                }
                let total_size = files.values().map(|value| value.len()).sum::<usize>();
                StepSnapshot {
                    kind: "write_files",
                    path: joined,
                    size: Some(total_size),
                    blake3: Some(hasher.finalize().to_hex().to_string()),
                }
            }
            WizardStep::RunCli { command, .. } => StepSnapshot {
                kind: "run_cli",
                path: command.clone(),
                size: None,
                blake3: None,
            },
            WizardStep::Delegate { id, .. } => StepSnapshot {
                kind: "delegate",
                path: id.as_str().to_string(),
                size: None,
                blake3: None,
            },
            WizardStep::BuildComponent { project_root } => StepSnapshot {
                kind: "build_component",
                path: project_root.clone(),
                size: None,
                blake3: None,
            },
            WizardStep::TestComponent { project_root, full } => StepSnapshot {
                kind: "test_component",
                path: format!("{project_root}:{full}"),
                size: None,
                blake3: None,
            },
            WizardStep::Doctor { project_root } => StepSnapshot {
                kind: "doctor",
                path: project_root.clone(),
                size: None,
                blake3: None,
            },
        })
        .collect::<Vec<_>>();

    let snap = PlanSnapshot {
        plan_version: envelope.plan_version,
        generator: &envelope.metadata.generator,
        template_version: &envelope.metadata.template_version,
        template_digest_blake3: &envelope.metadata.template_digest_blake3,
        requested_abi_version: &envelope.metadata.requested_abi_version,
        step_count: plan.steps.len(),
        steps,
    };

    assert_json_snapshot!("scaffold_plan_snapshot", snap);
}

#[test]
fn execute_plan_writes_expected_files() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let target = temp.path().join("exec-demo");
    let request = WizardRequest {
        name: "exec-demo".to_string(),
        abi_version: "0.6.0".to_string(),
        mode: greentic_component::wizard::WizardMode::Default,
        target: target.clone(),
        answers: None,
        required_capabilities: Vec::new(),
        provided_capabilities: Vec::new(),
        user_operations: vec!["handle_message".to_string()],
        default_operation: Some("handle_message".to_string()),
        runtime_capabilities: RuntimeCapabilitiesInput::default(),
        config_schema: ConfigSchemaInput::default(),
    };

    let result = apply_scaffold(request, true).expect("plan should build");
    execute_plan(&result.plan).expect("plan execution should succeed");

    assert!(target.join("Cargo.toml").exists());
    assert!(target.join("src/lib.rs").exists());
    assert!(target.join("src/qa.rs").exists());
    assert!(target.join("assets/i18n/en.json").exists());
    assert!(target.join("assets/i18n/locales.json").exists());
    assert!(target.join("tools/i18n.sh").exists());
    let i18n_tool =
        std::fs::read_to_string(target.join("tools/i18n.sh")).expect("read tools/i18n.sh");
    assert!(
        i18n_tool.contains("--skip-git-repo-check"),
        "wizard i18n script should wrap codex exec with --skip-git-repo-check"
    );
    assert!(
        i18n_tool.contains("greentic-i18n-translator"),
        "wizard i18n script should call greentic-i18n-translator directly"
    );
    assert!(
        i18n_tool.contains("cargo binstall -y greentic-i18n-translator"),
        "wizard i18n script should install greentic-i18n-translator with cargo-binstall when missing"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(target.join("tools/i18n.sh"))
            .expect("i18n.sh metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "tools/i18n.sh should be executable");
    }

    let cargo = std::fs::read_to_string(target.join("Cargo.toml")).expect("cargo.toml");
    assert!(cargo.contains("name = \"exec-demo\""));
}

#[test]
fn spec_uses_namespaced_question_ids() {
    let spec =
        greentic_component::wizard::spec_scaffold(greentic_component::wizard::WizardMode::Default);
    let ids = spec.questions.into_iter().map(|q| q.id).collect::<Vec<_>>();
    assert!(ids.contains(&"component.name".to_string()));
    assert!(ids.contains(&"component.path".to_string()));
    assert!(ids.contains(&"component.kind".to_string()));
    assert!(ids.contains(&"component.features.enabled".to_string()));
}
