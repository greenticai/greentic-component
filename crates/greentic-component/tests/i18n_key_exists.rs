#![cfg(feature = "cli")]

use std::collections::BTreeMap;

#[test]
fn wizard_i18n_keys_exist_in_root_en_catalog() {
    let root_en = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("i18n/en.json");
    let raw = std::fs::read_to_string(&root_en).expect("read root i18n/en.json");
    let catalog: BTreeMap<String, String> = serde_json::from_str(&raw).expect("parse en.json");

    let required = [
        "cli.wizard.result.answers_mode_mismatch",
        "cli.wizard.result.invalid_schema",
        "cli.wizard.result.dry_run",
        "cli.wizard.result.execute_ok",
        "cli.wizard.result.interactive_header",
        "cli.wizard.result.plan_header",
        "cli.wizard.result.plan_steps",
        "cli.wizard.result.validate_apply_conflict",
        "cli.wizard.result.answer_doc_invalid_shape",
        "cli.wizard.result.answer_schema_id_mismatch",
        "cli.wizard.result.answer_schema_version_mismatch",
        "cli.wizard.result.answer_mode_unsupported",
        "cli.wizard.prompt.component_name",
        "cli.wizard.prompt.output_dir",
        "cli.wizard.prompt.abi_version",
        "cli.wizard.prompt.project_root",
        "cli.wizard.prompt.full_tests",
        "cli.wizard.prompt.overwrite_dir",
        "cli.wizard.prompt.template_id",
        "cli.wizard.result.qa_value_required",
        "cli.wizard.result.qa_answer_yes_no",
        "cli.wizard.result.qa_invalid_choice",
        "cli.wizard.result.qa_select_number_or_value",
        "cli.wizard.result.qa_validation_error",
        "cli.wizard.prompt.plan_out",
        "cli.wizard.result.plan_out_required_non_interactive",
        "cli.wizard.result.plan_written",
        "cli.wizard.result.component_written",
        "cli.wizard.result.choose_another_output_dir",
        "cli.wizard.step.template_used",
    ];

    for key in required {
        assert!(catalog.contains_key(key), "missing i18n key {key}");
    }
}
