#![cfg(feature = "cli")]

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use component_manifest::validate_config_schema;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::config::{
    ConfigInferenceOptions, ConfigOutcome, load_manifest_with_schema, resolve_manifest_path,
};

const DEFAULT_MANIFEST: &str = "component.manifest.json";
const DEFAULT_KIND: &str = "component-config";
pub(crate) const COMPONENT_EXEC_KIND: &str = "component.exec";

#[derive(Subcommand, Debug, Clone)]
pub enum FlowCommand {
    /// Regenerate config flows and embed them into component.manifest.json
    Update(FlowUpdateArgs),
}

#[derive(Args, Debug, Clone)]
pub struct FlowUpdateArgs {
    /// Path to component.manifest.json (or directory containing it)
    #[arg(long = "manifest", value_name = "PATH", default_value = DEFAULT_MANIFEST)]
    pub manifest: PathBuf,
    /// Skip config inference; fail if config_schema is missing
    #[arg(long = "no-infer-config")]
    pub no_infer_config: bool,
    /// Do not write inferred config_schema back to the manifest
    #[arg(long = "no-write-schema")]
    pub no_write_schema: bool,
    /// Overwrite existing config_schema with inferred schema
    #[arg(long = "force-write-schema")]
    pub force_write_schema: bool,
    /// Skip schema validation
    #[arg(long = "no-validate")]
    pub no_validate: bool,
}

