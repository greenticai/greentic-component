use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ciborium::Value as CborValue;
use greentic_types::cbor::canonical;
use greentic_types::i18n_text::I18nText;
use greentic_types::schemas::component::v0_6_0::{
    ChoiceOption, ComponentQaSpec, QaMode, Question, QuestionKind,
};
use serde::Serialize;
use serde_json::Map as JsonMap;
use serde_json::Value as JsonValue;

pub const PLAN_VERSION: u32 = 1;
pub const TEMPLATE_VERSION: &str = "component-scaffold-v0.6.0";
pub const GENERATOR_ID: &str = "greentic-component/wizard-provider";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WizardMode {
    Default,
    Setup,
    Update,
    Remove,
}

#[derive(Debug, Clone)]
pub struct AnswersPayload {
    pub json: String,
    pub cbor: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct WizardRequest {
    pub name: String,
    pub abi_version: String,
    pub mode: WizardMode,
    pub target: PathBuf,
    pub answers: Option<AnswersPayload>,
    pub required_capabilities: Vec<String>,
    pub provided_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub plan: WizardPlanEnvelope,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WizardPlanEnvelope {
    pub plan_version: u32,
    pub metadata: WizardPlanMetadata,
    pub target_root: PathBuf,
    pub plan: WizardPlan,
}

#[derive(Debug, Clone, Serialize)]
pub struct WizardPlanMetadata {
    pub generator: String,
    pub template_version: String,
    pub template_digest_blake3: String,
    pub requested_abi_version: String,
}

// Compat shim: keep deterministic plan JSON stable without requiring newer
// greentic-types exports during cargo package verification.
#[derive(Debug, Clone, Serialize)]
pub struct WizardPlan {
    pub meta: WizardPlanMeta,
    pub steps: Vec<WizardStep>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WizardPlanMeta {
    pub id: String,
    pub target: WizardTarget,
    pub mode: WizardPlanMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardTarget {
    Component,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardPlanMode {
    Scaffold,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WizardStep {
    EnsureDir { paths: Vec<String> },
    WriteFiles { files: BTreeMap<String, String> },
    RunCli { command: String },
    Delegate { id: String },
    BuildComponent { project_root: String },
    TestComponent { project_root: String, full: bool },
    Doctor { project_root: String },
}

pub fn spec_scaffold(mode: WizardMode) -> ComponentQaSpec {
    let title = match mode {
        WizardMode::Default => "wizard.component.default.title",
        WizardMode::Setup => "wizard.component.setup.title",
        WizardMode::Update => "wizard.component.update.title",
        WizardMode::Remove => "wizard.component.remove.title",
    };
    ComponentQaSpec {
        mode: qa_mode(mode),
        title: I18nText::new(title, None),
        description: Some(I18nText::new("wizard.component.description", None)),
        questions: vec![
            Question {
                id: "component.name".to_string(),
                label: I18nText::new("wizard.component.name.label", None),
                help: Some(I18nText::new("wizard.component.name.help", None)),
                error: None,
                kind: QuestionKind::Text,
                required: true,
                default: None,
            },
            Question {
                id: "component.path".to_string(),
                label: I18nText::new("wizard.component.path.label", None),
                help: Some(I18nText::new("wizard.component.path.help", None)),
                error: None,
                kind: QuestionKind::Text,
                required: false,
                default: None,
            },
            Question {
                id: "component.kind".to_string(),
                label: I18nText::new("wizard.component.kind.label", None),
                help: Some(I18nText::new("wizard.component.kind.help", None)),
                error: None,
                kind: QuestionKind::Choice {
                    options: vec![
                        ChoiceOption {
                            value: "tool".to_string(),
                            label: I18nText::new("wizard.component.kind.option.tool", None),
                        },
                        ChoiceOption {
                            value: "source".to_string(),
                            label: I18nText::new("wizard.component.kind.option.source", None),
                        },
                    ],
                },
                required: false,
                default: None,
            },
            Question {
                id: "component.features.enabled".to_string(),
                label: I18nText::new("wizard.component.features.enabled.label", None),
                help: Some(I18nText::new(
                    "wizard.component.features.enabled.help",
                    None,
                )),
                error: None,
                kind: QuestionKind::Bool,
                required: false,
                default: None,
            },
        ],
        defaults: BTreeMap::from([(
            "component.features.enabled".to_string(),
            CborValue::Bool(true),
        )]),
    }
}

pub fn apply_scaffold(request: WizardRequest, dry_run: bool) -> Result<ApplyResult> {
    let warnings = abi_warnings(&request.abi_version);
    let (prefill_answers_json, prefill_answers_cbor, mut mapping_warnings) =
        normalize_answers(request.answers, request.mode)?;
    let mut all_warnings = warnings;
    all_warnings.append(&mut mapping_warnings);
    let context = WizardContext {
        name: request.name,
        abi_version: request.abi_version.clone(),
        prefill_mode: request.mode,
        prefill_answers_cbor,
        prefill_answers_json,
    };

    let files = build_files(&context)?;
    let plan = build_plan(request.target, &request.abi_version, files);
    if !dry_run {
        execute_plan(&plan)?;
    }

    Ok(ApplyResult {
        plan,
        warnings: all_warnings,
    })
}

pub fn execute_plan(envelope: &WizardPlanEnvelope) -> Result<()> {
    for step in &envelope.plan.steps {
        match step {
            WizardStep::EnsureDir { paths } => {
                for path in paths {
                    let dir = envelope.target_root.join(path);
                    fs::create_dir_all(&dir).with_context(|| {
                        format!("wizard: failed to create directory {}", dir.display())
                    })?;
                }
            }
            WizardStep::WriteFiles { files } => {
                for (relative_path, content) in files {
                    let target = envelope.target_root.join(relative_path);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("wizard: failed to create directory {}", parent.display())
                        })?;
                    }
                    let bytes = decode_step_content(relative_path, content)?;
                    fs::write(&target, bytes)
                        .with_context(|| format!("wizard: failed to write {}", target.display()))?;
                    #[cfg(unix)]
                    if is_executable_heuristic(Path::new(relative_path)) {
                        use std::os::unix::fs::PermissionsExt;
                        let mut permissions = fs::metadata(&target)
                            .with_context(|| {
                                format!("wizard: failed to stat {}", target.display())
                            })?
                            .permissions();
                        permissions.set_mode(0o755);
                        fs::set_permissions(&target, permissions).with_context(|| {
                            format!("wizard: failed to set executable bit {}", target.display())
                        })?;
                    }
                }
            }
            WizardStep::RunCli { command, .. } => {
                bail!("wizard: unsupported plan step run_cli ({command})")
            }
            WizardStep::Delegate { id, .. } => {
                bail!("wizard: unsupported plan step delegate ({})", id.as_str())
            }
            WizardStep::BuildComponent { project_root } => {
                bail!("wizard: unsupported plan step build_component ({project_root})")
            }
            WizardStep::TestComponent { project_root, .. } => {
                bail!("wizard: unsupported plan step test_component ({project_root})")
            }
            WizardStep::Doctor { project_root } => {
                bail!("wizard: unsupported plan step doctor ({project_root})")
            }
        }
    }
    Ok(())
}

fn is_executable_heuristic(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("sh" | "bash" | "zsh" | "ps1")
    ) || path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "Makefile")
        .unwrap_or(false)
}

pub fn load_answers_payload(path: &Path) -> Result<AnswersPayload> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("wizard: failed to open answers file {}", path.display()))?;
    let value: JsonValue = serde_json::from_str(&json)
        .with_context(|| format!("wizard: answers file {} is not valid JSON", path.display()))?;
    let cbor = canonical::to_canonical_cbor_allow_floats(&value)
        .map_err(|err| anyhow!("wizard: failed to encode answers as CBOR: {err}"))?;
    Ok(AnswersPayload { json, cbor })
}

