#![cfg(feature = "cli")]

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand, ValueEnum};
use greentic_qa_lib::{
    I18nConfig, QaLibError, ResolvedI18nMap, WizardDriver, WizardFrontend, WizardRunConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::cmd::build::BuildArgs;
use crate::cmd::doctor::{DoctorArgs, DoctorFormat};
use crate::cmd::i18n;
use crate::scaffold::validate::{ComponentName, normalize_version};
use crate::wizard::{self, AnswersPayload, WizardPlanEnvelope, WizardPlanMetadata, WizardStep};

const WIZARD_RUN_SCHEMA: &str = "component-wizard-run/v1";
const ANSWER_DOC_WIZARD_ID: &str = "greentic-component.wizard.run";
const ANSWER_DOC_SCHEMA_ID: &str = "greentic-component.wizard.run";
const ANSWER_DOC_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Args, Debug, Clone)]
pub struct WizardCliArgs {
    #[command(subcommand)]
    pub command: Option<WizardSubcommand>,
    #[command(flatten)]
    pub args: WizardArgs,
}

#[derive(Subcommand, Debug, Clone)]
pub enum WizardSubcommand {
    Run(WizardArgs),
    Validate(WizardArgs),
    Apply(WizardArgs),
    #[command(hide = true)]
    New(WizardLegacyNewArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WizardLegacyNewArgs {
    #[arg(value_name = "LEGACY_NAME")]
    pub name: Option<String>,
    #[arg(long = "out", value_name = "PATH")]
    pub out: Option<PathBuf>,
    #[command(flatten)]
    pub args: WizardArgs,
}

#[derive(Args, Debug, Clone)]
pub struct WizardArgs {
    #[arg(long, value_enum, default_value = "create")]
    pub mode: RunMode,
    #[arg(long, value_enum, default_value = "execute")]
    pub execution: ExecutionMode,
    #[arg(
        long = "dry-run",
        default_value_t = false,
        conflicts_with = "execution"
    )]
    pub dry_run: bool,
    #[arg(
        long = "validate",
        default_value_t = false,
        conflicts_with_all = ["execution", "dry_run", "apply"]
    )]
    pub validate: bool,
    #[arg(
        long = "apply",
        default_value_t = false,
        conflicts_with_all = ["execution", "dry_run", "validate"]
    )]
    pub apply: bool,
    #[arg(long = "qa-answers", value_name = "answers.json")]
    pub qa_answers: Option<PathBuf>,
    #[arg(
        long = "answers",
        value_name = "answers.json",
        conflicts_with = "qa_answers"
    )]
    pub answers: Option<PathBuf>,
    #[arg(long = "qa-answers-out", value_name = "answers.json")]
    pub qa_answers_out: Option<PathBuf>,
    #[arg(
        long = "emit-answers",
        value_name = "answers.json",
        conflicts_with = "qa_answers_out"
    )]
    pub emit_answers: Option<PathBuf>,
    #[arg(long = "schema-version", value_name = "VER")]
    pub schema_version: Option<String>,
    #[arg(long = "migrate", default_value_t = false)]
    pub migrate: bool,
    #[arg(long = "plan-out", value_name = "plan.json")]
    pub plan_out: Option<PathBuf>,
    #[arg(long = "project-root", value_name = "PATH", default_value = ".")]
    pub project_root: PathBuf,
    #[arg(long = "template", value_name = "TEMPLATE_ID")]
    pub template: Option<String>,
    #[arg(long = "full-tests")]
    pub full_tests: bool,
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Create,
    #[value(alias = "build_test")]
    BuildTest,
    Doctor,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[value(alias = "dry_run")]
    DryRun,
    Execute,
}

