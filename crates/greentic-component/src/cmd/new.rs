#![cfg(feature = "cli")]

use std::collections::HashSet;
use std::env;
use std::io::{Write, stdout};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Args;
use serde::Serialize;
use serde_json::json;

use crate::cmd::i18n;
use crate::cmd::post::{self, GitInitStatus, PostInitReport};
use crate::scaffold::config_schema::{ConfigSchemaInput, parse_config_field};
use crate::scaffold::deps::DependencyMode;
use crate::scaffold::engine::{
    DEFAULT_WIT_WORLD, ScaffoldEngine, ScaffoldOutcome, ScaffoldRequest,
};
use crate::scaffold::runtime_capabilities::{
    RuntimeCapabilitiesInput, parse_filesystem_mode, parse_filesystem_mount, parse_secret_format,
    parse_telemetry_attributes, parse_telemetry_scope,
};
use crate::scaffold::validate::{self, ComponentName, OrgNamespace, ValidationError};

type ValidationResult<T> = std::result::Result<T, ValidationError>;
const SKIP_GIT_ENV: &str = "GREENTIC_SKIP_GIT";

#[derive(Args, Debug, Clone)]
pub struct NewArgs {
    /// Name for the component (kebab-or-snake case)
    #[arg(long = "name", value_name = "kebab_or_snake", required = true)]
    pub name: String,
    /// Path to create the component (defaults to ./<name>)
    #[arg(long = "path", value_name = "dir")]
    pub path: Option<PathBuf>,
    /// Template identifier to scaffold from
    #[arg(
        long = "template",
        default_value = "rust-wasi-p2-min",
        value_name = "id"
    )]
    pub template: String,
    /// Reverse DNS-style organisation identifier
    #[arg(
        long = "org",
        default_value = "ai.greentic",
        value_name = "reverse.dns"
    )]
    pub org: String,
    /// Initial component version
    #[arg(long = "version", default_value = "0.1.0", value_name = "semver")]
    pub version: String,
    /// License to embed into generated sources
    #[arg(long = "license", default_value = "MIT", value_name = "id")]
    pub license: String,
    /// Exported WIT world name
    #[arg(
        long = "wit-world",
        default_value = DEFAULT_WIT_WORLD,
        value_name = "name"
    )]
    pub wit_world: String,
    /// User operations to scaffold into the canonical manifest (repeat or pass comma-separated values)
    #[arg(long = "operation", value_name = "name", value_delimiter = ',')]
    pub operation_names: Vec<String>,
    /// Default user operation written to `default_operation`
    #[arg(long = "default-operation", value_name = "name")]
    pub default_operation: Option<String>,
    /// Filesystem capability mode written to `capabilities.wasi.filesystem.mode`
    #[arg(long = "filesystem-mode", default_value = "none", value_name = "mode")]
    pub filesystem_mode: String,
    /// Filesystem mount written to `capabilities.wasi.filesystem.mounts` as `name:host_class:guest_path`
    #[arg(long = "filesystem-mount", value_name = "name:host_class:guest_path")]
    pub filesystem_mounts: Vec<String>,
    /// Enable `capabilities.host.http.client`
    #[arg(long = "http-client")]
    pub http_client: bool,
    /// Enable `capabilities.host.messaging.inbound`
    #[arg(long = "messaging-inbound")]
    pub messaging_inbound: bool,
    /// Enable `capabilities.host.messaging.outbound`
    #[arg(long = "messaging-outbound")]
    pub messaging_outbound: bool,
    /// Enable `capabilities.host.events.inbound`
    #[arg(long = "events-inbound")]
    pub events_inbound: bool,
    /// Enable `capabilities.host.events.outbound`
    #[arg(long = "events-outbound")]
    pub events_outbound: bool,
    /// Enable `capabilities.host.http.server`
    #[arg(long = "http-server")]
    pub http_server: bool,
    /// Enable `capabilities.host.state.read`
    #[arg(long = "state-read")]
    pub state_read: bool,
    /// Enable `capabilities.host.state.write`
    #[arg(long = "state-write")]
    pub state_write: bool,
    /// Enable `capabilities.host.state.delete`
    #[arg(long = "state-delete")]
    pub state_delete: bool,
    /// Telemetry permission scope for `capabilities.host.telemetry.scope`
    #[arg(long = "telemetry-scope", default_value = "node", value_name = "scope")]
    pub telemetry_scope: String,
    /// Top-level telemetry span prefix written to `telemetry.span_prefix`
    #[arg(long = "telemetry-span-prefix", value_name = "prefix")]
    pub telemetry_span_prefix: Option<String>,
    /// Top-level telemetry attribute written to `telemetry.attributes` as `key=value`
    #[arg(long = "telemetry-attribute", value_name = "key=value")]
    pub telemetry_attributes: Vec<String>,
    /// Secret key written to both `secret_requirements` and `capabilities.host.secrets.required`
    #[arg(long = "secret-key", value_name = "key")]
    pub secret_keys: Vec<String>,
    /// Shared secret env scope for scaffolded secret requirements
    #[arg(long = "secret-env", default_value = "dev", value_name = "env")]
    pub secret_env: String,
    /// Shared secret tenant scope for scaffolded secret requirements
    #[arg(
        long = "secret-tenant",
        default_value = "default",
        value_name = "tenant"
    )]
    pub secret_tenant: String,
    /// Shared secret format for scaffolded secret requirements
    #[arg(long = "secret-format", default_value = "text", value_name = "format")]
    pub secret_format: String,
    /// Config schema field as `name:type[:required|optional]`
    #[arg(long = "config-field", value_name = "name:type[:required|optional]")]
    pub config_fields: Vec<String>,
    /// Run without prompting for confirmation
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
    /// Skip the post-scaffold cargo check (hidden flag for testing/local dev)
    #[arg(long = "no-check", hide = true)]
    pub no_check: bool,
    /// Skip git initialization after scaffolding
    #[arg(long = "no-git")]
    pub no_git: bool,
    /// Emit JSON instead of human-readable output
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: NewArgs, engine: &ScaffoldEngine) -> Result<()> {
    let request = match build_request(&args) {
        Ok(req) => req,
        Err(err) => {
            emit_validation_failure(&err, args.json)?;
            return Err(err.into());
        }
    };
    if !args.json {
        println!("{}", i18n::tr_lit("scaffolding component..."));
        println!(
            "{}",
            i18n::tr_lit("- template: {} -> {}")
                .replacen("{}", &request.template_id, 1)
                .replacen("{}", &request.path.display().to_string(), 1)
        );
        println!(
            "{}",
            i18n::tr_lit("- wit world: {}").replacen("{}", &request.wit_world, 1)
        );
        stdout().flush().ok();
    }
    let scaffold_started = Instant::now();
    let outcome = engine.scaffold(request)?;
    if !args.json {
        println!(
            "{}",
            i18n::tr_lit("scaffolded files in {:.2?}")
                .replace("{:.2?}", &format!("{:.2?}", scaffold_started.elapsed()))
        );
        stdout().flush().ok();
    }
    let post_started = Instant::now();
    let skip_git = should_skip_git(&args);
    let post_init = post::run_post_init(&outcome, skip_git);
    if !args.json && !args.no_check {
        println!(
            "{}",
            i18n::tr_lit(
                "running cargo check --target wasm32-wasip2 (downloads toolchain on first run)...",
            )
        );
        stdout().flush().ok();
    }
    let compile_check = run_compile_check(&outcome.path, args.no_check)?;
    if args.json {
        let payload = NewCliOutput {
            scaffold: &outcome,
            compile_check: &compile_check,
            post_init: &post_init,
        };
        print_json(&payload)?;
    } else {
        print_human(&outcome, &compile_check, &post_init);
        println!(
            "{}",
            i18n::tr_lit("post-init + checks in {:.2?}")
                .replace("{:.2?}", &format!("{:.2?}", post_started.elapsed()))
        );
    }
    if compile_check.ran && !compile_check.passed {
        anyhow::bail!(
            "{}",
            i18n::tr_lit("cargo check --target wasm32-wasip2 failed")
        );
    }
    Ok(())
}

