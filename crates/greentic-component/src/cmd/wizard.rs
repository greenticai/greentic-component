#![cfg(feature = "cli")]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand, ValueEnum};
use greentic_types::cbor::canonical;
use serde_json::Value as JsonValue;

use crate::scaffold::validate::{
    ComponentName, ValidationError, ensure_path_available, normalize_version,
};

#[derive(Subcommand, Debug, Clone)]
pub enum WizardCommand {
    /// Generate a component@0.6.0 template scaffold
    New(WizardNewArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WizardNewArgs {
    /// Component name (kebab-or-snake case)
    #[arg(value_name = "name")]
    pub name: String,
    /// ABI version to target (template is fixed to 0.6.0 for now)
    #[arg(long = "abi-version", default_value = "0.6.0", value_name = "semver")]
    pub abi_version: String,
    /// QA mode to prefill when --answers is provided
    #[arg(long = "mode", value_enum, default_value = "default")]
    pub mode: WizardMode,
    /// Answers JSON to prefill QA setup
    #[arg(long = "answers", value_name = "answers.json")]
    pub answers: Option<PathBuf>,
    /// Output directory (template will be created under <out>/<name>)
    #[arg(long = "out", value_name = "dir")]
    pub out: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardMode {
    Default,
    Setup,
    Upgrade,
    Remove,
}

pub fn run(command: WizardCommand) -> Result<()> {
    match command {
        WizardCommand::New(args) => run_new(args),
    }
}

fn run_new(args: WizardNewArgs) -> Result<()> {
    let name = ComponentName::parse(&args.name)?;
    let abi_version = normalize_version(&args.abi_version)?;
    let target = resolve_out_path(&name, args.out.as_deref())?;
    ensure_path_available(&target)?;

    if abi_version != "0.6.0" {
        eprintln!(
            "wizard: warning: only component@0.6.0 template is generated (requested {})",
            abi_version
        );
    }

    let answers = match args.answers.as_ref() {
        Some(path) => Some(load_answers_payload(path)?),
        None => None,
    };

    let context = WizardContext {
        name: name.into_string(),
        abi_version,
        prefill_mode: args.mode,
        prefill_answers_cbor: answers.as_ref().map(|payload| payload.cbor.clone()),
        prefill_answers_json: answers.map(|payload| payload.json),
    };

    write_template(&target, &context)?;

    println!("wizard: created {}", target.display());
    Ok(())
}

fn resolve_out_path(
    name: &ComponentName,
    out: Option<&Path>,
) -> std::result::Result<PathBuf, ValidationError> {
    if let Some(out) = out {
        let base = if out.is_absolute() {
            out.to_path_buf()
        } else {
            env::current_dir()
                .map_err(ValidationError::WorkingDir)?
                .join(out)
        };
        Ok(base.join(name.as_str()))
    } else {
        crate::scaffold::validate::resolve_target_path(name, None)
    }
}

struct AnswersPayload {
    json: String,
    cbor: Vec<u8>,
}

fn load_answers_payload(path: &Path) -> Result<AnswersPayload> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("wizard: failed to open answers file {}", path.display()))?;
    let value: JsonValue = serde_json::from_str(&json)
        .with_context(|| format!("wizard: answers file {} is not valid JSON", path.display()))?;
    let cbor = canonical::to_canonical_cbor_allow_floats(&value)
        .map_err(|err| anyhow!("wizard: failed to encode answers as CBOR: {err}"))?;
    Ok(AnswersPayload { json, cbor })
}

#[derive(Debug, Clone)]
struct WizardContext {
    name: String,
    abi_version: String,
    prefill_mode: WizardMode,
    prefill_answers_cbor: Option<Vec<u8>>,
    prefill_answers_json: Option<String>,
}

#[derive(Debug, Clone)]
struct GeneratedFile {
    path: PathBuf,
    contents: Vec<u8>,
}

fn write_template(path: &Path, context: &WizardContext) -> Result<()> {
    let files = build_files(context)?;
    for file in files {
        let target = path.join(&file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("wizard: failed to create directory {}", parent.display())
            })?;
        }
        fs::write(&target, &file.contents)
            .with_context(|| format!("wizard: failed to write {}", target.display()))?;
    }
    Ok(())
}