struct WizardContext {
    name: String,
    abi_version: String,
    prefill_mode: WizardMode,
    prefill_answers_cbor: Option<Vec<u8>>,
    prefill_answers_json: Option<String>,
}

type NormalizedAnswers = (Option<String>, Option<Vec<u8>>, Vec<String>);

fn normalize_answers(
    answers: Option<AnswersPayload>,
    mode: WizardMode,
) -> Result<NormalizedAnswers> {
    let warnings = Vec::new();
    let Some(payload) = answers else {
        return Ok((None, None, warnings));
    };
    let mut value: JsonValue = serde_json::from_str(&payload.json).with_context(|| {
        "wizard: answers JSON payload should be valid after initial parse".to_string()
    })?;
    let JsonValue::Object(mut root) = value else {
        return Ok((Some(payload.json), Some(payload.cbor), warnings));
    };

    let enabled = extract_bool(&root, &["component.features.enabled", "enabled"]);
    if let Some(flag) = enabled {
        root.insert("enabled".to_string(), JsonValue::Bool(flag));
    } else if matches!(
        mode,
        WizardMode::Default | WizardMode::Setup | WizardMode::Update
    ) {
        root.insert("enabled".to_string(), JsonValue::Bool(true));
    }

    value = JsonValue::Object(root);
    let json = serde_json::to_string_pretty(&value)?;
    let cbor = canonical::to_canonical_cbor_allow_floats(&value)
        .map_err(|err| anyhow!("wizard: failed to encode normalized answers as CBOR: {err}"))?;
    Ok((Some(json), Some(cbor), warnings))
}

fn extract_bool(root: &JsonMap<String, JsonValue>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = root.get(*key)
            && let Some(flag) = value.as_bool()
        {
            return Some(flag);
        }
        if let Some(flag) = nested_bool(root, key) {
            return Some(flag);
        }
    }
    None
}

fn nested_bool(root: &JsonMap<String, JsonValue>, dotted: &str) -> Option<bool> {
    nested_value(root, dotted).and_then(|value| value.as_bool())
}

fn nested_value<'a>(root: &'a JsonMap<String, JsonValue>, dotted: &str) -> Option<&'a JsonValue> {
    let mut parts = dotted.split('.');
    let first = parts.next()?;
    let mut current = root.get(first)?;
    for segment in parts {
        let JsonValue::Object(map) = current else {
            return None;
        };
        current = map.get(segment)?;
    }
    Some(current)
}

struct GeneratedFile {
    path: PathBuf,
    contents: Vec<u8>,
}

fn build_files(context: &WizardContext) -> Result<Vec<GeneratedFile>> {
    let mut files = vec![
        text_file("Cargo.toml", render_cargo_toml(context)),
        text_file("rust-toolchain.toml", render_rust_toolchain_toml()),
        text_file("README.md", render_readme(context)),
        text_file("component.manifest.json", render_manifest_json(context)),
        text_file("Makefile", render_makefile()),
        text_file("build.rs", render_build_rs()),
        text_file("src/lib.rs", render_lib_rs(context)),
        text_file("src/qa.rs", render_qa_rs()),
        text_file("src/i18n.rs", render_i18n_rs()),
        text_file("src/i18n_bundle.rs", render_i18n_bundle_rs()),
        text_file("assets/i18n/en.json", render_i18n_bundle()),
        text_file("assets/i18n/locales.json", render_i18n_locales_json()),
        text_file("tools/i18n.sh", render_i18n_sh()),
    ];

    if let (Some(json), Some(cbor)) = (
        context.prefill_answers_json.as_ref(),
        context.prefill_answers_cbor.as_ref(),
    ) {
        let mode = match context.prefill_mode {
            WizardMode::Default => "default",
            WizardMode::Setup => "setup",
            WizardMode::Update => "update",
            WizardMode::Remove => "remove",
        };
        files.push(text_file(
            &format!("examples/{mode}.answers.json"),
            json.clone(),
        ));
        files.push(binary_file(
            &format!("examples/{mode}.answers.cbor"),
            cbor.clone(),
        ));
    }

    Ok(files)
}