fn build_request(args: &NewArgs) -> ValidationResult<ScaffoldRequest> {
    let component_name = ComponentName::parse(&args.name)?;
    let org = OrgNamespace::parse(&args.org)?;
    let version = validate::normalize_version(&args.version)?;
    let target_path = resolve_path(&component_name, args.path.as_deref())?;
    Ok(ScaffoldRequest {
        name: component_name.into_string(),
        path: target_path,
        template_id: args.template.clone(),
        org: org.into_string(),
        version,
        license: args.license.clone(),
        wit_world: args.wit_world.clone(),
        user_operations: resolve_user_operations(args)?,
        default_operation: resolve_default_operation(args)?,
        runtime_capabilities: resolve_runtime_capabilities(args)?,
        config_schema: resolve_config_schema(args)?,
        non_interactive: args.non_interactive,
        year_override: None,
        dependency_mode: DependencyMode::from_env(),
    })
}

fn resolve_user_operations(args: &NewArgs) -> ValidationResult<Vec<String>> {
    if args.operation_names.is_empty() {
        return Ok(vec!["handle_message".to_string()]);
    }

    let mut user_operations = Vec::new();
    let mut seen = HashSet::new();
    for value in &args.operation_names {
        let normalized = validate::normalize_operation_name(value)?;
        if !seen.insert(normalized.clone()) {
            return Err(ValidationError::DuplicateOperationName(normalized));
        }
        user_operations.push(normalized);
    }
    Ok(user_operations)
}