fn build_files(context: &WizardContext) -> Result<Vec<GeneratedFile>> {
    let mut files = vec![
        text_file("Cargo.toml", render_cargo_toml(context)),
        text_file("README.md", render_readme(context)),
        text_file("Makefile", render_makefile()),
        text_file("src/lib.rs", render_lib_rs()),
        text_file("src/descriptor.rs", render_descriptor_rs(context)),
        text_file("src/schema.rs", render_schema_rs()),
        text_file("src/runtime.rs", render_runtime_rs()),
        text_file("src/qa.rs", render_qa_rs(context)),
        text_file("src/i18n.rs", render_i18n_rs()),
        text_file("wit/package.wit", render_wit_package()),
        text_file("assets/i18n/en.json", render_i18n_bundle()),
    ];

    if let (Some(json), Some(cbor)) = (
        context.prefill_answers_json.as_ref(),
        context.prefill_answers_cbor.as_ref(),
    ) {
        let mode = match context.prefill_mode {
            WizardMode::Default => "default",
            WizardMode::Setup => "setup",
            WizardMode::Upgrade => "upgrade",
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
rust-version = "1.90"
description = "Greentic component {name}"

[lib]
crate-type = ["cdylib", "rlib"]

[package.metadata.greentic]
abi_version = "{abi_version}"

[package.metadata.component]
package = "greentic:component"

[package.metadata.component.target]
path = "wit"
world = "component-v0-v6-v0"

[dependencies]
greentic-types = "0.4"
wit-bindgen = "0.53"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        name = context.name,
        abi_version = context.abi_version
    )
}

fn render_readme(context: &WizardContext) -> String {
    format!(
        r#"# {name}

Generated by `greentic-component wizard new` for component@0.6.0.

## Next steps
- Update `wit/package.wit` if you need custom interfaces.
- Refine schemas in `src/schema.rs`.
- Implement runtime logic in `src/runtime.rs`.
- Extend QA flows in `src/qa.rs` and i18n keys in `src/i18n.rs`.

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

fn render_lib_rs() -> String {
    r#"wit_bindgen::generate!({
    path: "wit",
    world: "component-v0-v6-v0",
});

mod descriptor;
mod schema;
mod runtime;
mod qa;
mod i18n;

#[cfg(target_arch = "wasm32")]
#[used]
#[unsafe(link_section = ".greentic.wasi")]
static WASI_TARGET_MARKER: [u8; 13] = *b"wasm32-wasip2";

struct Component;

impl exports::greentic::component::component_descriptor::Guest for Component {
    fn get_component_info() -> Vec<u8> {
        descriptor::info_cbor()
    }

    fn describe() -> Vec<u8> {
        descriptor::describe_cbor()
    }
}

impl exports::greentic::component::component_schema::Guest for Component {
    fn input_schema() -> Vec<u8> {
        schema::input_schema_cbor()
    }

    fn output_schema() -> Vec<u8> {
        schema::output_schema_cbor()
    }

    fn config_schema() -> Vec<u8> {
        schema::config_schema_cbor()
    }
}

impl exports::greentic::component::component_qa::Guest for Component {
    fn qa_spec(mode: exports::greentic::component::component_qa::QaMode) -> Vec<u8> {
        qa::qa_spec_cbor(qa::Mode::from(mode))
    }

    fn apply_answers(
        mode: exports::greentic::component::component_qa::QaMode,
        current_config: Vec<u8>,
        answers: Vec<u8>,
    ) -> Vec<u8> {
        qa::apply_answers(qa::Mode::from(mode), current_config, answers)
    }
}

impl exports::greentic::component::component_i18n::Guest for Component {
    fn i18n_keys() -> Vec<String> {
        i18n::all_keys()
    }
}

impl exports::greentic::component::component_runtime::Guest for Component {
    fn run(
        input: Vec<u8>,
        state: Vec<u8>,
    ) -> exports::greentic::component::component_runtime::RunResult {
        let (output, new_state) = runtime::run(input, state);
        exports::greentic::component::component_runtime::RunResult { output, new_state }
    }
}

export!(Component);
"#
    .to_string()
}

fn render_qa_rs(context: &WizardContext) -> String {
    let (default_prefill, setup_prefill, upgrade_prefill, remove_prefill) =
        match context.prefill_answers_cbor.as_ref() {
            Some(bytes) if context.prefill_mode == WizardMode::Default => (
                bytes_literal(bytes),
                "&[]".to_string(),
                "&[]".to_string(),
                "&[]".to_string(),
            ),
            Some(bytes) if context.prefill_mode == WizardMode::Setup => (
                "&[]".to_string(),
                bytes_literal(bytes),
                "&[]".to_string(),
                "&[]".to_string(),
            ),
            Some(bytes) if context.prefill_mode == WizardMode::Upgrade => (
                "&[]".to_string(),
                "&[]".to_string(),
                bytes_literal(bytes),
                "&[]".to_string(),
            ),
            Some(bytes) if context.prefill_mode == WizardMode::Remove => (
                "&[]".to_string(),
                "&[]".to_string(),
                "&[]".to_string(),
                bytes_literal(bytes),
            ),
            _ => (
                "&[]".to_string(),
                "&[]".to_string(),
                "&[]".to_string(),
                "&[]".to_string(),
            ),
        };

    let template = r#"use std::collections::BTreeMap;

use greentic_types::cbor::canonical;
use greentic_types::i18n_text::I18nText;
use greentic_types::schemas::component::v0_6_0::{ComponentQaSpec, QaMode, Question, QuestionKind};
use serde_json::Value as JsonValue;

const DEFAULT_PREFILLED_ANSWERS_CBOR: &[u8] = __DEFAULT_PREFILL__;
const SETUP_PREFILLED_ANSWERS_CBOR: &[u8] = __SETUP_PREFILL__;
const UPGRADE_PREFILLED_ANSWERS_CBOR: &[u8] = __UPGRADE_PREFILL__;
const REMOVE_PREFILLED_ANSWERS_CBOR: &[u8] = __REMOVE_PREFILL__;

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Default,
    Setup,
    Upgrade,
    Remove,
}

impl From<crate::exports::greentic::component::component_qa::QaMode> for Mode {
    fn from(mode: crate::exports::greentic::component::component_qa::QaMode) -> Self {
        match mode {
            crate::exports::greentic::component::component_qa::QaMode::Default => Mode::Default,
            crate::exports::greentic::component::component_qa::QaMode::Setup => Mode::Setup,
            crate::exports::greentic::component::component_qa::QaMode::Upgrade => Mode::Upgrade,
            crate::exports::greentic::component::component_qa::QaMode::Remove => Mode::Remove,
        }
    }
}

pub fn qa_spec_cbor(mode: Mode) -> Vec<u8> {
    let spec = qa_spec(mode);
    canonical::to_canonical_cbor_allow_floats(&spec).unwrap_or_default()
}

pub fn prefilled_answers_cbor(mode: Mode) -> &'static [u8] {
    match mode {
        Mode::Default => DEFAULT_PREFILLED_ANSWERS_CBOR,
        Mode::Setup => SETUP_PREFILLED_ANSWERS_CBOR,
        Mode::Upgrade => UPGRADE_PREFILLED_ANSWERS_CBOR,
        Mode::Remove => REMOVE_PREFILLED_ANSWERS_CBOR,
    }
}