fn build_plan(target: PathBuf, abi_version: &str, files: Vec<GeneratedFile>) -> WizardPlanEnvelope {
    let mut dirs = BTreeSet::new();
    for file in &files {
        if let Some(parent) = file.path.parent()
            && !parent.as_os_str().is_empty()
        {
            dirs.insert(parent.to_path_buf());
        }
    }
    let mut steps: Vec<WizardStep> = Vec::new();
    if !dirs.is_empty() {
        let paths = dirs
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        steps.push(WizardStep::EnsureDir { paths });
    }

    let mut file_map = BTreeMap::new();
    for file in &files {
        let key = file.path.to_string_lossy().into_owned();
        file_map.insert(key, encode_step_content(&file.path, &file.contents));
    }
    if !file_map.is_empty() {
        steps.push(WizardStep::WriteFiles { files: file_map });
    }

    let plan = WizardPlan {
        meta: WizardPlanMeta {
            id: "greentic.component.scaffold".to_string(),
            target: WizardTarget::Component,
            mode: WizardPlanMode::Scaffold,
        },
        steps,
    };
    let metadata = WizardPlanMetadata {
        generator: GENERATOR_ID.to_string(),
        template_version: TEMPLATE_VERSION.to_string(),
        template_digest_blake3: template_digest_hex(&files),
        requested_abi_version: abi_version.to_string(),
    };
    WizardPlanEnvelope {
        plan_version: PLAN_VERSION,
        metadata,
        target_root: target,
        plan,
    }
}

const STEP_BASE64_PREFIX: &str = "base64:";

fn encode_step_content(path: &Path, bytes: &[u8]) -> String {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "cbor")
    {
        format!(
            "{STEP_BASE64_PREFIX}{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
        )
    } else {
        String::from_utf8(bytes.to_vec()).unwrap_or_default()
    }
}

fn decode_step_content(relative_path: &str, content: &str) -> Result<Vec<u8>> {
    if relative_path.ends_with(".cbor") && content.starts_with(STEP_BASE64_PREFIX) {
        let raw = content.trim_start_matches(STEP_BASE64_PREFIX);
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw)
            .map_err(|err| anyhow!("wizard: invalid base64 content for {relative_path}: {err}"))?;
        return Ok(decoded);
    }
    Ok(content.as_bytes().to_vec())
}

fn template_digest_hex(files: &[GeneratedFile]) -> String {
    let mut hasher = blake3::Hasher::new();
    for file in files {
        hasher.update(file.path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(&file.contents);
        hasher.update(&[0xff]);
    }
    hasher.finalize().to_hex().to_string()
}

fn abi_warnings(abi_version: &str) -> Vec<String> {
    if abi_version == "0.6.0" {
        Vec::new()
    } else {
        vec![format!(
            "wizard: warning: only component@0.6.0 template is generated (requested {abi_version})"
        )]
    }
}

fn qa_mode(mode: WizardMode) -> QaMode {
    match mode {
        WizardMode::Default => QaMode::Default,
        WizardMode::Setup => QaMode::Setup,
        WizardMode::Update => QaMode::Update,
        WizardMode::Remove => QaMode::Remove,
    }
}

fn render_rust_toolchain_toml() -> String {
    r#"[toolchain]
channel = "1.91.0"
components = ["clippy", "rustfmt"]
targets = ["wasm32-wasip2", "x86_64-unknown-linux-gnu"]
profile = "minimal"
"#
    .to_string()
}

fn text_file(path: &str, contents: String) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        contents: contents.into_bytes(),
    }
}

fn binary_file(path: &str, contents: Vec<u8>) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        contents,
    }
}

fn render_cargo_toml(context: &WizardContext) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
license = "MIT"
rust-version = "1.91"
description = "Greentic component {name}"
build = "build.rs"

[lib]
crate-type = ["cdylib", "rlib"]

[package.metadata.greentic]
abi_version = "{abi_version}"

[package.metadata.component]
package = "greentic:component"

[package.metadata.component.target]
world = "greentic:component/component@0.6.0"