fn resolve_default_operation(args: &NewArgs) -> ValidationResult<String> {
    let operations = resolve_user_operations(args)?;
    match args.default_operation.as_deref() {
        Some(value) => {
            let normalized = validate::normalize_operation_name(value)?;
            if operations.iter().any(|name| name == &normalized) {
                Ok(normalized)
            } else {
                Err(ValidationError::UnknownDefaultOperation(normalized))
            }
        }
        None => Ok(operations
            .first()
            .cloned()
            .unwrap_or_else(|| "handle_message".to_string())),
    }
}

fn resolve_runtime_capabilities(args: &NewArgs) -> ValidationResult<RuntimeCapabilitiesInput> {
    Ok(RuntimeCapabilitiesInput {
        filesystem_mode: parse_filesystem_mode(&args.filesystem_mode)?,
        filesystem_mounts: args
            .filesystem_mounts
            .iter()
            .map(|value| parse_filesystem_mount(value))
            .collect::<ValidationResult<Vec<_>>>()?,
        messaging_inbound: args.messaging_inbound,
        messaging_outbound: args.messaging_outbound,
        events_inbound: args.events_inbound,
        events_outbound: args.events_outbound,
        http_client: args.http_client,
        http_server: args.http_server,
        state_read: args.state_read,
        state_write: args.state_write,
        state_delete: args.state_delete,
        telemetry_scope: parse_telemetry_scope(&args.telemetry_scope)?,
        telemetry_span_prefix: args
            .telemetry_span_prefix
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        telemetry_attributes: parse_telemetry_attributes(&args.telemetry_attributes)?,
        secret_keys: args.secret_keys.clone(),
        secret_env: args.secret_env.trim().to_string(),
        secret_tenant: args.secret_tenant.trim().to_string(),
        secret_format: parse_secret_format(&args.secret_format)?,
    })
}

fn resolve_config_schema(args: &NewArgs) -> ValidationResult<ConfigSchemaInput> {
    Ok(ConfigSchemaInput {
        fields: args
            .config_fields
            .iter()
            .map(|value| parse_config_field(value))
            .collect::<ValidationResult<Vec<_>>>()?,
    })
}

fn resolve_path(name: &ComponentName, provided: Option<&Path>) -> ValidationResult<PathBuf> {
    let path = validate::resolve_target_path(name, provided)?;
    Ok(path)
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let mut handle = std::io::stdout();
    serde_json::to_writer_pretty(&mut handle, value)?;
    handle.write_all(b"\n").ok();
    Ok(())
}

fn print_human(outcome: &ScaffoldOutcome, check: &CompileCheckReport, post: &PostInitReport) {
    println!("{}", outcome.human_summary());
    print_template_metadata(outcome);
    for path in &outcome.created {
        println!("{}", i18n::tr_lit("- {path}").replace("{path}", path));
    }
    print_git_summary(&post.git);
    if !check.ran {
        println!(
            "{}",
            i18n::tr_lit("cargo check (wasm32-wasip2): skipped (--no-check)")
        );
    } else if check.passed {
        if let Some(ms) = check.duration_ms {
            println!(
                "{}",
                i18n::tr_lit("cargo check (wasm32-wasip2): ok ({:.2}s)")
                    .replace("{:.2}", &format!("{:.2}", ms as f64 / 1000.0))
            );
        } else {
            println!("{}", i18n::tr_lit("cargo check (wasm32-wasip2): ok"));
        }
    } else {
        println!(
            "{}",
            i18n::tr_lit("cargo check (wasm32-wasip2): FAILED (exit code {:?})")
                .replace("{:?}", &format!("{:?}", check.exit_code))
        );
        if let Some(stderr) = &check.stderr
            && !stderr.is_empty()
        {
            println!("{stderr}");
        }
    }
    if !post.next_steps.is_empty() {
        println!("{}", i18n::tr_lit("Next steps:"));
        for step in &post.next_steps {
            println!("{}", i18n::tr_lit("$ {step}").replace("{step}", step));
        }
    }
}