pub fn apply_answers(mode: Mode, current_config: Vec<u8>, answers: Vec<u8>) -> Vec<u8> {
    let mut config = decode_map(&current_config);
    let updates = decode_map(&answers);
    match mode {
        Mode::Default | Mode::Setup | Mode::Upgrade => {
            for (key, value) in updates {
                config.insert(key, value);
            }
        }
        Mode::Remove => {
            config.clear();
            config.insert("enabled".to_string(), JsonValue::Bool(false));
        }
    }
    canonical::to_canonical_cbor_allow_floats(&config).unwrap_or_default()
}

fn qa_spec(mode: Mode) -> ComponentQaSpec {
    let (title_key, description_key, questions) = match mode {
        Mode::Default => (
            "qa.default.title",
            Some("qa.default.description"),
            vec![question_enabled("qa.default.enabled.label", "qa.default.enabled.help")],
        ),
        Mode::Setup => (
            "qa.setup.title",
            Some("qa.setup.description"),
            vec![question_enabled("qa.setup.enabled.label", "qa.setup.enabled.help")],
        ),
        Mode::Upgrade => ("qa.upgrade.title", None, Vec::new()),
        Mode::Remove => ("qa.remove.title", None, Vec::new()),
    };
    ComponentQaSpec {
        mode: match mode {
            Mode::Default => QaMode::Default,
            Mode::Setup => QaMode::Setup,
            Mode::Upgrade => QaMode::Upgrade,
            Mode::Remove => QaMode::Remove,
        },
        title: I18nText::new(title_key, None),
        description: description_key.map(|key| I18nText::new(key, None)),
        questions,
        defaults: BTreeMap::new(),
    }
}