[dependencies]
greentic-types = "0.4"
greentic-interfaces-guest = {{ version = "0.4", default-features = false, features = ["component-v0-6"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[build-dependencies]
greentic-types = "0.4"
serde_json = "1"
"#,
        name = context.name,
        abi_version = context.abi_version
    )
}

fn render_readme(context: &WizardContext) -> String {
    format!(
        r#"# {name}

Generated by `greentic-component wizard` for component@0.6.0.

## Next steps
- Extend QA flows in `src/qa.rs` and i18n keys in `src/i18n.rs`.
- Generate/update locales via `./tools/i18n.sh`.
- Rebuild to embed translations: `cargo build`.

## QA ops
- `qa-spec`: emits setup/update/remove semantics and accepts `default|setup|install|update|upgrade|remove`.
- `apply-answers`: returns base response shape `{{ ok, config?, warnings, errors }}`.
- `i18n-keys`: returns i18n keys used by QA/setup messaging.

## ABI version
Requested ABI version: {abi_version}

Note: the wizard currently emits a fixed 0.6.0 template.
"#,
        name = context.name,
        abi_version = context.abi_version
    )
}

fn render_makefile() -> String {
    r#"SHELL := /bin/sh

NAME := $(shell awk 'BEGIN{in_pkg=0} /^\[package\]/{in_pkg=1; next} /^\[/{in_pkg=0} in_pkg && /^name = / {gsub(/"/ , "", $$3); print $$3; exit}' Cargo.toml)
NAME_UNDERSCORE := $(subst -,_,$(NAME))
ABI_VERSION := $(shell awk 'BEGIN{in_meta=0} /^\[package.metadata.greentic\]/{in_meta=1; next} /^\[/{in_meta=0} in_meta && /^abi_version = / {gsub(/"/ , "", $$3); print $$3; exit}' Cargo.toml)
ABI_VERSION_UNDERSCORE := $(subst .,_,$(ABI_VERSION))
DIST_DIR := dist
WASM_OUT := $(DIST_DIR)/$(NAME)__$(ABI_VERSION_UNDERSCORE).wasm

.PHONY: build test fmt clippy wasm doctor

build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

wasm:
	if ! cargo component --version >/dev/null 2>&1; then \
		echo "cargo-component is required to produce a valid component@0.6.0 wasm"; \
		echo "install with: cargo install cargo-component --locked"; \
		exit 1; \
	fi
	RUSTFLAGS= CARGO_ENCODED_RUSTFLAGS= cargo component build --release --target wasm32-wasip2
	WASM_SRC=""; \
	for cand in \
		"$${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/$(NAME_UNDERSCORE).wasm" \
		"$${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/$(NAME).wasm" \
		"$${CARGO_TARGET_DIR:-target}/wasm32-wasip1/release/$(NAME_UNDERSCORE).wasm" \
		"$${CARGO_TARGET_DIR:-target}/wasm32-wasip1/release/$(NAME).wasm" \
		"target/wasm32-wasip2/release/$(NAME_UNDERSCORE).wasm" \
		"target/wasm32-wasip2/release/$(NAME).wasm" \
		"target/wasm32-wasip1/release/$(NAME_UNDERSCORE).wasm" \
		"target/wasm32-wasip1/release/$(NAME).wasm"; do \
		if [ -f "$$cand" ]; then WASM_SRC="$$cand"; break; fi; \
	done; \
	if [ -z "$$WASM_SRC" ]; then \
		echo "unable to locate wasm build artifact for $(NAME)"; \
		exit 1; \
	fi; \
	mkdir -p $(DIST_DIR); \
	cp "$$WASM_SRC" $(WASM_OUT)

doctor:
	greentic-component doctor $(WASM_OUT)
"#
    .to_string()
}

fn render_manifest_json(context: &WizardContext) -> String {
    let name_snake = context.name.replace('-', "_");
    format!(
        r#"{{
  "$schema": "https://greenticai.github.io/greentic-component/schemas/v1/component.manifest.schema.json",
  "id": "com.example.{name}",
  "name": "{name}",
  "version": "0.1.0",
  "world": "greentic:component/component@0.6.0",
  "describe_export": "describe",
  "operations": [
    {{
      "name": "handle_message",
      "input_schema": {{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "{name} handle input",
        "type": "object",
        "required": ["input"],
        "properties": {{
          "input": {{
            "type": "string",
            "default": "Hello from {name}!"
          }}
        }},
        "additionalProperties": false
      }},
      "output_schema": {{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "{name} handle output",
        "type": "object",
        "required": ["message"],
        "properties": {{
          "message": {{
            "type": "string"
          }}
        }},
        "additionalProperties": false
      }}
    }},
    {{
      "name": "qa-spec",
      "input_schema": {{
        "type": "object",
        "properties": {{
          "mode": {{ "type": "string" }}
        }},
        "additionalProperties": true
      }},
      "output_schema": {{
        "type": "object",
        "additionalProperties": true
      }}
    }},
    {{
      "name": "apply-answers",
      "input_schema": {{
        "type": "object",
        "properties": {{
          "mode": {{ "type": "string" }},
          "current_config": {{ "type": "object" }},
          "answers": {{ "type": "object" }}
        }},
        "additionalProperties": true
      }},
      "output_schema": {{
        "type": "object",
        "required": ["ok", "warnings", "errors"],
        "properties": {{
          "ok": {{ "type": "boolean" }},
          "warnings": {{ "type": "array" }},
          "errors": {{ "type": "array" }},
          "config": {{ "type": "object" }}
        }},
        "additionalProperties": true
      }}
    }},
    {{
      "name": "i18n-keys",
      "input_schema": {{
        "type": "object",
        "additionalProperties": true
      }},
      "output_schema": {{
        "type": "array",
        "items": {{ "type": "string" }}
      }}
    }}
  ],
  "default_operation": "handle_message",
  "config_schema": {{
    "type": "object",
    "required": [],
    "properties": {{}},
    "additionalProperties": false
  }},
  "supports": ["messaging"],
  "profiles": {{
    "default": "stateless",
    "supported": ["stateless"]
  }},
  "secret_requirements": [],
  "capabilities": {{
    "wasi": {{
      "filesystem": {{
        "mode": "none",
        "mounts": []
      }},
      "random": true,
      "clocks": true
    }},
    "host": {{
      "messaging": {{
        "inbound": true,
        "outbound": true
      }},
      "telemetry": {{
        "scope": "node"
      }},
      "secrets": {{
        "required": []
      }}
    }}
  }},
  "limits": {{
    "memory_mb": 128,
    "wall_time_ms": 1000
  }},
  "artifacts": {{
    "component_wasm": "target/wasm32-wasip2/release/{name_snake}.wasm"
  }},
  "hashes": {{
    "component_wasm": "blake3:0000000000000000000000000000000000000000000000000000000000000000"
  }},
  "dev_flows": {{
    "default": {{
      "format": "flow-ir-json",
      "graph": {{
        "nodes": [
          {{ "id": "start", "type": "start" }},
          {{ "id": "end", "type": "end" }}
        ],
        "edges": [
          {{ "from": "start", "to": "end" }}
        ]
      }}
    }}
  }}
}}
"#,
        name = context.name,
        name_snake = name_snake
    )
}