fn print_git_summary(report: &post::GitInitReport) {
    match report.status {
        GitInitStatus::Initialized => {
            if let Some(commit) = &report.commit {
                println!(
                    "{}",
                    i18n::tr_lit("git init: ok (commit {commit})").replace("{commit}", commit)
                );
            } else {
                println!("{}", i18n::tr_lit("git init: ok"));
            }
        }
        GitInitStatus::AlreadyPresent => {
            println!(
                "{}",
                i18n::tr_lit("git init: skipped ({})").replacen(
                    "{}",
                    report
                        .message
                        .as_deref()
                        .unwrap_or("directory already contains .git"),
                    1
                )
            );
        }
        GitInitStatus::InsideWorktree => {
            println!(
                "{}",
                i18n::tr_lit("git init: skipped ({})").replacen(
                    "{}",
                    report
                        .message
                        .as_deref()
                        .unwrap_or("already inside an existing git worktree"),
                    1
                )
            );
        }
        GitInitStatus::Skipped => {
            println!(
                "{}",
                i18n::tr_lit("git init: skipped ({})").replacen(
                    "{}",
                    report.message.as_deref().unwrap_or("not requested"),
                    1
                )
            );
        }
        GitInitStatus::Failed => {
            println!(
                "{}",
                i18n::tr_lit("git init: failed ({})").replacen(
                    "{}",
                    report
                        .message
                        .as_deref()
                        .unwrap_or("see logs for more details"),
                    1
                )
            );
        }
    }
}

fn print_template_metadata(outcome: &ScaffoldOutcome) {
    match &outcome.template_description {
        Some(desc) => println!(
            "{}",
            i18n::tr_lit("Template: {} — {desc}")
                .replacen("{}", &outcome.template, 1)
                .replace("{desc}", desc)
        ),
        None => println!(
            "{}",
            i18n::tr_lit("Template: {}").replacen("{}", &outcome.template, 1)
        ),
    }
    if !outcome.template_tags.is_empty() {
        println!(
            "{}",
            i18n::tr_lit("tags: {}").replacen("{}", &outcome.template_tags.join(", "), 1)
        );
    }
}

fn should_skip_git(args: &NewArgs) -> bool {
    if args.no_git {
        return true;
    }
    match env::var(SKIP_GIT_ENV) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
        Err(_) => false,
    }
}

fn run_compile_check(path: &Path, skip: bool) -> Result<CompileCheckReport> {
    const COMMAND_DISPLAY: &str = "cargo check --target wasm32-wasip2";
    if skip {
        return Ok(CompileCheckReport::skipped(COMMAND_DISPLAY));
    }
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.arg("check").arg("--target").arg("wasm32-wasip2");
    cmd.current_dir(path);
    let start = Instant::now();
    let output = cmd
        .output()
        .with_context(|| format!("failed to run `{COMMAND_DISPLAY}`"))?;
    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok(CompileCheckReport {
        command: COMMAND_DISPLAY.to_string(),
        ran: true,
        passed: output.status.success(),
        exit_code: output.status.code(),
        duration_ms: Some(duration_ms),
        stdout: if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        },
        stderr: if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        },
        reason: None,
    })
}

fn emit_validation_failure(err: &ValidationError, json: bool) -> Result<()> {
    if json {
        let payload = json!({
            "error": {
                "kind": "validation",
                "code": err.code(),
                "message": err.to_string()
            }
        });
        print_json(&payload)?;
        process::exit(1);
    }
    Ok(())
}

#[derive(Serialize)]
struct NewCliOutput<'a> {
    scaffold: &'a ScaffoldOutcome,
    compile_check: &'a CompileCheckReport,
    post_init: &'a PostInitReport,
}