fn question_enabled(label_key: &str, help_key: &str) -> Question {
    Question {
        id: "enabled".to_string(),
        label: I18nText::new(label_key, None),
        help: Some(I18nText::new(help_key, None)),
        error: None,
        kind: QuestionKind::Bool,
        required: true,
        default: None,
    }
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
"#;
    template
        .replace("__DEFAULT_PREFILL__", &default_prefill)
        .replace("__SETUP_PREFILL__", &setup_prefill)
        .replace("__UPGRADE_PREFILL__", &upgrade_prefill)
        .replace("__REMOVE_PREFILL__", &remove_prefill)
}

fn render_descriptor_rs(context: &WizardContext) -> String {
    let template = r#"use std::collections::BTreeMap;

use greentic_types::cbor::canonical;
use greentic_types::schemas::component::v0_6_0::{
    ComponentDescribe, ComponentInfo, ComponentOperation, ComponentRunInput, ComponentRunOutput,
    RedactionRule, RedactionKind, schema_hash,
};

use crate::schema;

pub fn info() -> ComponentInfo {
    ComponentInfo {
        id: "com.example.__NAME__".to_string(),
        version: "0.1.0".to_string(),
        role: "tool".to_string(),
        display_name: None,
    }
}

pub fn info_cbor() -> Vec<u8> {
    canonical::to_canonical_cbor_allow_floats(&info()).unwrap_or_default()
}

pub fn describe() -> ComponentDescribe {
    let input_schema = schema::input_schema();
    let output_schema = schema::output_schema();
    let config_schema = schema::config_schema();
    let op_hash = schema_hash(&input_schema, &output_schema, &config_schema)
        .expect("schema hash");
    let operation = ComponentOperation {
        id: "run".to_string(),
        display_name: None,
        input: ComponentRunInput { schema: input_schema },
        output: ComponentRunOutput { schema: output_schema },
        defaults: BTreeMap::new(),
        redactions: vec![RedactionRule {
            json_pointer: "/secret".to_string(),
            kind: RedactionKind::Secret,
        }],
        constraints: BTreeMap::new(),
        schema_hash: op_hash,
    };
    ComponentDescribe {
        info: info(),
        provided_capabilities: Vec::new(),
        required_capabilities: Vec::new(),
        metadata: BTreeMap::new(),
        operations: vec![operation],
        config_schema,
    }
}

pub fn describe_cbor() -> Vec<u8> {
    canonical::to_canonical_cbor_allow_floats(&describe()).unwrap_or_default()
}
"#;
    template.replace("__NAME__", &context.name)
}

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
    r#"pub const I18N_KEYS: &[&str] = &[
    "qa.default.title",
    "qa.default.description",
    "qa.default.enabled.label",
    "qa.default.enabled.help",
    "qa.setup.title",
    "qa.setup.description",
    "qa.setup.enabled.label",
    "qa.setup.enabled.help",
    "qa.upgrade.title",
    "qa.remove.title",
];

pub fn all_keys() -> Vec<String> {
    I18N_KEYS.iter().map(|key| (*key).to_string()).collect()
}
"#
    .to_string()
}

fn render_i18n_bundle() -> String {
    r#"{
  "qa.default.title": "Default configuration",
  "qa.default.description": "Review default settings for this component.",
  "qa.default.enabled.label": "Enable the component",
  "qa.default.enabled.help": "Toggle whether the component should run.",
  "qa.setup.title": "Initial setup",
  "qa.setup.description": "Provide initial configuration values.",
  "qa.setup.enabled.label": "Enable on setup",
  "qa.setup.enabled.help": "Enable the component after setup completes.",
  "qa.upgrade.title": "Upgrade configuration",
  "qa.remove.title": "Removal settings"
}
"#
    .to_string()
}

fn render_wit_package() -> String {
    r#"package greentic:component@0.6.0;

interface component-descriptor {
  get-component-info: func() -> list<u8>;
  describe: func() -> list<u8>;
}

interface component-schema {
  input-schema: func() -> list<u8>;
  output-schema: func() -> list<u8>;
  config-schema: func() -> list<u8>;
}

interface component-runtime {
  record run-result {
    output: list<u8>,
    new-state: list<u8>,
  }
  run: func(input: list<u8>, state: list<u8>) -> run-result;
}

interface component-qa {
  enum qa-mode {
    default,
    setup,
    upgrade,
    remove,
  }
  qa-spec: func(mode: qa-mode) -> list<u8>;
  apply-answers: func(mode: qa-mode, current-config: list<u8>, answers: list<u8>) -> list<u8>;
}

interface component-i18n {
  i18n-keys: func() -> list<string>;
}

world component-v0-v6-v0 {
  export component-descriptor;
  export component-schema;
  export component-runtime;
  export component-qa;
  export component-i18n;
}
"#
    .to_string()
}

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