fn render_lib_rs(context: &WizardContext) -> String {
    format!(
        r#"#[cfg(target_arch = "wasm32")]
use std::collections::BTreeMap;

#[cfg(target_arch = "wasm32")]
use greentic_interfaces_guest::component_v0_6::node;
#[cfg(target_arch = "wasm32")]
use greentic_types::cbor::canonical;
#[cfg(target_arch = "wasm32")]
use greentic_types::schemas::common::schema_ir::{{AdditionalProperties, SchemaIr}};
#[cfg(target_arch = "wasm32")]
use greentic_types::schemas::component::v0_6_0::{{ComponentInfo, I18nText}};

// i18n: runtime lookup + embedded CBOR bundle helpers.
pub mod i18n;
pub mod i18n_bundle;
// qa: mode normalization, QA spec generation, apply-answers validation.
pub mod qa;

const COMPONENT_NAME: &str = "{name}";
const COMPONENT_ORG: &str = "com.example";
const COMPONENT_VERSION: &str = "0.1.0";

#[cfg(target_arch = "wasm32")]
#[used]
#[unsafe(link_section = ".greentic.wasi")]
static WASI_TARGET_MARKER: [u8; 13] = *b"wasm32-wasip2";

#[cfg(target_arch = "wasm32")]
struct Component;

#[cfg(target_arch = "wasm32")]
impl node::Guest for Component {{
    // Component metadata advertised to host/operator tooling.
    // Extend here when you add more operations or capability declarations.
    fn describe() -> node::ComponentDescriptor {{
        let input_schema_cbor = input_schema_cbor();
        let output_schema_cbor = output_schema_cbor();
        node::ComponentDescriptor {{
            name: COMPONENT_NAME.to_string(),
            version: COMPONENT_VERSION.to_string(),
            summary: Some(format!("Greentic component {{COMPONENT_NAME}}")),
            capabilities: Vec::new(),
            ops: vec![
                node::Op {{
                    name: "handle_message".to_string(),
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
                }},
                node::Op {{
                    name: "qa-spec".to_string(),
                    summary: Some("Return QA spec for requested mode".to_string()),
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
                }},
                node::Op {{
                    name: "apply-answers".to_string(),
                    summary: Some("Apply QA answers and optionally return config override".to_string()),
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
                }},
                node::Op {{
                    name: "i18n-keys".to_string(),
                    summary: Some("Return i18n keys referenced by QA/setup".to_string()),
                    input: node::IoSchema {{
                        schema: node::SchemaSource::InlineCbor(input_schema_cbor.clone()),
                        content_type: "application/cbor".to_string(),
                        schema_version: None,
                    }},
                    output: node::IoSchema {{
                        schema: node::SchemaSource::InlineCbor(output_schema_cbor),
                        content_type: "application/cbor".to_string(),
                        schema_version: None,
                    }},
                    examples: Vec::new(),
                }},
            ],
            schemas: Vec::new(),
            setup: None,
        }}
    }}

    // Single ABI entrypoint. Keep this dispatcher model intact.
    // Extend behavior by adding/adjusting operation branches in `run_component_cbor`.
    fn invoke(
        operation: String,
        envelope: node::InvocationEnvelope,
    ) -> Result<node::InvocationResult, node::NodeError> {{
        let output = run_component_cbor(&operation, envelope.payload_cbor);
        Ok(node::InvocationResult {{
            ok: true,
            output_cbor: output,
            output_metadata_cbor: None,
        }})
    }}
}}

#[cfg(target_arch = "wasm32")]
greentic_interfaces_guest::export_component_v060!(Component);

// Default user-operation implementation.
// Replace this with domain logic for your component.
pub fn handle_message(operation: &str, input: &str) -> String {{
    format!("{{COMPONENT_NAME}}::{{operation}} => {{}}", input.trim())
}}

#[cfg(target_arch = "wasm32")]
fn encode_cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {{
    canonical::to_canonical_cbor_allow_floats(value).expect("encode cbor")
}}

#[cfg(target_arch = "wasm32")]
// Accept canonical CBOR first, then fall back to JSON for local debugging.
fn parse_payload(input: &[u8]) -> serde_json::Value {{
    if let Ok(value) = canonical::from_cbor(input) {{
        return value;
    }}
    serde_json::from_slice(input).unwrap_or_else(|_| serde_json::json!({{}}))
}}

#[cfg(target_arch = "wasm32")]
// Keep ingress compatibility: default/setup/install -> setup, update/upgrade -> update.
fn normalized_mode(payload: &serde_json::Value) -> qa::NormalizedMode {{
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("operation").and_then(|v| v.as_str()))
        .unwrap_or("setup");
    qa::normalize_mode(mode).unwrap_or(qa::NormalizedMode::Setup)
}}

#[cfg(target_arch = "wasm32")]
// Minimal schema for generic operation input.
// Extend these schemas when you harden operation contracts.
fn input_schema() -> SchemaIr {{
    SchemaIr::Object {{
        properties: BTreeMap::from([(
            "input".to_string(),
            SchemaIr::String {{
                min_len: Some(0),
                max_len: None,
                regex: None,
                format: None,
            }},
        )]),
        required: vec!["input".to_string()],
        additional: AdditionalProperties::Allow,
    }}
}}

#[cfg(target_arch = "wasm32")]
fn output_schema() -> SchemaIr {{
    SchemaIr::Object {{
        properties: BTreeMap::from([(
            "message".to_string(),
            SchemaIr::String {{
                min_len: Some(0),
                max_len: None,
                regex: None,
                format: None,
            }},
        )]),
        required: vec!["message".to_string()],
        additional: AdditionalProperties::Allow,
    }}
}}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn config_schema() -> SchemaIr {{
    SchemaIr::Object {{
        properties: BTreeMap::new(),
        required: Vec::new(),
        additional: AdditionalProperties::Forbid,
    }}
}}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn component_info() -> ComponentInfo {{
    ComponentInfo {{
        id: format!("{{COMPONENT_ORG}}.{{COMPONENT_NAME}}"),
        version: COMPONENT_VERSION.to_string(),
        role: "tool".to_string(),
        display_name: Some(I18nText::new("component.display_name", Some(COMPONENT_NAME.to_string()))),
    }}
}}

#[cfg(target_arch = "wasm32")]
fn input_schema_cbor() -> Vec<u8> {{
    encode_cbor(&input_schema())
}}

#[cfg(target_arch = "wasm32")]
fn output_schema_cbor() -> Vec<u8> {{
    encode_cbor(&output_schema())
}}

#[cfg(target_arch = "wasm32")]
// Central operation dispatcher.
// This is the primary extension point for new operations.
fn run_component_cbor(operation: &str, input: Vec<u8>) -> Vec<u8> {{
    let value = parse_payload(&input);
    let output = match operation {{
        "qa-spec" => {{
            let mode = normalized_mode(&value);
            qa::qa_spec_json(mode)
        }}
        "apply-answers" => {{
            let mode = normalized_mode(&value);
            qa::apply_answers(mode, &value)
        }}
        "i18n-keys" => serde_json::Value::Array(
            qa::i18n_keys()
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
        _ => {{
            let op_name = value
                .get("operation")
                .and_then(|v| v.as_str())
                .unwrap_or(operation);
            let input_text = value
                .get("input")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| value.to_string());
            serde_json::json!({{
                "message": handle_message(op_name, &input_text)
            }})
        }}
    }};
    encode_cbor(&output)
}}
"#,
        name = context.name
    )
}

fn render_qa_rs() -> String {
    r#"use greentic_types::i18n_text::I18nText;
use greentic_types::schemas::component::v0_6_0::{QaMode, Question, QuestionKind};
use serde_json::{json, Value as JsonValue};

// Internal normalized lifecycle semantics used by scaffolded QA operations.
// Input compatibility accepts legacy/provision aliases via `normalize_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedMode {
    Setup,
    Update,
    Remove,
}

impl NormalizedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Update => "update",
            Self::Remove => "remove",
        }
    }
}

// Compatibility mapping for mode strings from operator/flow payloads.
pub fn normalize_mode(raw: &str) -> Option<NormalizedMode> {
    match raw {
        "default" | "setup" | "install" => Some(NormalizedMode::Setup),
        "update" | "upgrade" => Some(NormalizedMode::Update),
        "remove" => Some(NormalizedMode::Remove),
        _ => None,
    }
}

