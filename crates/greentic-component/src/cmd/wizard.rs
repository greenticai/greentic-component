#![cfg(feature = "cli")]

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgMatches, Args, Subcommand, ValueEnum};
use greentic_qa_lib::QaLibError;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::cmd::build::BuildArgs;
use crate::cmd::doctor::{DoctorArgs, DoctorFormat};
use crate::cmd::i18n;
use crate::scaffold::config_schema::{ConfigSchemaInput, parse_config_field};
use crate::scaffold::runtime_capabilities::{
    RuntimeCapabilitiesInput, parse_filesystem_mode, parse_filesystem_mount, parse_secret_format,
    parse_telemetry_attributes, parse_telemetry_scope,
};
use crate::scaffold::validate::{ComponentName, ValidationError, normalize_version};
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
    #[value(alias = "add_operation")]
    #[serde(alias = "add-operation")]
    AddOperation,
    #[value(alias = "update_operation")]
    #[serde(alias = "update-operation")]
    UpdateOperation,
    #[value(alias = "build_test")]
    #[serde(alias = "build-test")]
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

pub(crate) fn maybe_run_schema_from_matches(matches: &ArgMatches) -> Option<Result<()>> {
    let (subcommand, wizard_matches) = matches.subcommand()?;
    if subcommand != "wizard" {
        return None;
    }
    if !wizard_matches.get_flag("schema") {
        return None;
    }

    let args = WizardArgs {
        mode: wizard_matches
            .get_one::<RunMode>("mode")
            .copied()
            .unwrap_or(RunMode::Create),
        execution: wizard_matches
            .get_one::<ExecutionMode>("execution")
            .copied()
            .unwrap_or(ExecutionMode::Execute),
        dry_run: wizard_matches.get_flag("dry_run"),
        validate: wizard_matches.get_flag("validate"),
        apply: wizard_matches.get_flag("apply"),
        qa_answers: wizard_matches.get_one::<PathBuf>("qa_answers").cloned(),
        answers: wizard_matches.get_one::<PathBuf>("answers").cloned(),
        qa_answers_out: wizard_matches.get_one::<PathBuf>("qa_answers_out").cloned(),
        emit_answers: wizard_matches.get_one::<PathBuf>("emit_answers").cloned(),
        schema_version: wizard_matches.get_one::<String>("schema_version").cloned(),
        migrate: wizard_matches.get_flag("migrate"),
        plan_out: wizard_matches.get_one::<PathBuf>("plan_out").cloned(),
        project_root: wizard_matches
            .get_one::<PathBuf>("project_root")
            .cloned()
            .unwrap_or_else(|| PathBuf::from(".")),
        template: wizard_matches.get_one::<String>("template").cloned(),
        full_tests: wizard_matches.get_flag("full_tests"),
        json: wizard_matches.get_flag("json"),
    };

    let schema =
        serde_json::to_string_pretty(&wizard_answer_schema(&args)).map_err(anyhow::Error::from);
    Some(schema.map(|schema| {
        println!("{schema}");
    }))
}