#[derive(Debug, Clone)]
struct WizardLegacyNewCompat {
    name: Option<String>,
    out: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WizardRunAnswers {
    schema: String,
    mode: RunMode,
    #[serde(default)]
    fields: JsonMap<String, JsonValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnswerDocument {
    wizard_id: String,
    schema_id: String,
    schema_version: String,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    answers: JsonMap<String, JsonValue>,
    #[serde(default)]
    locks: JsonMap<String, JsonValue>,
}

#[derive(Debug, Clone)]
struct LoadedRunAnswers {
    run_answers: WizardRunAnswers,
    source_document: Option<AnswerDocument>,
}

#[derive(Debug, Serialize)]
struct WizardRunOutput {
    mode: RunMode,
    execution: ExecutionMode,
    plan: WizardPlanEnvelope,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

pub fn run_cli(cli: WizardCliArgs) -> Result<()> {
    let mut execution_override = None;
    let mut legacy_new = None;
    let args = match cli.command {
        Some(WizardSubcommand::Run(args)) => args,
        Some(WizardSubcommand::Validate(args)) => {
            execution_override = Some(ExecutionMode::DryRun);
            args
        }
        Some(WizardSubcommand::Apply(args)) => {
            execution_override = Some(ExecutionMode::Execute);
            args
        }
        Some(WizardSubcommand::New(new_args)) => {
            legacy_new = Some(WizardLegacyNewCompat {
                name: new_args.name,
                out: new_args.out,
            });
            new_args.args
        }
        None => cli.args,
    };
    run_with_context(args, execution_override, legacy_new)
}

pub fn run(args: WizardArgs) -> Result<()> {
    run_with_context(args, None, None)
}

fn run_with_context(
    args: WizardArgs,
    execution_override: Option<ExecutionMode>,
    legacy_new: Option<WizardLegacyNewCompat>,
) -> Result<()> {
    let mut args = args;
    if args.validate && args.apply {
        bail!("{}", tr("cli.wizard.result.validate_apply_conflict"));
    }

    let mut execution = if args.dry_run {
        ExecutionMode::DryRun
    } else {
        args.execution
    };
    if let Some(override_mode) = execution_override {
        execution = override_mode;
    }

    let input_answers = args.answers.as_ref().or(args.qa_answers.as_ref());
    let loaded_answers = match input_answers {
        Some(path) => Some(load_run_answers(path, &args)?),
        None => None,
    };
    let mut answers = loaded_answers
        .as_ref()
        .map(|loaded| loaded.run_answers.clone());
    if args.validate {
        execution = ExecutionMode::DryRun;
    } else if args.apply {
        execution = ExecutionMode::Execute;
    }

    apply_legacy_wizard_new_compat(legacy_new, &mut args, &mut answers)?;

    if answers.is_none() && io::stdin().is_terminal() && io::stdout().is_terminal() {
        answers = Some(collect_interactive_answers(&args)?);
    }

    if let Some(doc) = &answers
        && doc.mode != args.mode
    {
        bail!(
            "{}",
            trf(
                "cli.wizard.result.answers_mode_mismatch",
                &[&format!("{:?}", doc.mode), &format!("{:?}", args.mode)],
            )
        );
    }

    let output = build_run_output(&args, execution, answers.as_ref())?;

    if let Some(path) = &args.qa_answers_out {
        let doc = answers
            .clone()
            .unwrap_or_else(|| default_answers_for(&args));
        let payload = serde_json::to_string_pretty(&doc)?;
        write_json_file(path, &payload, "qa-answers-out")?;
    }

    if let Some(path) = &args.emit_answers {
        let run_answers = answers
            .clone()
            .unwrap_or_else(|| default_answers_for(&args));
        let source_document = loaded_answers
            .as_ref()
            .and_then(|loaded| loaded.source_document.clone());
        let doc = answer_document_from_run_answers(&run_answers, &args, source_document);
        let payload = serde_json::to_string_pretty(&doc)?;
        write_json_file(path, &payload, "emit-answers")?;
    }

    match execution {
        ExecutionMode::DryRun => {
            let plan_out = resolve_plan_out(&args)?;
            write_plan_json(&output.plan, &plan_out)?;
            println!(
                "{}",
                trf(
                    "cli.wizard.result.plan_written",
                    &[plan_out.to_string_lossy().as_ref()],
                )
            );
        }
        ExecutionMode::Execute => {
            execute_run_plan(&output.plan)?;
            if args.mode == RunMode::Create {
                println!(
                    "{}",
                    trf(
                        "cli.wizard.result.component_written",
                        &[output.plan.target_root.to_string_lossy().as_ref()],
                    )
                );
            } else {
                println!("{}", tr("cli.wizard.result.execute_ok"));
            }
        }
    }

    if args.json {
        let json = serde_json::to_string_pretty(&output)?;
        println!("{json}");
    }
    Ok(())
}

fn apply_legacy_wizard_new_compat(
    legacy_new: Option<WizardLegacyNewCompat>,
    args: &mut WizardArgs,
    answers: &mut Option<WizardRunAnswers>,
) -> Result<()> {
    let Some(legacy_new) = legacy_new else {
        return Ok(());
    };

    let component_name = legacy_new.name.unwrap_or_else(|| "component".to_string());
    ComponentName::parse(&component_name)?;
    let output_parent = legacy_new.out.unwrap_or_else(|| args.project_root.clone());
    let output_dir = output_parent.join(&component_name);

    args.mode = RunMode::Create;
    let mut doc = answers.take().unwrap_or_else(|| default_answers_for(args));
    doc.mode = RunMode::Create;
    doc.fields.insert(
        "component_name".to_string(),
        JsonValue::String(component_name),
    );
    doc.fields.insert(
        "output_dir".to_string(),
        JsonValue::String(output_dir.display().to_string()),
    );
    *answers = Some(doc);
    Ok(())
}

fn build_run_output(
    args: &WizardArgs,
    execution: ExecutionMode,
    answers: Option<&WizardRunAnswers>,
) -> Result<WizardRunOutput> {
    let mode = args.mode;

    let (plan, warnings) = match mode {
        RunMode::Create => build_create_plan(args, execution, answers)?,
        RunMode::BuildTest => build_build_test_plan(args, answers),
        RunMode::Doctor => build_doctor_plan(args, answers),
    };

    Ok(WizardRunOutput {
        mode,
        execution,
        plan,
        warnings,
    })
}

fn resolve_plan_out(args: &WizardArgs) -> Result<PathBuf> {
    if let Some(path) = &args.plan_out {
        return Ok(path.clone());
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return prompt_path(
            tr("cli.wizard.prompt.plan_out"),
            Some("./answers.json".to_string()),
        );
    }
    bail!(
        "{}",
        tr("cli.wizard.result.plan_out_required_non_interactive")
    );
}

fn write_plan_json(plan: &WizardPlanEnvelope, path: &PathBuf) -> Result<()> {
    let payload = serde_json::to_string_pretty(plan)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create plan-out parent {}", parent.display()))?;
    }
    fs::write(path, payload).with_context(|| format!("failed to write plan {}", path.display()))
}

fn build_create_plan(
    args: &WizardArgs,
    execution: ExecutionMode,
    answers: Option<&WizardRunAnswers>,
) -> Result<(WizardPlanEnvelope, Vec<String>)> {
    let fields = answers.map(|doc| &doc.fields);

    let component_name = fields
        .and_then(|f| f.get("component_name"))
        .and_then(JsonValue::as_str)
        .unwrap_or("component");
    let component_name = ComponentName::parse(component_name)?.into_string();

    let abi_version = fields
        .and_then(|f| f.get("abi_version"))
        .and_then(JsonValue::as_str)
        .unwrap_or("0.6.0");
    let abi_version = normalize_version(abi_version)?;

    let output_dir = fields
        .and_then(|f| f.get("output_dir"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| args.project_root.join(&component_name));

    let overwrite_output = fields
        .and_then(|f| f.get("overwrite_output"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    if overwrite_output {
        if execution == ExecutionMode::Execute && output_dir.exists() {
            fs::remove_dir_all(&output_dir).with_context(|| {
                format!(
                    "failed to clear output directory before overwrite {}",
                    output_dir.display()
                )
            })?;
        }
    } else {
        validate_output_path_available(&output_dir)?;
    }

    let template_id = args
        .template
        .clone()
        .or_else(|| {
            fields
                .and_then(|f| f.get("template_id"))
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(default_template_id);

    let required_capabilities = parse_string_array(fields, "required_capabilities");
    let provided_capabilities = parse_string_array(fields, "provided_capabilities");

    let prefill = fields
        .and_then(|f| f.get("prefill_answers"))
        .filter(|value| value.is_object())
        .map(|value| -> Result<AnswersPayload> {
            let json = serde_json::to_string_pretty(value)?;
            let cbor = greentic_types::cbor::canonical::to_canonical_cbor_allow_floats(value)
                .map_err(|err| anyhow!("failed to encode prefill_answers: {err}"))?;
            Ok(AnswersPayload { json, cbor })
        })
        .transpose()?;

    let request = wizard::WizardRequest {
        name: component_name,
        abi_version,
        mode: wizard::WizardMode::Default,
        target: output_dir,
        answers: prefill,
        required_capabilities,
        provided_capabilities,
    };

    let result = wizard::apply_scaffold(request, true)?;
    let mut warnings = result.warnings;
    warnings.push(trf("cli.wizard.step.template_used", &[&template_id]));
    Ok((result.plan, warnings))
}

fn build_build_test_plan(
    args: &WizardArgs,
    answers: Option<&WizardRunAnswers>,
) -> (WizardPlanEnvelope, Vec<String>) {
    let fields = answers.map(|doc| &doc.fields);
    let project_root = fields
        .and_then(|f| f.get("project_root"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| args.project_root.clone());

    let mut steps = vec![WizardStep::BuildComponent {
        project_root: project_root.display().to_string(),
    }];

    let full_tests = fields
        .and_then(|f| f.get("full_tests"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(args.full_tests);

    if full_tests {
        steps.push(WizardStep::TestComponent {
            project_root: project_root.display().to_string(),
            full: true,
        });
    }

    (
        WizardPlanEnvelope {
            plan_version: wizard::PLAN_VERSION,
            metadata: WizardPlanMetadata {
                generator: "greentic-component/wizard-runner".to_string(),
                template_version: "component-wizard-run/v1".to_string(),
                template_digest_blake3: "mode-build-test".to_string(),
                requested_abi_version: "0.6.0".to_string(),
            },
            target_root: project_root,
            plan: wizard::WizardPlan {
                meta: wizard::WizardPlanMeta {
                    id: "greentic.component.build_test".to_string(),
                    target: wizard::WizardTarget::Component,
                    mode: wizard::WizardPlanMode::Scaffold,
                },
                steps,
            },
        },
        Vec::new(),
    )
}

fn build_doctor_plan(
    args: &WizardArgs,
    answers: Option<&WizardRunAnswers>,
) -> (WizardPlanEnvelope, Vec<String>) {
    let fields = answers.map(|doc| &doc.fields);
    let project_root = fields
        .and_then(|f| f.get("project_root"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| args.project_root.clone());

    (
        WizardPlanEnvelope {
            plan_version: wizard::PLAN_VERSION,
            metadata: WizardPlanMetadata {
                generator: "greentic-component/wizard-runner".to_string(),
                template_version: "component-wizard-run/v1".to_string(),
                template_digest_blake3: "mode-doctor".to_string(),
                requested_abi_version: "0.6.0".to_string(),
            },
            target_root: project_root.clone(),
            plan: wizard::WizardPlan {
                meta: wizard::WizardPlanMeta {
                    id: "greentic.component.doctor".to_string(),
                    target: wizard::WizardTarget::Component,
                    mode: wizard::WizardPlanMode::Scaffold,
                },
                steps: vec![WizardStep::Doctor {
                    project_root: project_root.display().to_string(),
                }],
            },
        },
        Vec::new(),
    )
}

fn execute_run_plan(plan: &WizardPlanEnvelope) -> Result<()> {
    for step in &plan.plan.steps {
        match step {
            WizardStep::EnsureDir { .. } | WizardStep::WriteFiles { .. } => {
                let single = WizardPlanEnvelope {
                    plan_version: plan.plan_version,
                    metadata: plan.metadata.clone(),
                    target_root: plan.target_root.clone(),
                    plan: wizard::WizardPlan {
                        meta: plan.plan.meta.clone(),
                        steps: vec![step.clone()],
                    },
                };
                wizard::execute_plan(&single)?;
            }
            WizardStep::BuildComponent { project_root } => {
                let manifest = PathBuf::from(project_root).join("component.manifest.json");
                crate::cmd::build::run(BuildArgs {
                    manifest,
                    cargo_bin: None,
                    no_flow: false,
                    no_infer_config: false,
                    no_write_schema: false,
                    force_write_schema: false,
                    no_validate: false,
                    json: false,
                    permissive: false,
                })?;
            }
            WizardStep::Doctor { project_root } => {
                crate::cmd::doctor::run(DoctorArgs {
                    target: project_root.clone(),
                    manifest: None,
                    format: DoctorFormat::Human,
                })
                .map_err(|err| anyhow!(err.to_string()))?;
            }
            WizardStep::TestComponent { project_root, full } => {
                if *full {
                    let status = Command::new("cargo")
                        .arg("test")
                        .current_dir(project_root)
                        .status()
                        .with_context(|| format!("failed to run cargo test in {project_root}"))?;
                    if !status.success() {
                        bail!("cargo test failed in {}", project_root);
                    }
                }
            }
            WizardStep::RunCli { command } => {
                bail!("wizard: unsupported plan step run_cli ({command})");
            }
            WizardStep::Delegate { id } => {
                bail!("wizard: unsupported plan step delegate ({})", id.as_str());
            }
        }
    }
    Ok(())
}

fn parse_string_array(fields: Option<&JsonMap<String, JsonValue>>, key: &str) -> Vec<String> {
    fields
        .and_then(|f| f.get(key))
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn load_run_answers(path: &PathBuf, args: &WizardArgs) -> Result<LoadedRunAnswers> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read qa answers {}", path.display()))?;
    let value: JsonValue = serde_json::from_str(&raw)
        .with_context(|| format!("qa answers {} must be valid JSON", path.display()))?;

    if let Some(doc) = parse_answer_document(&value)? {
        let migrated = maybe_migrate_document(doc, args)?;
        let run_answers = run_answers_from_answer_document(&migrated, args)?;
        return Ok(LoadedRunAnswers {
            run_answers,
            source_document: Some(migrated),
        });
    }

    let answers: WizardRunAnswers = serde_json::from_value(value)
        .with_context(|| format!("qa answers {} must be valid JSON", path.display()))?;
    if answers.schema != WIZARD_RUN_SCHEMA {
        bail!(
            "{}",
            trf(
                "cli.wizard.result.invalid_schema",
                &[&answers.schema, WIZARD_RUN_SCHEMA],
            )
        );
    }
    Ok(LoadedRunAnswers {
        run_answers: answers,
        source_document: None,
    })
}

fn parse_answer_document(value: &JsonValue) -> Result<Option<AnswerDocument>> {
    let JsonValue::Object(map) = value else {
        return Ok(None);
    };
    if map.contains_key("wizard_id")
        || map.contains_key("schema_id")
        || map.contains_key("schema_version")
        || map.contains_key("answers")
    {
        let doc: AnswerDocument = serde_json::from_value(value.clone())
            .with_context(|| tr("cli.wizard.result.answer_doc_invalid_shape"))?;
        return Ok(Some(doc));
    }
    Ok(None)
}

fn maybe_migrate_document(doc: AnswerDocument, args: &WizardArgs) -> Result<AnswerDocument> {
    if doc.schema_id != ANSWER_DOC_SCHEMA_ID {
        bail!(
            "{}",
            trf(
                "cli.wizard.result.answer_schema_id_mismatch",
                &[&doc.schema_id, ANSWER_DOC_SCHEMA_ID],
            )
        );
    }
    let target_version = requested_schema_version(args);
    if doc.schema_version == target_version {
        return Ok(doc);
    }
    if !args.migrate {
        bail!(
            "{}",
            trf(
                "cli.wizard.result.answer_schema_version_mismatch",
                &[&doc.schema_version, &target_version],
            )
        );
    }
    let mut migrated = doc;
    migrated.schema_version = target_version;
    Ok(migrated)
}

fn run_answers_from_answer_document(
    doc: &AnswerDocument,
    args: &WizardArgs,
) -> Result<WizardRunAnswers> {
    let mode = doc
        .answers
        .get("mode")
        .and_then(JsonValue::as_str)
        .map(parse_run_mode)
        .transpose()?
        .unwrap_or(args.mode);
    let fields = match doc.answers.get("fields") {
        Some(JsonValue::Object(fields)) => fields.clone(),
        _ => doc.answers.clone(),
    };
    Ok(WizardRunAnswers {
        schema: WIZARD_RUN_SCHEMA.to_string(),
        mode,
        fields,
    })
}

fn parse_run_mode(value: &str) -> Result<RunMode> {
    match value {
        "create" => Ok(RunMode::Create),
        "build-test" | "build_test" => Ok(RunMode::BuildTest),
        "doctor" => Ok(RunMode::Doctor),
        _ => bail!(
            "{}",
            trf("cli.wizard.result.answer_mode_unsupported", &[value])
        ),
    }
}

fn answer_document_from_run_answers(
    run_answers: &WizardRunAnswers,
    args: &WizardArgs,
    source_document: Option<AnswerDocument>,
) -> AnswerDocument {
    let locale = i18n::selected_locale().to_string();
    let mut answers = JsonMap::new();
    answers.insert(
        "mode".to_string(),
        JsonValue::String(mode_name(run_answers.mode).replace('_', "-")),
    );
    answers.insert(
        "fields".to_string(),
        JsonValue::Object(run_answers.fields.clone()),
    );

    let locks = source_document
        .as_ref()
        .map(|doc| doc.locks.clone())
        .unwrap_or_default();

    AnswerDocument {
        wizard_id: source_document
            .as_ref()
            .map(|doc| doc.wizard_id.clone())
            .unwrap_or_else(|| ANSWER_DOC_WIZARD_ID.to_string()),
        schema_id: source_document
            .as_ref()
            .map(|doc| doc.schema_id.clone())
            .unwrap_or_else(|| ANSWER_DOC_SCHEMA_ID.to_string()),
        schema_version: requested_schema_version(args),
        locale: Some(locale),
        answers,
        locks,
    }
}

fn requested_schema_version(args: &WizardArgs) -> String {
    args.schema_version
        .clone()
        .unwrap_or_else(|| ANSWER_DOC_SCHEMA_VERSION.to_string())
}

fn write_json_file(path: &PathBuf, payload: &str, label: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {label} parent {}", parent.display()))?;
    }
    fs::write(path, payload).with_context(|| format!("failed to write {label} {}", path.display()))
}

fn default_answers_for(args: &WizardArgs) -> WizardRunAnswers {
    WizardRunAnswers {
        schema: WIZARD_RUN_SCHEMA.to_string(),
        mode: args.mode,
        fields: JsonMap::new(),
    }
}

fn collect_interactive_answers(args: &WizardArgs) -> Result<WizardRunAnswers> {
    let locale = i18n::selected_locale().to_string();
    let config = WizardRunConfig {
        spec_json: build_qa_spec(args).to_string(),
        initial_answers_json: Some(default_qa_answers(args).to_string()),
        frontend: WizardFrontend::Text,
        i18n: I18nConfig {
            locale: Some(locale.clone()),
            resolved: Some(build_resolved_i18n(&locale)),
            debug: false,
        },
        verbose: false,
    };
    let mut driver = WizardDriver::new(config)
        .map_err(|err| anyhow!("wizard QA flow failed (greentic-qa-lib): {err}"))?;
    let mut answered = JsonMap::new();

    loop {
        driver
            .next_payload_json()
            .map_err(|err| anyhow!("wizard QA flow failed (greentic-qa-lib): {err}"))?;
        if driver.is_complete() {
            break;
        }
        let ui_raw = driver.last_ui_json().ok_or_else(|| {
            anyhow!("wizard QA flow failed (greentic-qa-lib): missing ui payload")
        })?;
        let ui: JsonValue = serde_json::from_str(ui_raw)
            .with_context(|| "wizard QA flow failed (greentic-qa-lib): parse ui payload")?;
        let question_id = ui
            .get("next_question_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                anyhow!("wizard QA flow failed (greentic-qa-lib): missing next_question_id")
            })?
            .to_string();
        let question = question_for_id(&ui, &question_id)?;
        let answer = loop {
            let answer = prompt_for_wizard_answer(
                &question_id,
                question,
                fallback_default_for_question(args, &question_id, &answered),
            )
            .map_err(|err| anyhow!("wizard QA flow failed (greentic-qa-lib): {err}"))?;
            if args.mode == RunMode::Create
                && question_id == "output_dir"
                && let Some(path) = answer.as_str()
            {
                let path = PathBuf::from(path);
                if path_exists_and_non_empty(&path)? {
                    let overwrite = prompt_yes_no(
                        trf(
                            "cli.wizard.prompt.overwrite_dir",
                            &[path.to_string_lossy().as_ref()],
                        ),
                        false,
                    )?;
                    if overwrite {
                        answered.insert("overwrite_output".to_string(), JsonValue::Bool(true));
                        break answer;
                    }
                    println!("{}", tr("cli.wizard.result.choose_another_output_dir"));
                    continue;
                }
            }
            break answer;
        };
        answered.insert(question_id.clone(), answer.clone());
        let _submit = driver
            .submit_patch_json(&json!({ question_id: answer }).to_string())
            .map_err(|err| anyhow!("wizard QA flow failed (greentic-qa-lib): {err}"))?;
    }

    let result = driver
        .finish()
        .map_err(|err| anyhow!("wizard QA flow failed (greentic-qa-lib): {err}"))?;
    let mut fields = match result.answer_set.answers {
        JsonValue::Object(map) => map,
        _ => JsonMap::new(),
    };
    if let Some(overwrite) = answered.get("overwrite_output").cloned() {
        fields.insert("overwrite_output".to_string(), overwrite);
    }
    Ok(WizardRunAnswers {
        schema: WIZARD_RUN_SCHEMA.to_string(),
        mode: args.mode,
        fields,
    })
}

fn build_qa_spec(args: &WizardArgs) -> JsonValue {
    let locale = i18n::selected_locale().to_string();
    let questions = match args.mode {
        RunMode::Create => {
            let templates = available_template_ids();
            let mut create = vec![
                json!({
                    "id": "component_name",
                    "type": "string",
                    "title": tr("cli.wizard.prompt.component_name"),
                    "title_i18n": {"key":"cli.wizard.prompt.component_name"},
                    "required": true,
                    "default": "component"
                }),
                json!({
                    "id": "output_dir",
                    "type": "string",
                    "title": tr("cli.wizard.prompt.output_dir"),
                    "title_i18n": {"key":"cli.wizard.prompt.output_dir"},
                    "required": true,
                    "default": args.project_root.join("component").display().to_string()
                }),
                json!({
                    "id": "abi_version",
                    "type": "string",
                    "title": tr("cli.wizard.prompt.abi_version"),
                    "title_i18n": {"key":"cli.wizard.prompt.abi_version"},
                    "required": true,
                    "default": "0.6.0"
                }),
            ];
            if args.template.is_none() && templates.len() > 1 {
                let template_choices = templates
                    .into_iter()
                    .map(JsonValue::String)
                    .collect::<Vec<_>>();
                create.push(json!({
                    "id": "template_id",
                    "type": "enum",
                    "title": tr("cli.wizard.prompt.template_id"),
                    "title_i18n": {"key":"cli.wizard.prompt.template_id"},
                    "required": true,
                    "default": "component-v0_6",
                    "choices": template_choices
                }));
            }
            create
        }
        RunMode::BuildTest => vec![
            json!({
                "id": "project_root",
                "type": "string",
                "title": tr("cli.wizard.prompt.project_root"),
                "title_i18n": {"key":"cli.wizard.prompt.project_root"},
                "required": true,
                "default": args.project_root.display().to_string()
            }),
            json!({
                "id": "full_tests",
                "type": "boolean",
                "title": tr("cli.wizard.prompt.full_tests"),
                "title_i18n": {"key":"cli.wizard.prompt.full_tests"},
                "required": false,
                "default": args.full_tests
            }),
        ],
        RunMode::Doctor => vec![json!({
            "id": "project_root",
            "type": "string",
            "title": tr("cli.wizard.prompt.project_root"),
            "title_i18n": {"key":"cli.wizard.prompt.project_root"},
            "required": true,
            "default": args.project_root.display().to_string()
        })],
    };

    json!({
        "id": format!("component.wizard.run.{}", mode_name(args.mode)),
        "title": tr("cli.wizard.result.interactive_header"),
        "version": requested_schema_version(args),
        "presentation": {"default_locale": locale},
        "questions": questions,
    })
}

fn default_qa_answers(args: &WizardArgs) -> JsonValue {
    let mut map = JsonMap::new();
    if args.mode == RunMode::Create
        && let Some(template) = &args.template
    {
        map.insert(
            "template_id".to_string(),
            JsonValue::String(template.clone()),
        );
    }
    JsonValue::Object(map)
}

fn available_template_ids() -> Vec<String> {
    vec!["component-v0_6".to_string()]
}

fn default_template_id() -> String {
    available_template_ids()
        .into_iter()
        .next()
        .unwrap_or_else(|| "component-v0_6".to_string())
}

fn build_resolved_i18n(locale: &str) -> ResolvedI18nMap {
    i18n::resolved_catalog(locale)
}

fn mode_name(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Create => "create",
        RunMode::BuildTest => "build_test",
        RunMode::Doctor => "doctor",
    }
}

fn question_for_id<'a>(ui: &'a JsonValue, question_id: &str) -> Result<&'a JsonValue> {
    ui.get("questions")
        .and_then(JsonValue::as_array)
        .and_then(|questions| {
            questions.iter().find(|question| {
                question.get("id").and_then(JsonValue::as_str) == Some(question_id)
            })
        })
        .ok_or_else(|| anyhow!("wizard QA flow failed (greentic-qa-lib): missing question"))
}

fn prompt_for_wizard_answer(
    question_id: &str,
    question: &JsonValue,
    fallback_default: Option<JsonValue>,
) -> Result<JsonValue, QaLibError> {
    let title = question
        .get("title")
        .and_then(JsonValue::as_str)
        .unwrap_or(question_id);
    let required = question
        .get("required")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let kind = question
        .get("type")
        .and_then(JsonValue::as_str)
        .unwrap_or("string");
    let default_owned = question.get("default").cloned().or(fallback_default);
    let default = default_owned.as_ref();

    match kind {
        "string" if question_id == "component_name" => {
            prompt_component_name_value(title, required, default)
        }
        "string" => prompt_string_value(title, required, default),
        "boolean" => prompt_bool_value(title, required, default),
        "enum" => prompt_enum_value(question_id, title, required, question, default),
        _ => prompt_string_value(title, required, default),
    }
}

fn prompt_component_name_value(
    title: &str,
    required: bool,
    default: Option<&JsonValue>,
) -> Result<JsonValue, QaLibError> {
    loop {
        let value = prompt_string_value(title, required, default)?;
        let Some(name) = value.as_str() else {
            return Ok(value);
        };
        match ComponentName::parse(name) {
            Ok(_) => return Ok(value),
            Err(err) => println!("{err}"),
        }
    }
}

fn prompt_path(label: String, default: Option<String>) -> Result<PathBuf> {
    loop {
        if let Some(value) = &default {
            print!("{label} [{value}]: ");
        } else {
            print!("{label}: ");
        }
        io::stdout().flush()?;
        let mut input = String::new();
        let read = io::stdin().read_line(&mut input)?;
        if read == 0 {
            bail!("stdin closed");
        }
        let trimmed = input.trim();
        if trimmed.is_empty()
            && let Some(value) = &default
        {
            return Ok(PathBuf::from(value));
        }
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
        println!("{}", tr("cli.wizard.result.qa_value_required"));
    }
}

fn path_exists_and_non_empty(path: &PathBuf) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_dir() {
        return Ok(true);
    }
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read output directory {}", path.display()))?;
    Ok(entries.next().is_some())
}

fn validate_output_path_available(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        bail!(
            "target path {} already exists and is not a directory",
            path.display()
        );
    }
    if path_exists_and_non_empty(path)? {
        bail!(
            "target directory {} already exists and is not empty",
            path.display()
        );
    }
    Ok(())
}

fn prompt_yes_no(prompt: String, default_yes: bool) -> Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{prompt} {suffix}: ");
        io::stdout().flush()?;
        let mut line = String::new();
        let read = io::stdin().read_line(&mut line)?;
        if read == 0 {
            bail!("stdin closed");
        }
        let token = line.trim().to_ascii_lowercase();
        if token.is_empty() {
            return Ok(default_yes);
        }
        match token.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("{}", tr("cli.wizard.result.qa_answer_yes_no")),
        }
    }
}

fn fallback_default_for_question(
    args: &WizardArgs,
    question_id: &str,
    answered: &JsonMap<String, JsonValue>,
) -> Option<JsonValue> {
    match (args.mode, question_id) {
        (RunMode::Create, "component_name") => Some(JsonValue::String("component".to_string())),
        (RunMode::Create, "output_dir") => {
            let name = answered
                .get("component_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("component");
            Some(JsonValue::String(
                args.project_root.join(name).display().to_string(),
            ))
        }
        (RunMode::Create, "abi_version") => Some(JsonValue::String("0.6.0".to_string())),
        (RunMode::Create, "template_id") => Some(JsonValue::String(default_template_id())),
        (RunMode::BuildTest, "project_root") | (RunMode::Doctor, "project_root") => {
            Some(JsonValue::String(args.project_root.display().to_string()))
        }
        (RunMode::BuildTest, "full_tests") => Some(JsonValue::Bool(args.full_tests)),
        _ => None,
    }
}

fn prompt_string_value(
    title: &str,
    required: bool,
    default: Option<&JsonValue>,
) -> Result<JsonValue, QaLibError> {
    let default_text = default.and_then(JsonValue::as_str);
    loop {
        if let Some(value) = default_text {
            print!("{title} [{value}]: ");
        } else {
            print!("{title}: ");
        }
        io::stdout()
            .flush()
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        let mut input = String::new();
        let read = io::stdin()
            .read_line(&mut input)
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        if read == 0 {
            return Err(QaLibError::Component("stdin closed".to_string()));
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(value) = default_text {
                return Ok(JsonValue::String(value.to_string()));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(JsonValue::Null);
        }
        return Ok(JsonValue::String(trimmed.to_string()));
    }
}

fn prompt_bool_value(
    title: &str,
    required: bool,
    default: Option<&JsonValue>,
) -> Result<JsonValue, QaLibError> {
    let default_bool = default.and_then(JsonValue::as_bool);
    loop {
        let suffix = match default_bool {
            Some(true) => "[Y/n]",
            Some(false) => "[y/N]",
            None => "[y/n]",
        };
        print!("{title} {suffix}: ");
        io::stdout()
            .flush()
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        let mut input = String::new();
        let read = io::stdin()
            .read_line(&mut input)
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        if read == 0 {
            return Err(QaLibError::Component("stdin closed".to_string()));
        }
        let trimmed = input.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            if let Some(value) = default_bool {
                return Ok(JsonValue::Bool(value));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(JsonValue::Null);
        }
        match trimmed.as_str() {
            "y" | "yes" | "true" | "1" => return Ok(JsonValue::Bool(true)),
            "n" | "no" | "false" | "0" => return Ok(JsonValue::Bool(false)),
            _ => println!("{}", tr("cli.wizard.result.qa_answer_yes_no")),
        }
    }
}

fn prompt_enum_value(
    question_id: &str,
    title: &str,
    required: bool,
    question: &JsonValue,
    default: Option<&JsonValue>,
) -> Result<JsonValue, QaLibError> {
    let choices = question
        .get("choices")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| QaLibError::MissingField("choices".to_string()))?
        .iter()
        .filter_map(JsonValue::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let default_text = default.and_then(JsonValue::as_str);
    if choices.is_empty() {
        return Err(QaLibError::MissingField("choices".to_string()));
    }
    loop {
        println!("{title}:");
        for (idx, choice) in choices.iter().enumerate() {
            println!("  {}. {}", idx + 1, enum_choice_label(question_id, choice));
        }
        if let Some(value) = default_text {
            print!(
                "{} [{value}] ",
                tr("cli.wizard.result.qa_select_number_or_value")
            );
        } else {
            print!("{} ", tr("cli.wizard.result.qa_select_number_or_value"));
        }
        io::stdout()
            .flush()
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        let mut input = String::new();
        let read = io::stdin()
            .read_line(&mut input)
            .map_err(|err| QaLibError::Component(err.to_string()))?;
        if read == 0 {
            return Err(QaLibError::Component("stdin closed".to_string()));
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(value) = default_text {
                return Ok(JsonValue::String(value.to_string()));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(JsonValue::Null);
        }
        if let Ok(n) = trimmed.parse::<usize>()
            && n > 0
            && n <= choices.len()
        {
            return Ok(JsonValue::String(choices[n - 1].clone()));
        }
        if choices.iter().any(|choice| choice == trimmed) {
            return Ok(JsonValue::String(trimmed.to_string()));
        }
        println!("{}", tr("cli.wizard.result.qa_invalid_choice"));
    }
}

fn enum_choice_label<'a>(question_id: &str, choice: &'a str) -> &'a str {
    let _ = question_id;
    choice
}

fn tr(key: &str) -> String {
    i18n::tr_key(key)
}

fn trf(key: &str, args: &[&str]) -> String {
    let mut msg = tr(key);
    for arg in args {
        msg = msg.replacen("{}", arg, 1);
    }
    msg
}