// Primary QA authoring entrypoint.
// Extend question sets here for your real setup/update/remove requirements.
pub fn qa_spec_json(mode: NormalizedMode) -> JsonValue {
    let (title_key, description_key, questions) = match mode {
        NormalizedMode::Setup => (
            "qa.install.title",
            Some("qa.install.description"),
            vec![
                question("api_key", "qa.field.api_key.label", "qa.field.api_key.help", true),
                question("region", "qa.field.region.label", "qa.field.region.help", true),
                question(
                    "webhook_base_url",
                    "qa.field.webhook_base_url.label",
                    "qa.field.webhook_base_url.help",
                    true,
                ),
                question("enabled", "qa.field.enabled.label", "qa.field.enabled.help", false),
            ],
        ),
        NormalizedMode::Update => (
            "qa.update.title",
            Some("qa.update.description"),
            vec![
                question("api_key", "qa.field.api_key.label", "qa.field.api_key.help", false),
                question("region", "qa.field.region.label", "qa.field.region.help", false),
                question(
                    "webhook_base_url",
                    "qa.field.webhook_base_url.label",
                    "qa.field.webhook_base_url.help",
                    false,
                ),
                question("enabled", "qa.field.enabled.label", "qa.field.enabled.help", false),
            ],
        ),
        NormalizedMode::Remove => (
            "qa.remove.title",
            Some("qa.remove.description"),
            vec![question(
                "confirm_remove",
                "qa.field.confirm_remove.label",
                "qa.field.confirm_remove.help",
                true,
            )],
        ),
    };

    json!({
        "mode": match mode {
            NormalizedMode::Setup => QaMode::Setup,
            NormalizedMode::Update => QaMode::Update,
            NormalizedMode::Remove => QaMode::Remove,
        },
        "title": I18nText::new(title_key, None),
        "description": description_key.map(|key| I18nText::new(key, None)),
        "questions": questions,
        "defaults": {}
    })
}

fn question(id: &str, label_key: &str, help_key: &str, required: bool) -> Question {
    Question {
        id: id.to_string(),
        label: I18nText::new(label_key, None),
        help: Some(I18nText::new(help_key, None)),
        error: None,
        kind: QuestionKind::Text,
        required,
        default: None,
    }
}

// Used by `i18n-keys` operation and contract checks in operator.
pub fn i18n_keys() -> Vec<String> {
    crate::i18n::all_keys()
}

// Apply answers and return operator-friendly base shape:
// { ok, config?, warnings, errors, ...optional metadata }
// Extend this method for domain validation rules and config patching.
pub fn apply_answers(mode: NormalizedMode, payload: &JsonValue) -> JsonValue {
    let answers = payload.get("answers").cloned().unwrap_or_else(|| json!({}));
    let current_config = payload
        .get("current_config")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let mut errors = Vec::new();
    match mode {
        NormalizedMode::Setup => {
            for key in ["api_key", "region", "webhook_base_url"] {
                if answers.get(key).and_then(|v| v.as_str()).is_none() {
                    errors.push(json!({
                        "key": "qa.error.required",
                        "msg_key": "qa.error.required",
                        "fields": [key]
                    }));
                }
            }
        }
        NormalizedMode::Remove => {
            if answers
                .get("confirm_remove")
                .and_then(|v| v.as_str())
                .map(|v| v != "true")
                .unwrap_or(true)
            {
                errors.push(json!({
                    "key": "qa.error.remove_confirmation",
                    "msg_key": "qa.error.remove_confirmation",
                    "fields": ["confirm_remove"]
                }));
            }
        }
        NormalizedMode::Update => {}
    }

    if !errors.is_empty() {
        return json!({
            "ok": false,
            "warnings": [],
            "errors": errors,
            "meta": {
                "mode": mode.as_str(),
                "version": "v1"
            }
        });
    }

    let mut config = match current_config {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let JsonValue::Object(map) = answers {
        for (key, value) in map {
            config.insert(key, value);
        }
    }
    if mode == NormalizedMode::Remove {
        config.insert("enabled".to_string(), JsonValue::Bool(false));
    }

    json!({
        "ok": true,
        "config": config,
        "warnings": [],
        "errors": [],
        "meta": {
            "mode": mode.as_str(),
            "version": "v1"
        },
        "audit": {
            "reasons": ["qa.apply_answers"],
            "timings_ms": {}
        }
    })
}
"#
    .to_string()
}

#[allow(dead_code)]
fn render_descriptor_rs(context: &WizardContext) -> String {
    let _ = context;
    String::new()
}

#[allow(dead_code)]
fn render_capability_list(capabilities: &[String]) -> String {
    let _ = capabilities;
    "&[]".to_string()
}

#[allow(dead_code)]
fn render_schema_rs() -> String {
    r#"use std::collections::BTreeMap;

use greentic_types::cbor::canonical;
use greentic_types::schemas::common::schema_ir::{AdditionalProperties, SchemaIr};

pub fn input_schema() -> SchemaIr {
    object_schema(vec![(
        "message",
        SchemaIr::String {
            min_len: Some(1),
            max_len: Some(1024),
            regex: None,
            format: None,
        },
    )])
}

pub fn output_schema() -> SchemaIr {
    object_schema(vec![(
        "result",
        SchemaIr::String {
            min_len: Some(1),
            max_len: Some(1024),
            regex: None,
            format: None,
        },
    )])
}

pub fn config_schema() -> SchemaIr {
    object_schema(vec![("enabled", SchemaIr::Bool)])
}

pub fn input_schema_cbor() -> Vec<u8> {
    canonical::to_canonical_cbor_allow_floats(&input_schema()).unwrap_or_default()
}

pub fn output_schema_cbor() -> Vec<u8> {
    canonical::to_canonical_cbor_allow_floats(&output_schema()).unwrap_or_default()
}

pub fn config_schema_cbor() -> Vec<u8> {
    canonical::to_canonical_cbor_allow_floats(&config_schema()).unwrap_or_default()
}

fn object_schema(props: Vec<(&str, SchemaIr)>) -> SchemaIr {
    let mut properties = BTreeMap::new();
    let mut required = Vec::new();
    for (name, schema) in props {
        properties.insert(name.to_string(), schema);
        required.push(name.to_string());
    }
    SchemaIr::Object {
        properties,
        required,
        additional: AdditionalProperties::Forbid,
    }
}
"#
    .to_string()
}