#[derive(Debug, Serialize)]
struct CompileCheckReport {
    command: String,
    ran: bool,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl CompileCheckReport {
    fn skipped(command: &str) -> Self {
        Self {
            command: command.to_string(),
            ran: false,
            passed: true,
            exit_code: None,
            duration_ms: None,
            stdout: None,
            stderr: None,
            reason: Some("skipped (--no-check)".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_uses_name() {
        let args = NewArgs {
            name: "demo-component".into(),
            path: None,
            template: "rust-wasi-p2-min".into(),
            org: "ai.greentic".into(),
            version: "0.1.0".into(),
            license: "MIT".into(),
            wit_world: DEFAULT_WIT_WORLD.into(),
            operation_names: Vec::new(),
            default_operation: None,
            filesystem_mode: "none".into(),
            filesystem_mounts: Vec::new(),
            messaging_inbound: false,
            messaging_outbound: false,
            events_inbound: false,
            events_outbound: false,
            http_client: false,
            http_server: false,
            state_read: false,
            state_write: false,
            state_delete: false,
            telemetry_scope: "node".into(),
            telemetry_span_prefix: None,
            telemetry_attributes: Vec::new(),
            secret_keys: Vec::new(),
            secret_env: "dev".into(),
            secret_tenant: "default".into(),
            secret_format: "text".into(),
            config_fields: Vec::new(),
            non_interactive: false,
            no_check: false,
            no_git: false,
            json: false,
        };
        let request = build_request(&args).unwrap();
        assert!(request.path.ends_with("demo-component"));
        assert_eq!(request.user_operations, vec!["handle_message"]);
        assert_eq!(request.default_operation, "handle_message");
    }

    #[test]
    fn build_request_accepts_custom_operations() {
        let args = NewArgs {
            name: "demo-component".into(),
            path: None,
            template: "rust-wasi-p2-min".into(),
            org: "ai.greentic".into(),
            version: "0.1.0".into(),
            license: "MIT".into(),
            wit_world: DEFAULT_WIT_WORLD.into(),
            operation_names: vec!["render".into(), "sync-state".into()],
            default_operation: Some("sync-state".into()),
            filesystem_mode: "sandbox".into(),
            filesystem_mounts: vec!["cache:cache:/cache".into()],
            messaging_inbound: true,
            messaging_outbound: false,
            events_inbound: false,
            events_outbound: true,
            http_client: true,
            http_server: false,
            state_read: true,
            state_write: false,
            state_delete: false,
            telemetry_scope: "pack".into(),
            telemetry_span_prefix: Some("component.demo".into()),
            telemetry_attributes: vec!["component=demo".into()],
            secret_keys: vec!["API_TOKEN".into()],
            secret_env: "prod".into(),
            secret_tenant: "acme".into(),
            secret_format: "text".into(),
            config_fields: vec!["enabled:bool:required".into(), "api_key:string".into()],
            non_interactive: false,
            no_check: false,
            no_git: false,
            json: false,
        };
        let request = build_request(&args).unwrap();
        assert_eq!(request.user_operations, vec!["render", "sync-state"]);
        assert_eq!(request.default_operation, "sync-state");
        assert_eq!(request.runtime_capabilities.filesystem_mode, "sandbox");
        assert_eq!(request.runtime_capabilities.filesystem_mounts.len(), 1);
        assert!(request.runtime_capabilities.messaging_inbound);
        assert!(request.runtime_capabilities.events_outbound);
        assert!(request.runtime_capabilities.http_client);
        assert_eq!(request.runtime_capabilities.telemetry_scope, "pack");
        assert_eq!(request.runtime_capabilities.secret_keys, vec!["API_TOKEN"]);
        assert_eq!(request.config_schema.fields.len(), 2);
    }

    #[test]
    fn build_request_rejects_unknown_default_operation() {
        let args = NewArgs {
            name: "demo-component".into(),
            path: None,
            template: "rust-wasi-p2-min".into(),
            org: "ai.greentic".into(),
            version: "0.1.0".into(),
            license: "MIT".into(),
            wit_world: DEFAULT_WIT_WORLD.into(),
            operation_names: vec!["render".into()],
            default_operation: Some("sync-state".into()),
            filesystem_mode: "none".into(),
            filesystem_mounts: Vec::new(),
            messaging_inbound: false,
            messaging_outbound: false,
            events_inbound: false,
            events_outbound: false,
            http_client: false,
            http_server: false,
            state_read: false,
            state_write: false,
            state_delete: false,
            telemetry_scope: "node".into(),
            telemetry_span_prefix: None,
            telemetry_attributes: Vec::new(),
            secret_keys: Vec::new(),
            secret_env: "dev".into(),
            secret_tenant: "default".into(),
            secret_format: "text".into(),
            config_fields: Vec::new(),
            non_interactive: false,
            no_check: false,
            no_git: false,
            json: false,
        };
        let err = build_request(&args).unwrap_err();
        assert!(matches!(err, ValidationError::UnknownDefaultOperation(_)));
    }
}