pub fn run(command: FlowCommand) -> Result<()> {
    match command {
        FlowCommand::Update(args) => {
            update(args)?;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct FlowUpdateResult {
    pub default_updated: bool,
    pub custom_updated: bool,
}

#[derive(Debug)]
pub struct FlowUpdateOutcome {
    pub manifest: JsonValue,
    pub result: FlowUpdateResult,
}

pub fn update(args: FlowUpdateArgs) -> Result<FlowUpdateResult> {
    let manifest_path = resolve_manifest_path(&args.manifest);
    let inference_opts = ConfigInferenceOptions {
        allow_infer: !args.no_infer_config,
        write_schema: !args.no_write_schema,
        force_write_schema: args.force_write_schema,
        validate: !args.no_validate,
    };
    let config = load_manifest_with_schema(&manifest_path, &inference_opts)?;
    let FlowUpdateOutcome {
        mut manifest,
        result,
    } = update_with_manifest(&config)?;

    if !config.persist_schema {
        manifest
            .as_object_mut()
            .map(|obj| obj.remove("config_schema"));
    }

    write_manifest(&manifest_path, &manifest)?;

    if config.schema_written && config.persist_schema {
        println!(
            "Updated {} with inferred config_schema ({:?})",
            manifest_path.display(),
            config.source
        );
    }
    println!(
        "Updated dev_flows (default: {}, custom: {}) in {}",
        result.default_updated,
        result.custom_updated,
        manifest_path.display()
    );

    Ok(result)
}

pub fn update_with_manifest(config: &ConfigOutcome) -> Result<FlowUpdateOutcome> {
    let component_id = manifest_component_id(&config.manifest)?;
    let component_name = manifest_component_name(&config.manifest)?;
    let _node_kind = resolve_node_kind(&config.manifest)?;
    let operation = resolve_operation(&config.manifest, component_id)?;
    let input_schema =
        load_operation_input_schema(&config.manifest_path, &config.manifest, &operation)?;

    validate_config_schema(&config.schema)
        .map_err(|err| anyhow!("config_schema failed validation: {err}"))?;

    let fields = collect_fields(&input_schema)?;

    let default_flow = render_default_flow(component_id, component_name, &operation, &fields)?;
    let custom_flow = render_custom_flow(component_id, component_name, &operation, &fields)?;

    let mut manifest = config.manifest.clone();
    let manifest_obj = manifest
        .as_object_mut()
        .ok_or_else(|| anyhow!("manifest must be a JSON object"))?;
    let dev_flows_entry = manifest_obj
        .entry("dev_flows")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let dev_flows = dev_flows_entry
        .as_object_mut()
        .ok_or_else(|| anyhow!("dev_flows must be an object"))?;

    let mut merged = BTreeMap::new();
    for (key, value) in dev_flows.iter() {
        if key != "custom" && key != "default" {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged.insert(
        "custom".to_string(),
        json!({
            "format": "flow-ir-json",
            "graph": custom_flow,
        }),
    );
    merged.insert(
        "default".to_string(),
        json!({
            "format": "flow-ir-json",
            "graph": default_flow,
        }),
    );

    *dev_flows = merged.into_iter().collect();

    Ok(FlowUpdateOutcome {
        manifest,
        result: FlowUpdateResult {
            default_updated: true,
            custom_updated: true,
        },
    })
}

fn collect_fields(config_schema: &JsonValue) -> Result<Vec<ConfigField>> {
    let properties = config_schema
        .get("properties")
        .and_then(|value| value.as_object())
        .ok_or_else(|| anyhow!("config_schema.properties must be an object"))?;
    let required = config_schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<HashSet<String>>()
        })
        .unwrap_or_default();

    let mut fields = properties
        .iter()
        .map(|(name, schema)| ConfigField::from_schema(name, schema, required.contains(name)))
        .collect::<Vec<_>>();
    fields.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(fields)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldType {
    String,
    Number,
    Integer,
    Boolean,
    Unknown,
}

impl FieldType {
    fn from_schema(schema: &JsonValue) -> Self {
        let type_value = schema.get("type");
        match type_value {
            Some(JsonValue::String(value)) => Self::from_type_str(value),
            Some(JsonValue::Array(types)) => types
                .iter()
                .filter_map(|v| v.as_str())
                .find_map(|value| {
                    let field_type = Self::from_type_str(value);
                    (field_type != FieldType::Unknown && value != "null").then_some(field_type)
                })
                .unwrap_or(FieldType::Unknown),
            _ => FieldType::Unknown,
        }
    }

    fn from_type_str(value: &str) -> Self {
        match value {
            "string" => FieldType::String,
            "number" => FieldType::Number,
            "integer" => FieldType::Integer,
            "boolean" => FieldType::Boolean,
            _ => FieldType::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigField {
    name: String,
    description: Option<String>,
    field_type: FieldType,
    enum_options: Vec<String>,
    default_value: Option<JsonValue>,
    required: bool,
    hidden: bool,
}

impl ConfigField {
    fn from_schema(name: &str, schema: &JsonValue, required: bool) -> Self {
        let field_type = FieldType::from_schema(schema);
        let description = schema
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let default_value = schema.get("default").cloned();
        let enum_options = schema
            .get("enum")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .map(|entry| {
                        entry
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| entry.to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hidden = schema
            .get("x_flow_hidden")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        Self {
            name: name.to_string(),
            description,
            field_type,
            enum_options,
            default_value,
            required,
            hidden,
        }
    }

    fn prompt(&self) -> String {
        if let Some(desc) = &self.description {
            return desc.clone();
        }
        humanize(&self.name)
    }

    fn question_type(&self) -> &'static str {
        if !self.enum_options.is_empty() {
            "enum"
        } else {
            match self.field_type {
                FieldType::String => "string",
                FieldType::Number | FieldType::Integer => "number",
                FieldType::Boolean => "boolean",
                FieldType::Unknown => "string",
            }
        }
    }

    fn is_string_like(&self) -> bool {
        !self.enum_options.is_empty()
            || matches!(self.field_type, FieldType::String | FieldType::Unknown)
    }
}

fn humanize(raw: &str) -> String {
    let mut result = raw
        .replace(['_', '-'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if !result.ends_with(':') && !result.is_empty() {
        result.push(':');
    }
    result
}

fn render_default_flow(
    component_id: &str,
    component_name: &str,
    operation: &str,
    fields: &[ConfigField],
) -> Result<JsonValue> {
    let field_values = compute_default_fields(fields)?;

    let emit_template = render_emit_template(component_name, operation, field_values);
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "emit_config".to_string(),
        json!({
            "template": emit_template,
        }),
    );

    let doc = FlowDocument {
        id: format!("{component_id}.default"),
        kind: DEFAULT_KIND.to_string(),
        description: format!("Auto-generated default config for {component_id}"),
        nodes,
    };

    flow_to_value(&doc)
}

fn render_custom_flow(
    component_id: &str,
    component_name: &str,
    operation: &str,
    fields: &[ConfigField],
) -> Result<JsonValue> {
    let visible_fields = fields
        .iter()
        .filter(|field| !field.hidden)
        .collect::<Vec<_>>();

    let mut question_fields = Vec::new();
    for field in &visible_fields {
        let mut mapping = JsonMap::new();
        mapping.insert("id".into(), JsonValue::String(field.name.clone()));
        mapping.insert("prompt".into(), JsonValue::String(field.prompt()));
        mapping.insert(
            "type".into(),
            JsonValue::String(field.question_type().to_string()),
        );
        if !field.enum_options.is_empty() {
            mapping.insert(
                "options".into(),
                JsonValue::Array(
                    field
                        .enum_options
                        .iter()
                        .map(|value| JsonValue::String(value.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(default_value) = &field.default_value {
            mapping.insert("default".into(), default_value.clone());
        }
        question_fields.push(JsonValue::Object(mapping));
    }

    let mut questions_inner = JsonMap::new();
    questions_inner.insert("fields".into(), JsonValue::Array(question_fields));

    let mut ask_node = JsonMap::new();
    ask_node.insert("questions".into(), JsonValue::Object(questions_inner));
    ask_node.insert(
        "routing".into(),
        JsonValue::Array(vec![json!({ "to": "emit_config" })]),
    );

    let emit_field_values = visible_fields
        .iter()
        .map(|field| EmitField {
            name: field.name.clone(),
            value: if field.is_string_like() {
                EmitFieldValue::StateQuoted(field.name.clone())
            } else {
                EmitFieldValue::StateRaw(field.name.clone())
            },
        })
        .collect::<Vec<_>>();
    let emit_template = render_emit_template(component_name, operation, emit_field_values);

    let mut nodes = BTreeMap::new();
    nodes.insert("ask_config".to_string(), JsonValue::Object(ask_node));
    nodes.insert(
        "emit_config".to_string(),
        json!({ "template": emit_template }),
    );

    let doc = FlowDocument {
        id: format!("{component_id}.custom"),
        kind: DEFAULT_KIND.to_string(),
        description: format!("Auto-generated custom config for {component_id}"),
        nodes,
    };

    flow_to_value(&doc)
}

fn render_emit_template(component_name: &str, operation: &str, fields: Vec<EmitField>) -> String {
    let mut lines = Vec::new();
    lines.push("{".to_string());
    lines.push(format!("  \"node_id\": \"{component_name}\","));
    lines.push("  \"node\": {".to_string());
    lines.push(format!("    \"{operation}\": {{"));
    lines.push("      \"input\": {".to_string());
    for (idx, field) in fields.iter().enumerate() {
        let suffix = if idx + 1 == fields.len() { "" } else { "," };
        lines.push(format!(
            "        \"{}\": {}{}",
            field.name,
            field.value.render(),
            suffix
        ));
    }
    lines.push("      }".to_string());
    lines.push("    },".to_string());
    lines.push("    \"routing\": [".to_string());
    lines.push("      { \"to\": \"NEXT_NODE_PLACEHOLDER\" }".to_string());
    lines.push("    ]".to_string());
    lines.push("  }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

pub(crate) fn manifest_component_id(manifest: &JsonValue) -> Result<&str> {
    manifest
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("component.manifest.json must contain a string `id` field"))
}

fn manifest_component_name(manifest: &JsonValue) -> Result<&str> {
    manifest
        .get("name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("component.manifest.json must contain a string `name` field"))
}

fn resolve_node_kind(manifest: &JsonValue) -> Result<&'static str> {
    let requested = manifest
        .get("mode")
        .or_else(|| manifest.get("kind"))
        .and_then(|value| value.as_str());
    let resolved = requested.unwrap_or(COMPONENT_EXEC_KIND);
    if resolved == "tool" {
        bail!("mode/kind `tool` is no longer supported for config flows");
    }
    if resolved != COMPONENT_EXEC_KIND {
        bail!(
            "unsupported config flow node kind `{resolved}`; allowed kinds: {COMPONENT_EXEC_KIND}"
        );
    }
    Ok(COMPONENT_EXEC_KIND)
}

pub(crate) fn resolve_operation(manifest: &JsonValue, component_id: &str) -> Result<String> {
    let missing_msg = || {
        anyhow!(
            "Component {component_id} has no operations; add at least one operation (e.g. handle_message)"
        )
    };
    let operations_value = manifest.get("operations").ok_or_else(missing_msg)?;
    let operations_array = operations_value
        .as_array()
        .ok_or_else(|| anyhow!("`operations` must be an array of objects"))?;
    let mut operations = Vec::new();
    for entry in operations_array {
        let op = entry
            .as_object()
            .ok_or_else(|| anyhow!("`operations` entries must be objects"))?;
        let name = op
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("`operations` entries must include a string `name` field"))?;
        if name.trim().is_empty() {
            return Err(missing_msg());
        }
        let input_schema = op.get("input_schema").ok_or_else(|| {
            anyhow!("`operations` entries must include input_schema and output_schema")
        })?;
        let output_schema = op.get("output_schema").ok_or_else(|| {
            anyhow!("`operations` entries must include input_schema and output_schema")
        })?;
        if !input_schema.is_object() || !output_schema.is_object() {
            return Err(anyhow!(
                "`operations` input_schema/output_schema must be objects"
            ));
        }
        operations.push(name.to_string());
    }
    if operations.is_empty() {
        return Err(missing_msg());
    }

    let default_operation = manifest
        .get("default_operation")
        .and_then(|value| value.as_str());
    let chosen = if let Some(default) = default_operation {
        if default.trim().is_empty() {
            return Err(anyhow!("default_operation cannot be empty"));
        }
        if operations.iter().any(|op| op == default) {
            default.to_string()
        } else {
            return Err(anyhow!(
                "default_operation `{default}` must match one of the declared operations"
            ));
        }
    } else if operations.len() == 1 {
        operations[0].clone()
    } else {
        return Err(anyhow!(
            "Component {component_id} declares multiple operations {:?}; set `default_operation` to pick one",
            operations
        ));
    };
    Ok(chosen)
}

struct EmitField {
    name: String,
    value: EmitFieldValue,
}

enum EmitFieldValue {
    Literal(String),
    StateQuoted(String),
    StateRaw(String),
}

impl EmitFieldValue {
    fn render(&self) -> String {
        match self {
            EmitFieldValue::Literal(value) => value.clone(),
            EmitFieldValue::StateQuoted(name) => format!("\"{{{{state.{name}}}}}\""),
            EmitFieldValue::StateRaw(name) => format!("{{{{state.{name}}}}}"),
        }
    }
}

#[derive(Serialize)]
struct FlowDocument {
    id: String,
    kind: String,
    description: String,
    nodes: BTreeMap<String, JsonValue>,
}

fn flow_to_value(doc: &FlowDocument) -> Result<JsonValue> {
    serde_json::to_value(doc).context("failed to render flow to JSON")
}

fn write_manifest(manifest_path: &PathBuf, manifest: &JsonValue) -> Result<()> {
    let formatted = serde_json::to_string_pretty(manifest)?;
    fs::write(manifest_path, formatted + "\n")
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn load_operation_input_schema(
    manifest_path: &Path,
    manifest: &JsonValue,
    operation_name: &str,
) -> Result<JsonValue> {
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("manifest path has no parent: {}", manifest_path.display()))?;

    let operations = manifest
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| anyhow!("manifest.operations must be an array"))?;
    let operation = operations
        .iter()
        .find(|op| op.get("name").and_then(JsonValue::as_str) == Some(operation_name))
        .ok_or_else(|| anyhow!("operation `{operation_name}` not found in manifest.operations"))?;
    let input_schema = operation
        .get("input_schema")
        .ok_or_else(|| anyhow!("operation `{operation_name}` is missing input_schema"))?;

    // If input_schema contains a $ref, resolve it by reading the referenced file
    if let Some(ref_path) = input_schema.get("$ref").and_then(JsonValue::as_str) {
        let schema_path = manifest_dir.join(ref_path);
        let text = fs::read_to_string(&schema_path)
            .with_context(|| format!("failed to read {}", schema_path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", schema_path.display()))
    } else {
        // Inline schema - use it directly
        Ok(input_schema.clone())
    }
}

fn compute_default_fields(fields: &[ConfigField]) -> Result<Vec<EmitField>> {
    let mut emit_fields = Vec::new();
    for field in fields {
        if field.required {
            if let Some(default_value) = &field.default_value {
                let literal = serde_json::to_string(default_value)
                    .context("failed to serialize default value")?;
                emit_fields.push(EmitField {
                    name: field.name.clone(),
                    value: EmitFieldValue::Literal(literal),
                });
            } else {
                bail!(
                    "Required field {} has no default; cannot generate default dev_flow. Provide defaults or use custom mode.",
                    field.name
                );
            }
        }
    }
    Ok(emit_fields)
}