#[allow(dead_code)]
fn render_runtime_rs() -> String {
    r#"use std::collections::BTreeMap;

use greentic_types::cbor::canonical;
use serde_json::Value as JsonValue;

pub fn run(input: Vec<u8>, state: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    let input_map = decode_map(&input);
    let message = input_map
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("ok");
    let mut output = BTreeMap::new();
    output.insert(
        "result".to_string(),
        JsonValue::String(format!("processed: {message}")),
    );
    let output_cbor = canonical::to_canonical_cbor_allow_floats(&output).unwrap_or_default();
    let state_cbor = canonicalize_or_empty(&state);
    (output_cbor, state_cbor)
}

fn canonicalize_or_empty(bytes: &[u8]) -> Vec<u8> {
    let empty = || {
        canonical::to_canonical_cbor_allow_floats(&BTreeMap::<String, JsonValue>::new())
            .unwrap_or_default()
    };
    if bytes.is_empty() {
        return empty();
    }
    let value: JsonValue = match canonical::from_cbor(bytes) {
        Ok(value) => value,
        Err(_) => return empty(),
    };
    canonical::to_canonical_cbor_allow_floats(&value).unwrap_or_default()
}

fn decode_map(bytes: &[u8]) -> BTreeMap<String, JsonValue> {
    if bytes.is_empty() {
        return BTreeMap::new();
    }
    let value: JsonValue = match canonical::from_cbor(bytes) {
        Ok(value) => value,
        Err(_) => return BTreeMap::new(),
    };
    let JsonValue::Object(map) = value else {
        return BTreeMap::new();
    };
    map.into_iter().collect()
}
"#
    .to_string()
}

fn render_i18n_rs() -> String {
    r#"use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::i18n_bundle::{unpack_locales_from_cbor, LocaleBundle};

// Generated by build.rs: static embedded CBOR translation bundle.
include!(concat!(env!("OUT_DIR"), "/i18n_bundle.rs"));

// Decode once for process lifetime.
static I18N_BUNDLE: OnceLock<LocaleBundle> = OnceLock::new();

fn bundle() -> &'static LocaleBundle {
    I18N_BUNDLE.get_or_init(|| unpack_locales_from_cbor(I18N_BUNDLE_CBOR).unwrap_or_default())
}

// Fallback precedence is deterministic:
// exact locale -> base language -> en
fn locale_chain(locale: &str) -> Vec<String> {
    let normalized = locale.replace('_', "-");
    let mut chain = vec![normalized.clone()];
    if let Some((base, _)) = normalized.split_once('-') {
        chain.push(base.to_string());
    }
    chain.push("en".to_string());
    chain
}

// Translation lookup function used throughout generated QA/setup code.
// Extend by adding pluralization/context handling if your component needs it.
pub fn t(locale: &str, key: &str) -> String {
    for candidate in locale_chain(locale) {
        if let Some(map) = bundle().get(&candidate)
            && let Some(value) = map.get(key)
        {
            return value.clone();
        }
    }
    key.to_string()
}

// Returns canonical source key list (from `en`).
pub fn all_keys() -> Vec<String> {
    let Some(en) = bundle().get("en") else {
        return Vec::new();
    };
    en.keys().cloned().collect()
}

// Returns English dictionary for diagnostics/tests/tools.
pub fn en_messages() -> BTreeMap<String, String> {
    bundle().get("en").cloned().unwrap_or_default()
}
"#
    .to_string()
}

fn render_i18n_bundle() -> String {
    r#"{
  "qa.install.title": "Install configuration",
  "qa.install.description": "Provide values for initial provider setup.",
  "qa.update.title": "Update configuration",
  "qa.update.description": "Adjust existing provider settings.",
  "qa.remove.title": "Remove configuration",
  "qa.remove.description": "Confirm provider removal settings.",
  "qa.field.api_key.label": "API key",
  "qa.field.api_key.help": "Secret key used to authenticate provider requests.",
  "qa.field.region.label": "Region",
  "qa.field.region.help": "Region identifier for the provider account.",
  "qa.field.webhook_base_url.label": "Webhook base URL",
  "qa.field.webhook_base_url.help": "Public base URL used for webhook callbacks.",
  "qa.field.enabled.label": "Enable provider",
  "qa.field.enabled.help": "Enable this provider after setup completes.",
  "qa.field.confirm_remove.label": "Confirm removal",
  "qa.field.confirm_remove.help": "Set to true to allow provider removal.",
  "qa.error.required": "One or more required fields are missing.",
  "qa.error.remove_confirmation": "Removal requires explicit confirmation."
}
"#
    .to_string()
}

fn render_i18n_locales_json() -> String {
    r#"["ar","ar-AE","ar-DZ","ar-EG","ar-IQ","ar-MA","ar-SA","ar-SD","ar-SY","ar-TN","ay","bg","bn","cs","da","de","el","en-GB","es","et","fa","fi","fr","fr-FR","gn","gu","hi","hr","ht","hu","id","it","ja","km","kn","ko","lo","lt","lv","ml","mr","ms","my","nah","ne","nl","nl-NL","no","pa","pl","pt","qu","ro","ru","si","sk","sr","sv","ta","te","th","tl","tr","uk","ur","vi","zh"]
"#
    .to_string()
}