fn is_interactive_session() -> bool {
    if std::env::var_os("GREENTIC_FORCE_NONINTERACTIVE").is_some() {
        return false;
    }
    let running_cli_binary = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .is_some_and(|stem| stem == "greentic-component");
    if !running_cli_binary {
        return false;
    }
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn run_with_context(
    args: WizardArgs,
    execution_override: Option<ExecutionMode>,
    legacy_new: Option<WizardLegacyNewCompat>,
) -> Result<()> {
    let mut args = args;
    let interactive = is_interactive_session();
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
        Some(path) => load_answers_with_recovery(Some(path), &args, interactive, |line| {
            println!("{line}");
        })?,
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

    if answers.is_none() && interactive {
        return run_interactive_loop(args, execution);
    }

    if let Some(doc) = &answers
        && doc.mode != args.mode
    {
        if args.mode == RunMode::Create {
            args.mode = doc.mode;
        } else if interactive {
            report_interactive_validation_error(
                &anyhow!(
                    "{}",
                    trf(
                        "cli.wizard.result.answers_mode_mismatch",
                        &[&format!("{:?}", doc.mode), &format!("{:?}", args.mode)],
                    )
                ),
                |line| println!("{line}"),
            );
            return run_interactive_loop(args, execution);
        } else {
            bail!(
                "{}",
                trf(
                    "cli.wizard.result.answers_mode_mismatch",
                    &[&format!("{:?}", doc.mode), &format!("{:?}", args.mode)],
                )
            );
        }
    }

    let Some(output) =
        build_output_with_recovery(&args, execution, answers.as_ref(), interactive, |line| {
            println!("{line}")
        })?
    else {
        return run_interactive_loop(args, execution);
    };

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

fn run_interactive_loop(mut args: WizardArgs, execution: ExecutionMode) -> Result<()> {
    loop {
        let Some(mode) = prompt_main_menu_mode(args.mode)? else {
            return Ok(());
        };
        args.mode = mode;

        let Some(answers) = collect_interactive_answers(&args)? else {
            continue;
        };
        let Some(output) =
            build_output_with_recovery(&args, execution, Some(&answers), true, |line| {
                println!("{line}");
            })?
        else {
            continue;
        };

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
    }
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
        RunMode::AddOperation => build_add_operation_plan(args, answers)?,
        RunMode::UpdateOperation => build_update_operation_plan(args, answers)?,
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
    if is_interactive_session() {
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

    let user_operations = parse_user_operations(fields)?;
    let default_operation = parse_default_operation(fields, &user_operations);
    let runtime_capabilities = parse_runtime_capabilities(fields)?;

    let prefill = fields
        .and_then(|f| f.get("prefill_answers"))
        .filter(|value| value.is_object())
        .map(|value| -> Result<AnswersPayload> {
            let json = serde_json::to_string_pretty(value)?;
            let cbor = greentic_types::cbor::canonical::to_canonical_cbor_allow_floats(value)
                .map_err(|err| {
                    anyhow!(
                        "{}",
                        trf(
                            "cli.wizard.error.prefill_answers_encode",
                            &[&err.to_string()]
                        )
                    )
                })?;
            Ok(AnswersPayload { json, cbor })
        })
        .transpose()?;

    let request = wizard::WizardRequest {
        name: component_name,
        abi_version,
        mode: wizard::WizardMode::Default,
        target: output_dir,
        answers: prefill,
        required_capabilities: Vec::new(),
        provided_capabilities: Vec::new(),
        user_operations,
        default_operation,
        runtime_capabilities,
        config_schema: parse_config_schema(fields)?,
    };

    let result = wizard::apply_scaffold(request, true)?;
    let mut warnings = result.warnings;
    warnings.push(trf("cli.wizard.step.template_used", &[&template_id]));
    Ok((result.plan, warnings))
}

fn build_add_operation_plan(
    args: &WizardArgs,
    answers: Option<&WizardRunAnswers>,
) -> Result<(WizardPlanEnvelope, Vec<String>)> {
    let fields = answers.map(|doc| &doc.fields);
    let project_root = resolve_project_root(args, fields);
    let manifest_path = project_root.join("component.manifest.json");
    let lib_path = project_root.join("src/lib.rs");
    let operation_name = fields
        .and_then(|f| f.get("operation_name"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.add_operation_name_required")))?;
    let operation_name = normalize_operation_name(operation_name)?;

    let mut manifest: JsonValue = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("manifest {} must be valid JSON", manifest_path.display()))?;
    let user_operations = add_operation_to_manifest(&mut manifest, &operation_name)?;
    if fields
        .and_then(|f| f.get("set_default_operation"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        manifest["default_operation"] = JsonValue::String(operation_name.clone());
    }

    let lib_source = fs::read_to_string(&lib_path)
        .with_context(|| format!("failed to read {}", lib_path.display()))?;
    let updated_lib = rewrite_lib_user_ops(&lib_source, &user_operations)?;

    Ok((
        write_files_plan(
            "greentic.component.add_operation",
            "mode-add-operation",
            &project_root,
            vec![
                (
                    "component.manifest.json".to_string(),
                    serde_json::to_string_pretty(&manifest)?,
                ),
                ("src/lib.rs".to_string(), updated_lib),
            ],
        ),
        Vec::new(),
    ))
}

fn build_update_operation_plan(
    args: &WizardArgs,
    answers: Option<&WizardRunAnswers>,
) -> Result<(WizardPlanEnvelope, Vec<String>)> {
    let fields = answers.map(|doc| &doc.fields);
    let project_root = resolve_project_root(args, fields);
    let manifest_path = project_root.join("component.manifest.json");
    let lib_path = project_root.join("src/lib.rs");
    let operation_name = fields
        .and_then(|f| f.get("operation_name"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.update_operation_name_required")))?;
    let operation_name = normalize_operation_name(operation_name)?;
    let new_name = fields
        .and_then(|f| f.get("new_operation_name"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(normalize_operation_name)
        .transpose()?;

    let mut manifest: JsonValue = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("manifest {} must be valid JSON", manifest_path.display()))?;
    let final_name =
        update_operation_in_manifest(&mut manifest, &operation_name, new_name.as_deref())?;
    if fields
        .and_then(|f| f.get("set_default_operation"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        manifest["default_operation"] = JsonValue::String(final_name.clone());
    }
    let user_operations = collect_user_operation_names(&manifest)?;

    let lib_source = fs::read_to_string(&lib_path)
        .with_context(|| format!("failed to read {}", lib_path.display()))?;
    let updated_lib = rewrite_lib_user_ops(&lib_source, &user_operations)?;

    Ok((
        write_files_plan(
            "greentic.component.update_operation",
            "mode-update-operation",
            &project_root,
            vec![
                (
                    "component.manifest.json".to_string(),
                    serde_json::to_string_pretty(&manifest)?,
                ),
                ("src/lib.rs".to_string(), updated_lib),
            ],
        ),
        Vec::new(),
    ))
}

fn resolve_project_root(args: &WizardArgs, fields: Option<&JsonMap<String, JsonValue>>) -> PathBuf {
    fields
        .and_then(|f| f.get("project_root"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| args.project_root.clone())
}

fn normalize_operation_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{}", tr("cli.wizard.error.operation_name_empty"));
    }
    let is_valid = trimmed.chars().enumerate().all(|(idx, ch)| match idx {
        0 => ch.is_ascii_lowercase(),
        _ => ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | ':' | '-'),
    });
    if !is_valid {
        bail!(
            "{}",
            trf("cli.wizard.error.operation_name_invalid", &[trimmed])
        );
    }
    Ok(trimmed.to_string())
}

fn parse_user_operations(fields: Option<&JsonMap<String, JsonValue>>) -> Result<Vec<String>> {
    if let Some(csv) = fields
        .and_then(|f| f.get("operation_names"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let parsed = parse_operation_names_csv(csv)?;
        if !parsed.is_empty() {
            return Ok(parsed);
        }
    }

    let operations = fields
        .and_then(|f| f.get("operations"))
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| match value {
                    JsonValue::String(name) => Some(name.clone()),
                    JsonValue::Object(map) => map
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .map(ToOwned::to_owned),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !operations.is_empty() {
        return operations
            .into_iter()
            .map(|name| normalize_operation_name(&name))
            .collect();
    }

    if let Some(name) = fields
        .and_then(|f| f.get("primary_operation_name"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(vec![normalize_operation_name(name)?]);
    }

    Ok(vec!["handle_message".to_string()])
}

fn parse_operation_names_csv(value: &str) -> Result<Vec<String>> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(normalize_operation_name)
        .collect()
}

fn parse_default_operation(
    fields: Option<&JsonMap<String, JsonValue>>,
    user_operations: &[String],
) -> Option<String> {
    fields
        .and_then(|f| f.get("default_operation"))
        .and_then(JsonValue::as_str)
        .and_then(|value| user_operations.iter().find(|name| name.as_str() == value))
        .cloned()
        .or_else(|| user_operations.first().cloned())
}

fn parse_runtime_capabilities(
    fields: Option<&JsonMap<String, JsonValue>>,
) -> Result<RuntimeCapabilitiesInput> {
    let filesystem_mode = fields
        .and_then(|f| f.get("filesystem_mode"))
        .and_then(JsonValue::as_str)
        .unwrap_or("none");
    let telemetry_scope = fields
        .and_then(|f| f.get("telemetry_scope"))
        .and_then(JsonValue::as_str)
        .unwrap_or("node");
    let filesystem_mounts = parse_string_array(fields, "filesystem_mounts")
        .into_iter()
        .map(|value| parse_filesystem_mount(&value).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let telemetry_attributes =
        parse_telemetry_attributes(&parse_string_array(fields, "telemetry_attributes"))
            .map_err(anyhow::Error::from)?;
    let telemetry_span_prefix = fields
        .and_then(|f| f.get("telemetry_span_prefix"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(RuntimeCapabilitiesInput {
        filesystem_mode: parse_filesystem_mode(filesystem_mode).map_err(anyhow::Error::from)?,
        filesystem_mounts,
        messaging_inbound: fields
            .and_then(|f| f.get("messaging_inbound"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        messaging_outbound: fields
            .and_then(|f| f.get("messaging_outbound"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        events_inbound: fields
            .and_then(|f| f.get("events_inbound"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        events_outbound: fields
            .and_then(|f| f.get("events_outbound"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        http_client: fields
            .and_then(|f| f.get("http_client"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        http_server: fields
            .and_then(|f| f.get("http_server"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        state_read: fields
            .and_then(|f| f.get("state_read"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        state_write: fields
            .and_then(|f| f.get("state_write"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        state_delete: fields
            .and_then(|f| f.get("state_delete"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        telemetry_scope: parse_telemetry_scope(telemetry_scope).map_err(anyhow::Error::from)?,
        telemetry_span_prefix,
        telemetry_attributes,
        secret_keys: parse_string_array(fields, "secret_keys"),
        secret_env: fields
            .and_then(|f| f.get("secret_env"))
            .and_then(JsonValue::as_str)
            .unwrap_or("dev")
            .trim()
            .to_string(),
        secret_tenant: fields
            .and_then(|f| f.get("secret_tenant"))
            .and_then(JsonValue::as_str)
            .unwrap_or("default")
            .trim()
            .to_string(),
        secret_format: parse_secret_format(
            fields
                .and_then(|f| f.get("secret_format"))
                .and_then(JsonValue::as_str)
                .unwrap_or("text"),
        )
        .map_err(anyhow::Error::from)?,
    })
}

fn parse_config_schema(fields: Option<&JsonMap<String, JsonValue>>) -> Result<ConfigSchemaInput> {
    Ok(ConfigSchemaInput {
        fields: parse_string_array(fields, "config_fields")
            .into_iter()
            .map(|value| parse_config_field(&value).map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?,
    })
}

fn default_operation_schema(component_name: &str, operation_name: &str) -> JsonValue {
    json!({
        "name": operation_name,
        "input_schema": {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": format!("{component_name} {operation_name} input"),
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": {
                    "type": "string",
                    "default": format!("Hello from {component_name}!")
                }
            },
            "additionalProperties": false
        },
        "output_schema": {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": format!("{component_name} {operation_name} output"),
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": { "type": "string" }
            },
            "additionalProperties": false
        }
    })
}

fn add_operation_to_manifest(
    manifest: &mut JsonValue,
    operation_name: &str,
) -> Result<Vec<String>> {
    let component_name = manifest
        .get("name")
        .and_then(JsonValue::as_str)
        .unwrap_or("component")
        .to_string();
    let operations = manifest
        .get_mut("operations")
        .and_then(JsonValue::as_array_mut)
        .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.manifest_operations_array")))?;
    if operations.iter().any(|entry| {
        entry
            .get("name")
            .and_then(JsonValue::as_str)
            .is_some_and(|name| name == operation_name)
    }) {
        bail!(
            "{}",
            trf("cli.wizard.error.operation_exists", &[operation_name])
        );
    }
    operations.push(default_operation_schema(&component_name, operation_name));
    collect_user_operation_names(manifest)
}

fn update_operation_in_manifest(
    manifest: &mut JsonValue,
    operation_name: &str,
    new_name: Option<&str>,
) -> Result<String> {
    let operations = manifest
        .get_mut("operations")
        .and_then(JsonValue::as_array_mut)
        .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.manifest_operations_array")))?;
    let target_index = operations.iter().position(|entry| {
        entry
            .get("name")
            .and_then(JsonValue::as_str)
            .is_some_and(|name| name == operation_name)
    });
    let Some(target_index) = target_index else {
        bail!(
            "{}",
            trf("cli.wizard.error.operation_not_found", &[operation_name])
        );
    };
    let final_name = new_name.unwrap_or(operation_name).to_string();
    if final_name != operation_name
        && operations.iter().any(|other| {
            other
                .get("name")
                .and_then(JsonValue::as_str)
                .is_some_and(|name| name == final_name)
        })
    {
        bail!(
            "{}",
            trf("cli.wizard.error.operation_exists", &[&final_name])
        );
    }
    let entry = operations.get_mut(target_index).ok_or_else(|| {
        anyhow!(
            "{}",
            trf("cli.wizard.error.operation_not_found", &[operation_name])
        )
    })?;
    entry["name"] = JsonValue::String(final_name.clone());
    if manifest
        .get("default_operation")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value == operation_name)
    {
        manifest["default_operation"] = JsonValue::String(final_name.clone());
    }
    Ok(final_name)
}

fn collect_user_operation_names(manifest: &JsonValue) -> Result<Vec<String>> {
    let operations = manifest
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.manifest_operations_array")))?;
    Ok(operations
        .iter()
        .filter_map(|entry| entry.get("name").and_then(JsonValue::as_str))
        .filter(|name| !matches!(*name, "qa-spec" | "apply-answers" | "i18n-keys"))
        .map(ToOwned::to_owned)
        .collect())
}

fn write_files_plan(
    id: &str,
    digest: &str,
    project_root: &Path,
    files: Vec<(String, String)>,
) -> WizardPlanEnvelope {
    let file_map = files
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    WizardPlanEnvelope {
        plan_version: wizard::PLAN_VERSION,
        metadata: WizardPlanMetadata {
            generator: "greentic-component/wizard-runner".to_string(),
            template_version: "component-wizard-run/v1".to_string(),
            template_digest_blake3: digest.to_string(),
            requested_abi_version: "0.6.0".to_string(),
        },
        target_root: project_root.to_path_buf(),
        plan: wizard::WizardPlan {
            meta: wizard::WizardPlanMeta {
                id: id.to_string(),
                target: wizard::WizardTarget::Component,
                mode: wizard::WizardPlanMode::Scaffold,
            },
            steps: vec![WizardStep::WriteFiles { files: file_map }],
        },
    }
}

fn rewrite_lib_user_ops(source: &str, user_operations: &[String]) -> Result<String> {
    let generated = user_operations
        .iter()
        .map(|name| {
            format!(
                r#"                node::Op {{
                    name: "{name}".to_string(),
                    summary: Some("Handle a single message input".to_string()),
                    input: node::IoSchema {{
                        schema: node::SchemaSource::InlineCbor(input_schema_cbor.clone()),
                        content_type: "application/cbor".to_string(),
                        schema_version: None,
                    }},
                    output: node::IoSchema {{
                        schema: node::SchemaSource::InlineCbor(output_schema_cbor.clone()),
                        content_type: "application/cbor".to_string(),
                        schema_version: None,
                    }},
                    examples: Vec::new(),
                }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    if let Some(start) = source.find("            ops: vec![")
        && let Some(end_rel) = source[start..].find("            schemas: Vec::new(),")
    {
        let end = start + end_rel;
        let qa_anchor = source[start..end]
            .find("                node::Op {\n                    name: \"qa-spec\".to_string(),")
            .map(|idx| start + idx)
            .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.lib_missing_qa_block")))?;
        let mut updated = String::new();
        updated.push_str(&source[..start]);
        updated.push_str("            ops: vec![\n");
        updated.push_str(&generated);
        updated.push_str(",\n");
        updated.push_str(&source[qa_anchor..end]);
        updated.push_str(&source[end..]);
        return Ok(updated);
    }

    if let Some(start) = source.find("        let mut ops = vec![")
        && let Some(end_anchor_rel) = source[start..].find("        ops.extend(vec![")
    {
        let end = start + end_anchor_rel;
        let mut updated = String::new();
        updated.push_str(&source[..start]);
        updated.push_str("        let mut ops = vec![\n");
        updated.push_str(
            &user_operations
                .iter()
                .map(|name| {
                    format!(
                        r#"            node::Op {{
                name: "{name}".to_string(),
                summary: Some("Handle a single message input".to_string()),
                input: node::IoSchema {{
                    schema: node::SchemaSource::InlineCbor(input_schema_cbor.clone()),
                    content_type: "application/cbor".to_string(),
                    schema_version: None,
                }},
                output: node::IoSchema {{
                    schema: node::SchemaSource::InlineCbor(output_schema_cbor.clone()),
                    content_type: "application/cbor".to_string(),
                    schema_version: None,
                }},
                examples: Vec::new(),
            }}"#
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n"),
        );
        updated.push_str("\n        ];\n");
        updated.push_str(&source[end..]);
        return Ok(updated);
    }

    bail!("{}", tr("cli.wizard.error.lib_unexpected_layout"))
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
                let manifest = PathBuf::from(project_root).join("component.manifest.json");
                crate::cmd::doctor::run(DoctorArgs {
                    target: project_root.clone(),
                    manifest: Some(manifest),
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
                        bail!(
                            "{}",
                            trf("cli.wizard.error.cargo_test_failed_in", &[project_root])
                        );
                    }
                }
            }
            WizardStep::RunCli { command } => {
                bail!(
                    "{}",
                    trf("cli.wizard.error.unsupported_run_cli", &[command])
                );
            }
            WizardStep::Delegate { id } => {
                bail!(
                    "{}",
                    trf("cli.wizard.error.unsupported_delegate", &[id.as_str()])
                );
            }
        }
    }
    Ok(())
}

fn parse_string_array(fields: Option<&JsonMap<String, JsonValue>>, key: &str) -> Vec<String> {
    match fields.and_then(|f| f.get(key)) {
        Some(JsonValue::Array(values)) => values
            .iter()
            .filter_map(JsonValue::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        Some(JsonValue::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
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

fn load_answers_with_recovery<F>(
    path: Option<&PathBuf>,
    args: &WizardArgs,
    interactive: bool,
    mut report: F,
) -> Result<Option<LoadedRunAnswers>>
where
    F: FnMut(String),
{
    let Some(path) = path else {
        return Ok(None);
    };
    match load_run_answers(path, args) {
        Ok(loaded) => Ok(Some(loaded)),
        Err(err) if interactive => {
            report_interactive_validation_error(&err, &mut report);
            Ok(None)
        }
        Err(err) => Err(err),
    }
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
        "add-operation" | "add_operation" => Ok(RunMode::AddOperation),
        "update-operation" | "update_operation" => Ok(RunMode::UpdateOperation),
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

fn wizard_answer_schema(args: &WizardArgs) -> JsonValue {
    let selected_mode = mode_name(args.mode).replace('_', "-");
    let fields_schema = wizard_answer_fields_schema(args);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://greenticai.github.io/greentic-component/schemas/wizard/{selected_mode}.answers.schema.json"),
        "title": format!("greentic-component wizard {} answers", selected_mode),
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "wizard_id": {
                "type": "string",
                "const": ANSWER_DOC_WIZARD_ID
            },
            "schema_id": {
                "type": "string",
                "const": ANSWER_DOC_SCHEMA_ID
            },
            "schema_version": {
                "type": "string",
                "const": requested_schema_version(args)
            },
            "locale": {
                "type": ["string", "null"]
            },
            "answers": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "mode": {
                        "type": "string",
                        "const": selected_mode
                    },
                    "fields": fields_schema
                },
                "required": ["mode", "fields"]
            },
            "locks": {
                "type": "object",
                "additionalProperties": true
            }
        },
        "required": ["wizard_id", "schema_id", "schema_version", "answers"]
    })
}

fn wizard_answer_fields_schema(args: &WizardArgs) -> JsonValue {
    let questions = match args.mode {
        RunMode::Create => create_questions(args, true),
        _ => interactive_questions(args),
    };
    let mut properties = JsonMap::new();
    let mut required = Vec::new();
    for question in questions {
        let Some(id) = question.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        if question
            .get("required")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            required.push(JsonValue::String(id.to_string()));
        }
        properties.insert(id.to_string(), question_schema_property(&question));
    }
    JsonValue::Object(JsonMap::from_iter([
        ("type".to_string(), JsonValue::String("object".to_string())),
        ("additionalProperties".to_string(), JsonValue::Bool(false)),
        ("properties".to_string(), JsonValue::Object(properties)),
        ("required".to_string(), JsonValue::Array(required)),
    ]))
}

fn question_schema_property(question: &JsonValue) -> JsonValue {
    let mut property = JsonMap::new();
    let question_type = question
        .get("type")
        .and_then(JsonValue::as_str)
        .unwrap_or("string");
    match question_type {
        "boolean" => {
            property.insert("type".to_string(), JsonValue::String("boolean".to_string()));
        }
        "enum" => {
            property.insert("type".to_string(), JsonValue::String("string".to_string()));
            if let Some(choices) = question.get("choices").cloned() {
                property.insert("enum".to_string(), choices);
            }
        }
        _ => {
            property.insert("type".to_string(), JsonValue::String("string".to_string()));
        }
    }
    if let Some(default) = question.get("default").cloned() {
        property.insert("default".to_string(), default);
    }
    JsonValue::Object(property)
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

fn collect_interactive_answers(args: &WizardArgs) -> Result<Option<WizardRunAnswers>> {
    println!("0 = back, M = main menu");
    if args.mode == RunMode::Create {
        return collect_interactive_create_answers(args);
    }

    let Some(fields) = collect_interactive_question_map(args, interactive_questions(args))? else {
        return Ok(None);
    };
    Ok(Some(WizardRunAnswers {
        schema: WIZARD_RUN_SCHEMA.to_string(),
        mode: args.mode,
        fields,
    }))
}

fn collect_interactive_create_answers(args: &WizardArgs) -> Result<Option<WizardRunAnswers>> {
    let mut answered = JsonMap::new();
    let Some(minimal_answers) = collect_interactive_question_map_with_answers(
        args,
        create_questions(args, false),
        answered,
    )?
    else {
        return Ok(None);
    };
    answered = minimal_answers;

    if answered
        .get("advanced_setup")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let Some(advanced_answers) = collect_interactive_question_map_with_skip(
            args,
            create_questions(args, true),
            answered,
            should_skip_create_advanced_question,
        )?
        else {
            return Ok(None);
        };
        answered = advanced_answers;
    }

    let operations = answered
        .get("operation_names")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(parse_operation_names_csv)
        .transpose()?
        .filter(|ops| !ops.is_empty())
        .or_else(|| {
            answered
                .get("primary_operation_name")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| vec![value.to_string()])
        });
    if let Some(operations) = operations {
        let default_operation = operations
            .first()
            .cloned()
            .unwrap_or_else(|| "handle_message".to_string());
        answered.insert(
            "operations".to_string(),
            JsonValue::Array(
                operations
                    .into_iter()
                    .map(JsonValue::String)
                    .collect::<Vec<_>>(),
            ),
        );
        answered.insert(
            "default_operation".to_string(),
            JsonValue::String(default_operation),
        );
    }

    Ok(Some(WizardRunAnswers {
        schema: WIZARD_RUN_SCHEMA.to_string(),
        mode: args.mode,
        fields: answered,
    }))
}

fn interactive_questions(args: &WizardArgs) -> Vec<JsonValue> {
    match args.mode {
        RunMode::Create => create_questions(args, true),
        RunMode::AddOperation => vec![
            json!({
                "id": "project_root",
                "type": "string",
                "title": tr("cli.wizard.prompt.project_root"),
                "title_i18n": {"key":"cli.wizard.prompt.project_root"},
                "required": true,
                "default": args.project_root.display().to_string()
            }),
            json!({
                "id": "operation_name",
                "type": "string",
                "title": tr("cli.wizard.prompt.operation_name"),
                "title_i18n": {"key":"cli.wizard.prompt.operation_name"},
                "required": true
            }),
            json!({
                "id": "set_default_operation",
                "type": "boolean",
                "title": tr("cli.wizard.prompt.set_default_operation"),
                "title_i18n": {"key":"cli.wizard.prompt.set_default_operation"},
                "required": false,
                "default": false
            }),
        ],
        RunMode::UpdateOperation => vec![
            json!({
                "id": "project_root",
                "type": "string",
                "title": tr("cli.wizard.prompt.project_root"),
                "title_i18n": {"key":"cli.wizard.prompt.project_root"},
                "required": true,
                "default": args.project_root.display().to_string()
            }),
            json!({
                "id": "operation_name",
                "type": "string",
                "title": tr("cli.wizard.prompt.existing_operation_name"),
                "title_i18n": {"key":"cli.wizard.prompt.existing_operation_name"},
                "required": true
            }),
            json!({
                "id": "new_operation_name",
                "type": "string",
                "title": tr("cli.wizard.prompt.new_operation_name"),
                "title_i18n": {"key":"cli.wizard.prompt.new_operation_name"},
                "required": false
            }),
            json!({
                "id": "set_default_operation",
                "type": "boolean",
                "title": tr("cli.wizard.prompt.set_default_operation"),
                "title_i18n": {"key":"cli.wizard.prompt.set_default_operation"},
                "required": false,
                "default": false
            }),
        ],
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
    }
}

fn create_questions(args: &WizardArgs, include_advanced: bool) -> Vec<JsonValue> {
    let templates = available_template_ids();
    let mut questions = vec![
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
            "id": "advanced_setup",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.advanced_setup"),
            "title_i18n": {"key":"cli.wizard.prompt.advanced_setup"},
            "required": true,
            "default": false
        }),
    ];
    if !include_advanced {
        return questions;
    }

    questions.extend([
        json!({
            "id": "abi_version",
            "type": "string",
            "title": tr("cli.wizard.prompt.abi_version"),
            "title_i18n": {"key":"cli.wizard.prompt.abi_version"},
            "required": true,
            "default": "0.6.0"
        }),
        json!({
            "id": "operation_names",
            "type": "string",
            "title": tr("cli.wizard.prompt.operation_names"),
            "title_i18n": {"key":"cli.wizard.prompt.operation_names"},
            "required": true,
            "default": "handle_message"
        }),
        json!({
            "id": "filesystem_mode",
            "type": "enum",
            "title": tr("cli.wizard.prompt.filesystem_mode"),
            "title_i18n": {"key":"cli.wizard.prompt.filesystem_mode"},
            "required": true,
            "default": "none",
            "choices": ["none", "read_only", "sandbox"]
        }),
        json!({
            "id": "filesystem_mounts",
            "type": "string",
            "title": tr("cli.wizard.prompt.filesystem_mounts"),
            "title_i18n": {"key":"cli.wizard.prompt.filesystem_mounts"},
            "required": false,
            "default": ""
        }),
        json!({
            "id": "http_client",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.http_client"),
            "title_i18n": {"key":"cli.wizard.prompt.http_client"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "messaging_inbound",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.messaging_inbound"),
            "title_i18n": {"key":"cli.wizard.prompt.messaging_inbound"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "messaging_outbound",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.messaging_outbound"),
            "title_i18n": {"key":"cli.wizard.prompt.messaging_outbound"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "events_inbound",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.events_inbound"),
            "title_i18n": {"key":"cli.wizard.prompt.events_inbound"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "events_outbound",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.events_outbound"),
            "title_i18n": {"key":"cli.wizard.prompt.events_outbound"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "http_server",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.http_server"),
            "title_i18n": {"key":"cli.wizard.prompt.http_server"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "state_read",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.state_read"),
            "title_i18n": {"key":"cli.wizard.prompt.state_read"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "state_write",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.state_write"),
            "title_i18n": {"key":"cli.wizard.prompt.state_write"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "state_delete",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.state_delete"),
            "title_i18n": {"key":"cli.wizard.prompt.state_delete"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "telemetry_scope",
            "type": "enum",
            "title": tr("cli.wizard.prompt.telemetry_scope"),
            "title_i18n": {"key":"cli.wizard.prompt.telemetry_scope"},
            "required": true,
            "default": "node",
            "choices": ["tenant", "pack", "node"]
        }),
        json!({
            "id": "telemetry_span_prefix",
            "type": "string",
            "title": tr("cli.wizard.prompt.telemetry_span_prefix"),
            "title_i18n": {"key":"cli.wizard.prompt.telemetry_span_prefix"},
            "required": false,
            "default": ""
        }),
        json!({
            "id": "telemetry_attributes",
            "type": "string",
            "title": tr("cli.wizard.prompt.telemetry_attributes"),
            "title_i18n": {"key":"cli.wizard.prompt.telemetry_attributes"},
            "required": false,
            "default": ""
        }),
        json!({
            "id": "secrets_enabled",
            "type": "boolean",
            "title": tr("cli.wizard.prompt.secrets_enabled"),
            "title_i18n": {"key":"cli.wizard.prompt.secrets_enabled"},
            "required": false,
            "default": false
        }),
        json!({
            "id": "secret_keys",
            "type": "string",
            "title": tr("cli.wizard.prompt.secret_keys"),
            "title_i18n": {"key":"cli.wizard.prompt.secret_keys"},
            "required": false,
            "default": ""
        }),
        json!({
            "id": "secret_env",
            "type": "string",
            "title": tr("cli.wizard.prompt.secret_env"),
            "title_i18n": {"key":"cli.wizard.prompt.secret_env"},
            "required": false,
            "default": "dev"
        }),
        json!({
            "id": "secret_tenant",
            "type": "string",
            "title": tr("cli.wizard.prompt.secret_tenant"),
            "title_i18n": {"key":"cli.wizard.prompt.secret_tenant"},
            "required": false,
            "default": "default"
        }),
        json!({
            "id": "secret_format",
            "type": "enum",
            "title": tr("cli.wizard.prompt.secret_format"),
            "title_i18n": {"key":"cli.wizard.prompt.secret_format"},
            "required": false,
            "default": "text",
            "choices": ["bytes", "text", "json"]
        }),
        json!({
            "id": "config_fields",
            "type": "string",
            "title": tr("cli.wizard.prompt.config_fields"),
            "title_i18n": {"key":"cli.wizard.prompt.config_fields"},
            "required": false,
            "default": ""
        }),
    ]);
    if args.template.is_none() && templates.len() > 1 {
        let template_choices = templates
            .into_iter()
            .map(JsonValue::String)
            .collect::<Vec<_>>();
        questions.push(json!({
            "id": "template_id",
            "type": "enum",
            "title": tr("cli.wizard.prompt.template_id"),
            "title_i18n": {"key":"cli.wizard.prompt.template_id"},
            "required": true,
            "default": "component-v0_6",
            "choices": template_choices
        }));
    }
    questions
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

fn mode_name(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Create => "create",
        RunMode::AddOperation => "add_operation",
        RunMode::UpdateOperation => "update_operation",
        RunMode::BuildTest => "build_test",
        RunMode::Doctor => "doctor",
    }
}

enum InteractiveAnswer {
    Value(JsonValue),
    Back,
    MainMenu,
}

fn prompt_for_wizard_answer(
    question_id: &str,
    question: &JsonValue,
    fallback_default: Option<JsonValue>,
) -> Result<InteractiveAnswer, QaLibError> {
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
) -> Result<InteractiveAnswer, QaLibError> {
    loop {
        let value = prompt_string_value(title, required, default)?;
        let InteractiveAnswer::Value(value) = value else {
            return Ok(value);
        };
        let Some(name) = value.as_str() else {
            return Ok(InteractiveAnswer::Value(value));
        };
        match ComponentName::parse(name) {
            Ok(_) => return Ok(InteractiveAnswer::Value(value)),
            Err(err) => println!("{}", render_validation_error_detail(&err.into())),
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
            bail!("{}", tr("cli.wizard.error.stdin_closed"));
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
            "{}",
            trf(
                "cli.wizard.error.target_path_not_directory",
                &[path.display().to_string().as_str()]
            )
        );
    }
    if path_exists_and_non_empty(path)? {
        bail!(
            "{}",
            trf(
                "cli.wizard.error.target_dir_not_empty",
                &[path.display().to_string().as_str()]
            )
        );
    }
    Ok(())
}

fn prompt_yes_no(prompt: String, default_yes: bool) -> Result<InteractiveAnswer> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{prompt} {suffix}: ");
        io::stdout().flush()?;
        let mut line = String::new();
        let read = io::stdin().read_line(&mut line)?;
        if read == 0 {
            bail!("{}", tr("cli.wizard.error.stdin_closed"));
        }
        let token = line.trim().to_ascii_lowercase();
        if token == "0" {
            return Ok(InteractiveAnswer::Back);
        }
        if token == "m" {
            return Ok(InteractiveAnswer::MainMenu);
        }
        if token.is_empty() {
            return Ok(InteractiveAnswer::Value(JsonValue::Bool(default_yes)));
        }
        match token.as_str() {
            "y" | "yes" => return Ok(InteractiveAnswer::Value(JsonValue::Bool(true))),
            "n" | "no" => return Ok(InteractiveAnswer::Value(JsonValue::Bool(false))),
            _ => println!("{}", tr("cli.wizard.result.qa_answer_yes_no")),
        }
    }
}

fn prompt_main_menu_mode(default: RunMode) -> Result<Option<RunMode>> {
    println!("{}", tr("cli.wizard.result.interactive_header"));
    println!("1) {}", tr("cli.wizard.menu.create_new_component"));
    println!("2) {}", tr("cli.wizard.menu.add_operation"));
    println!("3) {}", tr("cli.wizard.menu.update_operation"));
    println!("4) {}", tr("cli.wizard.menu.build_and_test_component"));
    println!("5) {}", tr("cli.wizard.menu.doctor_component"));
    println!("0) exit");
    let default_label = match default {
        RunMode::Create => "1",
        RunMode::AddOperation => "2",
        RunMode::UpdateOperation => "3",
        RunMode::BuildTest => "4",
        RunMode::Doctor => "5",
    };
    loop {
        print!(
            "{} ",
            trf("cli.wizard.prompt.select_option", &[default_label])
        );
        io::stdout().flush()?;
        let mut line = String::new();
        let read = io::stdin().read_line(&mut line)?;
        if read == 0 {
            bail!("{}", tr("cli.wizard.error.stdin_closed"));
        }
        let token = line.trim().to_ascii_lowercase();
        if token == "0" {
            return Ok(None);
        }
        if token == "m" {
            continue;
        }
        let selected = if token.is_empty() {
            default_label.to_string()
        } else {
            token
        };
        if let Some(mode) = parse_main_menu_selection(&selected) {
            return Ok(Some(mode));
        }
        println!("{}", tr("cli.wizard.result.qa_value_required"));
    }
}

fn parse_main_menu_selection(value: &str) -> Option<RunMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "create" => Some(RunMode::Create),
        "2" | "add-operation" | "add_operation" => Some(RunMode::AddOperation),
        "3" | "update-operation" | "update_operation" => Some(RunMode::UpdateOperation),
        "4" | "build" | "build-test" | "build_test" => Some(RunMode::BuildTest),
        "5" | "doctor" => Some(RunMode::Doctor),
        _ => None,
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
        (RunMode::Create, "advanced_setup") => Some(JsonValue::Bool(false)),
        (RunMode::Create, "secrets_enabled") => Some(JsonValue::Bool(false)),
        (RunMode::Create, "abi_version") => Some(JsonValue::String("0.6.0".to_string())),
        (RunMode::Create, "operation_names") | (RunMode::Create, "primary_operation_name") => {
            Some(JsonValue::String("handle_message".to_string()))
        }
        (RunMode::Create, "template_id") => Some(JsonValue::String(default_template_id())),
        (RunMode::AddOperation, "project_root")
        | (RunMode::UpdateOperation, "project_root")
        | (RunMode::BuildTest, "project_root")
        | (RunMode::Doctor, "project_root") => {
            Some(JsonValue::String(args.project_root.display().to_string()))
        }
        (RunMode::AddOperation, "set_default_operation")
        | (RunMode::UpdateOperation, "set_default_operation") => Some(JsonValue::Bool(false)),
        (RunMode::BuildTest, "full_tests") => Some(JsonValue::Bool(args.full_tests)),
        _ => None,
    }
}

fn is_secret_question(question_id: &str) -> bool {
    matches!(
        question_id,
        "secret_keys" | "secret_env" | "secret_tenant" | "secret_format"
    )
}

fn should_skip_create_advanced_question(
    question_id: &str,
    answered: &JsonMap<String, JsonValue>,
) -> bool {
    if answered.contains_key(question_id) {
        return true;
    }
    if question_id == "filesystem_mounts"
        && answered
            .get("filesystem_mode")
            .and_then(JsonValue::as_str)
            .is_some_and(|mode| mode == "none")
    {
        return true;
    }
    is_secret_question(question_id)
        && !answered
            .get("secrets_enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
}

fn prompt_string_value(
    title: &str,
    required: bool,
    default: Option<&JsonValue>,
) -> Result<InteractiveAnswer, QaLibError> {
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
            return Err(QaLibError::Component(tr("cli.wizard.error.stdin_closed")));
        }
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("m") {
            return Ok(InteractiveAnswer::MainMenu);
        }
        if trimmed == "0" {
            return Ok(InteractiveAnswer::Back);
        }
        if trimmed.is_empty() {
            if let Some(value) = default_text {
                return Ok(InteractiveAnswer::Value(JsonValue::String(
                    value.to_string(),
                )));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(InteractiveAnswer::Value(JsonValue::Null));
        }
        return Ok(InteractiveAnswer::Value(JsonValue::String(
            trimmed.to_string(),
        )));
    }
}

fn prompt_bool_value(
    title: &str,
    required: bool,
    default: Option<&JsonValue>,
) -> Result<InteractiveAnswer, QaLibError> {
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
            return Err(QaLibError::Component(tr("cli.wizard.error.stdin_closed")));
        }
        let trimmed = input.trim().to_ascii_lowercase();
        if trimmed == "m" {
            return Ok(InteractiveAnswer::MainMenu);
        }
        if trimmed == "0" {
            return Ok(InteractiveAnswer::Back);
        }
        if trimmed.is_empty() {
            if let Some(value) = default_bool {
                return Ok(InteractiveAnswer::Value(JsonValue::Bool(value)));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(InteractiveAnswer::Value(JsonValue::Null));
        }
        match trimmed.as_str() {
            "y" | "yes" | "true" | "1" => {
                return Ok(InteractiveAnswer::Value(JsonValue::Bool(true)));
            }
            "n" | "no" | "false" => return Ok(InteractiveAnswer::Value(JsonValue::Bool(false))),
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
) -> Result<InteractiveAnswer, QaLibError> {
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
            return Err(QaLibError::Component(tr("cli.wizard.error.stdin_closed")));
        }
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("m") {
            return Ok(InteractiveAnswer::MainMenu);
        }
        if trimmed == "0" {
            return Ok(InteractiveAnswer::Back);
        }
        if trimmed.is_empty() {
            if let Some(value) = default_text {
                return Ok(InteractiveAnswer::Value(JsonValue::String(
                    value.to_string(),
                )));
            }
            if required {
                println!("{}", tr("cli.wizard.result.qa_value_required"));
                continue;
            }
            return Ok(InteractiveAnswer::Value(JsonValue::Null));
        }
        if let Ok(n) = trimmed.parse::<usize>()
            && n > 0
            && n <= choices.len()
        {
            return Ok(InteractiveAnswer::Value(JsonValue::String(
                choices[n - 1].clone(),
            )));
        }
        if choices.iter().any(|choice| choice == trimmed) {
            return Ok(InteractiveAnswer::Value(JsonValue::String(
                trimmed.to_string(),
            )));
        }
        println!("{}", tr("cli.wizard.result.qa_invalid_choice"));
    }
}

fn enum_choice_label<'a>(question_id: &str, choice: &'a str) -> &'a str {
    let _ = question_id;
    choice
}

fn collect_interactive_question_map(
    args: &WizardArgs,
    questions: Vec<JsonValue>,
) -> Result<Option<JsonMap<String, JsonValue>>> {
    collect_interactive_question_map_with_answers(args, questions, JsonMap::new())
}

fn collect_interactive_question_map_with_answers(
    args: &WizardArgs,
    questions: Vec<JsonValue>,
    answered: JsonMap<String, JsonValue>,
) -> Result<Option<JsonMap<String, JsonValue>>> {
    collect_interactive_question_map_with_skip(
        args,
        questions,
        answered,
        |_question_id, _answered| false,
    )
}

fn collect_interactive_question_map_with_skip(
    args: &WizardArgs,
    questions: Vec<JsonValue>,
    mut answered: JsonMap<String, JsonValue>,
    should_skip: fn(&str, &JsonMap<String, JsonValue>) -> bool,
) -> Result<Option<JsonMap<String, JsonValue>>> {
    let mut index = 0usize;
    while index < questions.len() {
        let question = &questions[index];
        let question_id = question
            .get("id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| anyhow!("{}", tr("cli.wizard.error.create_missing_question_id")))?
            .to_string();

        if should_skip(&question_id, &answered) {
            index += 1;
            continue;
        }

        match prompt_for_wizard_answer(
            &question_id,
            question,
            fallback_default_for_question(args, &question_id, &answered),
        )
        .map_err(|err| anyhow!("{err}"))?
        {
            InteractiveAnswer::MainMenu => return Ok(None),
            InteractiveAnswer::Back => {
                if let Some(previous) =
                    previous_interactive_question_index(&questions, index, &answered, should_skip)
                {
                    if let Some(previous_id) =
                        questions[previous].get("id").and_then(JsonValue::as_str)
                    {
                        answered.remove(previous_id);
                        if previous_id == "output_dir" {
                            answered.remove("overwrite_output");
                        }
                    }
                    index = previous;
                }
            }
            InteractiveAnswer::Value(answer) => {
                let mut advance = true;
                if question_id == "output_dir"
                    && let Some(path) = answer.as_str()
                {
                    let path = PathBuf::from(path);
                    if path_exists_and_non_empty(&path)? {
                        match prompt_yes_no(
                            trf(
                                "cli.wizard.prompt.overwrite_dir",
                                &[path.to_string_lossy().as_ref()],
                            ),
                            false,
                        )? {
                            InteractiveAnswer::MainMenu => return Ok(None),
                            InteractiveAnswer::Back => {
                                if let Some(previous) = previous_interactive_question_index(
                                    &questions,
                                    index,
                                    &answered,
                                    should_skip,
                                ) {
                                    if let Some(previous_id) =
                                        questions[previous].get("id").and_then(JsonValue::as_str)
                                    {
                                        answered.remove(previous_id);
                                        if previous_id == "output_dir" {
                                            answered.remove("overwrite_output");
                                        }
                                    }
                                    index = previous;
                                }
                                advance = false;
                            }
                            InteractiveAnswer::Value(JsonValue::Bool(true)) => {
                                answered
                                    .insert("overwrite_output".to_string(), JsonValue::Bool(true));
                            }
                            InteractiveAnswer::Value(JsonValue::Bool(false)) => {
                                println!("{}", tr("cli.wizard.result.choose_another_output_dir"));
                                advance = false;
                            }
                            InteractiveAnswer::Value(_) => {
                                advance = false;
                            }
                        }
                    }
                }
                if advance {
                    answered.insert(question_id, answer);
                    index += 1;
                }
            }
        }
    }
    Ok(Some(answered))
}

fn build_output_with_recovery<F>(
    args: &WizardArgs,
    execution: ExecutionMode,
    answers: Option<&WizardRunAnswers>,
    interactive: bool,
    mut report: F,
) -> Result<Option<WizardRunOutput>>
where
    F: FnMut(String),
{
    match build_run_output(args, execution, answers) {
        Ok(output) => Ok(Some(output)),
        Err(err) if interactive => {
            report_interactive_validation_error(&err, &mut report);
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn previous_interactive_question_index(
    questions: &[JsonValue],
    current: usize,
    answered: &JsonMap<String, JsonValue>,
    should_skip: fn(&str, &JsonMap<String, JsonValue>) -> bool,
) -> Option<usize> {
    if current == 0 {
        return None;
    }
    for idx in (0..current).rev() {
        let question_id = questions[idx]
            .get("id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if !should_skip(question_id, answered) {
            return Some(idx);
        }
    }
    None
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

fn report_interactive_validation_error<F>(err: &anyhow::Error, mut report: F)
where
    F: FnMut(String),
{
    report(tr("cli.wizard.result.qa_validation_error"));
    report(render_validation_error_detail(err));
}

fn render_validation_error_detail(err: &anyhow::Error) -> String {
    if let Some(validation) = err.downcast_ref::<ValidationError>() {
        return match validation {
            ValidationError::EmptyName => tr("cli.wizard.result.qa_value_required"),
            ValidationError::InvalidName(name) => {
                trf("cli.wizard.validation.component_name_invalid", &[name])
            }
            ValidationError::InvalidOperationName(name) => {
                trf("cli.wizard.error.operation_name_invalid", &[name])
            }
            ValidationError::InvalidFilesystemMode(mode) => {
                trf("cli.wizard.validation.filesystem_mode_invalid", &[mode])
            }
            ValidationError::InvalidFilesystemMount(mount) => {
                trf("cli.wizard.validation.filesystem_mount_invalid", &[mount])
            }
            ValidationError::InvalidTelemetryScope(scope) => {
                trf("cli.wizard.validation.telemetry_scope_invalid", &[scope])
            }
            ValidationError::InvalidSecretFormat(format) => {
                trf("cli.wizard.validation.secret_format_invalid", &[format])
            }
            ValidationError::InvalidTelemetryAttribute(attr) => {
                trf("cli.wizard.validation.telemetry_attribute_invalid", &[attr])
            }
            ValidationError::InvalidConfigField(field) => {
                trf("cli.wizard.validation.config_field_invalid", &[field])
            }
            ValidationError::InvalidConfigFieldName(name) => {
                trf("cli.wizard.validation.config_field_name_invalid", &[name])
            }
            ValidationError::InvalidConfigFieldType(kind) => {
                trf("cli.wizard.validation.config_field_type_invalid", &[kind])
            }
            ValidationError::TargetIsFile(path) => trf(
                "cli.wizard.validation.target_path_is_file",
                &[path.display().to_string().as_str()],
            ),
            ValidationError::TargetDirNotEmpty(path) => trf(
                "cli.wizard.error.target_dir_not_empty",
                &[path.display().to_string().as_str()],
            ),
            ValidationError::Io(path, source) => trf(
                "cli.wizard.validation.path_io",
                &[path.display().to_string().as_str(), &source.to_string()],
            ),
            _ => validation.to_string(),
        };
    }
    err.to_string()
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use serde_json::{Map as JsonMap, Value as JsonValue};

    use super::{
        ExecutionMode, RunMode, WizardArgs, WizardRunAnswers, build_output_with_recovery,
        create_questions, fallback_default_for_question, load_answers_with_recovery,
        parse_main_menu_selection, render_validation_error_detail,
        should_skip_create_advanced_question, wizard_answer_schema,
    };

    #[test]
    fn parse_main_menu_selection_supports_numeric_options() {
        assert_eq!(parse_main_menu_selection("1"), Some(RunMode::Create));
        assert_eq!(parse_main_menu_selection("2"), Some(RunMode::AddOperation));
        assert_eq!(
            parse_main_menu_selection("3"),
            Some(RunMode::UpdateOperation)
        );
        assert_eq!(parse_main_menu_selection("4"), Some(RunMode::BuildTest));
        assert_eq!(parse_main_menu_selection("5"), Some(RunMode::Doctor));
    }

    #[test]
    fn parse_main_menu_selection_supports_mode_aliases() {
        assert_eq!(parse_main_menu_selection("create"), Some(RunMode::Create));
        assert_eq!(
            parse_main_menu_selection("add_operation"),
            Some(RunMode::AddOperation)
        );
        assert_eq!(
            parse_main_menu_selection("update-operation"),
            Some(RunMode::UpdateOperation)
        );
        assert_eq!(
            parse_main_menu_selection("build_test"),
            Some(RunMode::BuildTest)
        );
        assert_eq!(
            parse_main_menu_selection("build-test"),
            Some(RunMode::BuildTest)
        );
        assert_eq!(parse_main_menu_selection("doctor"), Some(RunMode::Doctor));
    }

    #[test]
    fn parse_main_menu_selection_rejects_unknown_values() {
        assert_eq!(parse_main_menu_selection(""), None);
        assert_eq!(parse_main_menu_selection("6"), None);
        assert_eq!(parse_main_menu_selection("unknown"), None);
    }

    #[test]
    fn render_validation_error_detail_localizes_component_name_errors() {
        let message = render_validation_error_detail(
            &crate::scaffold::validate::ComponentName::parse("Bad Name")
                .expect_err("invalid component name")
                .into(),
        );
        assert_eq!(
            message,
            "component name must be lowercase kebab-or-snake case (got `Bad Name`)"
        );
    }

    #[test]
    fn interactive_answers_recovery_reports_malformed_answers_without_exiting() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let answers_path = temp.path().join("faulty-answers.json");
        std::fs::write(&answers_path, "{ this is not valid json").expect("write malformed");
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: ExecutionMode::Execute,
            dry_run: false,
            validate: false,
            apply: false,
            qa_answers: None,
            answers: Some(answers_path.clone()),
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

        let mut reported = Vec::new();
        let loaded = load_answers_with_recovery(Some(&answers_path), &args, true, |line| {
            reported.push(line);
        })
        .expect("interactive recovery should continue");

        assert!(
            loaded.is_none(),
            "malformed answers should fall back to interactive mode"
        );
        assert_eq!(
            reported.first().map(String::as_str),
            Some("wizard input failed validation; please correct and try again")
        );
        assert!(
            reported
                .iter()
                .any(|line| line.contains("must be valid JSON")),
            "expected specific parse failure in {reported:?}"
        );
    }

    #[test]
    fn interactive_build_recovery_reports_invalid_answer_values_without_exiting() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let mut fields = JsonMap::new();
        fields.insert(
            "component_name".to_string(),
            JsonValue::String("Bad Name".to_string()),
        );
        fields.insert(
            "output_dir".to_string(),
            JsonValue::String(temp.path().join("component").display().to_string()),
        );
        fields.insert(
            "abi_version".to_string(),
            JsonValue::String("0.6.0".to_string()),
        );
        let answers = WizardRunAnswers {
            schema: "component-wizard-run/v1".to_string(),
            mode: RunMode::Create,
            fields,
        };
        let args = WizardArgs {
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
        };

        let mut reported = Vec::new();
        let output = build_output_with_recovery(
            &args,
            ExecutionMode::Execute,
            Some(&answers),
            true,
            |line| {
                reported.push(line);
            },
        )
        .expect("interactive recovery should continue");

        assert!(
            output.is_none(),
            "invalid answers should return to wizard prompts"
        );
        assert_eq!(
            reported.first().map(String::as_str),
            Some("wizard input failed validation; please correct and try again")
        );
        assert!(
            reported.iter().any(|line| {
                line.contains("component name must be lowercase kebab-or-snake case")
            }),
            "expected translated validation detail in {reported:?}"
        );
    }

    #[test]
    fn interactive_build_recovery_reports_existing_i18n_errors_without_exiting() {
        let mut reported = Vec::new();
        super::report_interactive_validation_error(
            &anyhow!("unsupported answers mode `broken`"),
            |line| reported.push(line),
        );
        assert_eq!(
            reported,
            vec![
                "wizard input failed validation; please correct and try again".to_string(),
                "unsupported answers mode `broken`".to_string()
            ]
        );
    }

    #[test]
    fn create_questions_minimal_flow_only_asks_core_fields() {
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: super::ExecutionMode::Execute,
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
            project_root: std::path::PathBuf::from("."),
            template: None,
            full_tests: false,
            json: false,
        };

        let questions = create_questions(&args, false);
        let ids = questions
            .iter()
            .filter_map(|question| question.get("id").and_then(JsonValue::as_str))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["component_name", "output_dir", "advanced_setup"]);
    }

    #[test]
    fn wizard_answer_schema_matches_create_answer_document_shape() {
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: super::ExecutionMode::Execute,
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
            project_root: std::path::PathBuf::from("/tmp/demo"),
            template: None,
            full_tests: false,
            json: false,
        };

        let schema = wizard_answer_schema(&args);
        assert_eq!(
            schema.pointer("/required"),
            Some(&JsonValue::Array(vec![
                JsonValue::String("wizard_id".to_string()),
                JsonValue::String("schema_id".to_string()),
                JsonValue::String("schema_version".to_string()),
                JsonValue::String("answers".to_string()),
            ]))
        );
        assert_eq!(
            schema.pointer("/properties/answers/properties/mode/const"),
            Some(&JsonValue::String("create".to_string()))
        );
        assert_eq!(
            schema.pointer("/properties/answers/properties/fields/properties/component_name/type"),
            Some(&JsonValue::String("string".to_string()))
        );
        assert_eq!(
            schema.pointer("/properties/answers/properties/fields/properties/output_dir/type"),
            Some(&JsonValue::String("string".to_string()))
        );
        assert_eq!(
            schema.pointer("/properties/answers/properties/fields/properties/advanced_setup/type"),
            Some(&JsonValue::String("boolean".to_string()))
        );
        assert_eq!(
            schema.pointer("/properties/answers/properties/fields/properties/filesystem_mode/enum"),
            Some(&JsonValue::Array(vec![
                JsonValue::String("none".to_string()),
                JsonValue::String("read_only".to_string()),
                JsonValue::String("sandbox".to_string()),
            ]))
        );
    }

    #[test]
    fn create_flow_defaults_advanced_setup_to_false() {
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: super::ExecutionMode::Execute,
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
            project_root: std::path::PathBuf::from("/tmp/demo"),
            template: None,
            full_tests: false,
            json: false,
        };

        assert_eq!(
            fallback_default_for_question(&args, "advanced_setup", &serde_json::Map::new()),
            Some(JsonValue::Bool(false))
        );
        assert_eq!(
            fallback_default_for_question(&args, "secrets_enabled", &serde_json::Map::new()),
            Some(JsonValue::Bool(false))
        );
    }

    #[test]
    fn create_questions_advanced_flow_includes_secret_gate_before_secret_fields() {
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: super::ExecutionMode::Execute,
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
            project_root: std::path::PathBuf::from("."),
            template: None,
            full_tests: false,
            json: false,
        };

        let questions = create_questions(&args, true);
        let ids = questions
            .iter()
            .filter_map(|question| question.get("id").and_then(JsonValue::as_str))
            .collect::<Vec<_>>();
        let gate_index = ids.iter().position(|id| *id == "secrets_enabled").unwrap();
        let key_index = ids.iter().position(|id| *id == "secret_keys").unwrap();
        assert!(gate_index < key_index);
    }

    #[test]
    fn create_questions_advanced_flow_includes_messaging_and_events_fields() {
        let args = WizardArgs {
            mode: RunMode::Create,
            execution: super::ExecutionMode::Execute,
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
            project_root: std::path::PathBuf::from("."),
            template: None,
            full_tests: false,
            json: false,
        };

        let questions = create_questions(&args, true);
        let ids = questions
            .iter()
            .filter_map(|question| question.get("id").and_then(JsonValue::as_str))
            .collect::<Vec<_>>();
        assert!(ids.contains(&"messaging_inbound"));
        assert!(ids.contains(&"messaging_outbound"));
        assert!(ids.contains(&"events_inbound"));
        assert!(ids.contains(&"events_outbound"));
    }

    #[test]
    fn advanced_create_flow_skips_questions_answered_in_minimal_pass() {
        let mut answered = JsonMap::new();
        answered.insert(
            "component_name".to_string(),
            JsonValue::String("demo".to_string()),
        );
        answered.insert(
            "output_dir".to_string(),
            JsonValue::String("/tmp/demo".to_string()),
        );
        answered.insert("advanced_setup".to_string(), JsonValue::Bool(true));

        assert!(should_skip_create_advanced_question(
            "component_name",
            &answered
        ));
        assert!(should_skip_create_advanced_question(
            "output_dir",
            &answered
        ));
        assert!(should_skip_create_advanced_question(
            "advanced_setup",
            &answered
        ));
        assert!(!should_skip_create_advanced_question(
            "operation_names",
            &answered
        ));
    }

    #[test]
    fn advanced_create_flow_skips_filesystem_mounts_when_mode_is_none() {
        let mut answered = JsonMap::new();
        answered.insert(
            "filesystem_mode".to_string(),
            JsonValue::String("none".to_string()),
        );

        assert!(should_skip_create_advanced_question(
            "filesystem_mounts",
            &answered
        ));

        answered.insert(
            "filesystem_mode".to_string(),
            JsonValue::String("sandbox".to_string()),
        );

        assert!(!should_skip_create_advanced_question(
            "filesystem_mounts",
            &answered
        ));
    }
}