fn render_i18n_bundle_rs() -> String {
    r#"use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use greentic_types::cbor::canonical;

// Locale -> (key -> translated message)
pub type LocaleBundle = BTreeMap<String, BTreeMap<String, String>>;

// Reads `assets/i18n/*.json` locale maps and returns stable BTreeMap ordering.
// Extend here if you need stricter file validation rules.
pub fn load_locale_files(dir: &Path) -> Result<LocaleBundle, String> {
    let mut locales = LocaleBundle::new();
    if !dir.exists() {
        return Ok(locales);
    }
    for entry in fs::read_dir(dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        // locales.json is metadata, not a translation dictionary.
        if stem == "locales" {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let map: BTreeMap<String, String> = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
        locales.insert(stem.to_string(), map);
    }
    Ok(locales)
}

pub fn pack_locales_to_cbor(locales: &LocaleBundle) -> Result<Vec<u8>, String> {
    canonical::to_canonical_cbor_allow_floats(locales).map_err(|err| err.to_string())
}

#[allow(dead_code)]
// Runtime decode helper used by src/i18n.rs.
pub fn unpack_locales_from_cbor(bytes: &[u8]) -> Result<LocaleBundle, String> {
    canonical::from_cbor(bytes).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip_contains_en() {
        let mut locales = LocaleBundle::new();
        let mut en = BTreeMap::new();
        en.insert("qa.install.title".to_string(), "Install".to_string());
        locales.insert("en".to_string(), en);

        let cbor = pack_locales_to_cbor(&locales).expect("pack locales");
        let decoded = unpack_locales_from_cbor(&cbor).expect("decode locales");
        assert!(decoded.contains_key("en"));
    }
}
"#
    .to_string()
}

fn render_build_rs() -> String {
    r#"#[path = "src/i18n_bundle.rs"]
mod i18n_bundle;

use std::env;
use std::fs;
use std::path::Path;

// Build-time embedding pipeline:
// 1) Read assets/i18n/*.json
// 2) Pack canonical CBOR bundle
// 3) Emit OUT_DIR constants included by src/i18n.rs
fn main() {
    let i18n_dir = Path::new("assets/i18n");
    println!("cargo:rerun-if-changed={}", i18n_dir.display());

    let locales = i18n_bundle::load_locale_files(i18n_dir)
        .unwrap_or_else(|err| panic!("failed to load locale files: {err}"));
    let bundle = i18n_bundle::pack_locales_to_cbor(&locales)
        .unwrap_or_else(|err| panic!("failed to pack locale bundle: {err}"));

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let bundle_path = Path::new(&out_dir).join("i18n.bundle.cbor");
    fs::write(&bundle_path, bundle).expect("write i18n.bundle.cbor");

    let rs_path = Path::new(&out_dir).join("i18n_bundle.rs");
    fs::write(
        &rs_path,
        "pub const I18N_BUNDLE_CBOR: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/i18n.bundle.cbor\"));\n",
    )
    .expect("write i18n_bundle.rs");
}
"#
    .to_string()
}

fn render_i18n_sh() -> String {
    r#"#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCALES_FILE="$ROOT_DIR/assets/i18n/locales.json"
SOURCE_FILE="$ROOT_DIR/assets/i18n/en.json"

log() {
  printf '[i18n] %s\n' "$*"
}

fail() {
  printf '[i18n] error: %s\n' "$*" >&2
  exit 1
}

ensure_codex() {
  if command -v codex >/dev/null 2>&1; then
    return
  fi
  if command -v npm >/dev/null 2>&1; then
    log "installing Codex CLI via npm"
    npm i -g @openai/codex || fail "failed to install Codex CLI via npm"
  elif command -v brew >/dev/null 2>&1; then
    log "installing Codex CLI via brew"
    brew install codex || fail "failed to install Codex CLI via brew"
  else
    fail "Codex CLI not found and no supported installer available (npm or brew)"
  fi
}

ensure_codex_login() {
  if codex login status >/dev/null 2>&1; then
    return
  fi
  log "Codex login status unavailable or not logged in; starting login flow"
  codex login || fail "Codex login failed"
}

probe_translator() {
  command -v greentic-i18n-translator >/dev/null 2>&1 || fail "greentic-i18n-translator not found. Install it and rerun this script."
  local help_output
  help_output="$(greentic-i18n-translator --help 2>&1 || true)"
  [[ -n "$help_output" ]] || fail "unable to inspect greentic-i18n-translator --help"
  if ! greentic-i18n-translator translate --help >/dev/null 2>&1; then
    fail "translator subcommand 'translate' is required but unavailable"
  fi
}

run_translate() {
  while IFS= read -r locale; do
    [[ -n "$locale" ]] || continue
    log "translating locale: $locale"
    greentic-i18n-translator translate \
      --langs "$locale" \
      --en "$SOURCE_FILE" || fail "translate failed for locale $locale"
  done < <(python3 - "$LOCALES_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = json.load(f)
for locale in data:
    if locale != "en":
        print(locale)
PY
)
}

run_validate_per_locale() {
  local failed=0
  while IFS= read -r locale; do
    [[ -n "$locale" ]] || continue
    if ! greentic-i18n-translator validate --langs "$locale" --en "$SOURCE_FILE"; then
      log "validate failed for locale: $locale"
      failed=1
    fi
  done < <(python3 - "$LOCALES_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = json.load(f)
for locale in data:
    if locale != "en":
        print(locale)
PY
)
  return "$failed"
}

run_status_per_locale() {
  local failed=0
  while IFS= read -r locale; do
    [[ -n "$locale" ]] || continue
    if ! greentic-i18n-translator status --langs "$locale" --en "$SOURCE_FILE"; then
      log "status failed for locale: $locale"
      failed=1
    fi
  done < <(python3 - "$LOCALES_FILE" <<'PY'
import json
import sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = json.load(f)
for locale in data:
    if locale != "en":
        print(locale)
PY
)
  return "$failed"
}

run_optional_checks() {
  if greentic-i18n-translator validate --help >/dev/null 2>&1; then
    log "running translator validate"
    if ! run_validate_per_locale; then
      fail "translator validate failed"
    fi
  else
    log "warning: translator validate command not available; skipping"
  fi
  if greentic-i18n-translator status --help >/dev/null 2>&1; then
    log "running translator status"
    run_status_per_locale || fail "translator status failed"
  else
    log "warning: translator status command not available; skipping"
  fi
}

[[ -f "$LOCALES_FILE" ]] || fail "missing locales file: $LOCALES_FILE"
[[ -f "$SOURCE_FILE" ]] || fail "missing source locale file: $SOURCE_FILE"

ensure_codex
ensure_codex_login
probe_translator
run_translate
run_optional_checks
log "translations updated. Run cargo build to embed translations into WASM"
"#
    .to_string()
}

#[allow(dead_code)]
fn bytes_literal(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "&[]".to_string();
    }
    let rendered = bytes
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("&[{rendered}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_answers_cbor() {
        let json = serde_json::json!({"b": 1, "a": 2});
        let cbor = canonical::to_canonical_cbor_allow_floats(&json).unwrap();
        assert!(!cbor.is_empty());
    }
}

