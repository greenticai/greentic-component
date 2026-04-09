#![cfg(feature = "cli")]

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{Value as JsonValue, json};

use super::validate::ValidationError;

static CONFIG_FIELD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z][a-z0-9_]*$").expect("valid config field regex"));

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigSchemaInput {
    pub fields: Vec<ConfigSchemaField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSchemaField {
    pub name: String,
    pub field_type: ConfigSchemaFieldType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSchemaFieldType {
    String,
    Bool,
    Integer,
    Number,
}

impl ConfigSchemaInput {
    pub fn manifest_schema(&self) -> JsonValue {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in &self.fields {
            properties.insert(field.name.clone(), field.field_type.json_schema());
            if field.required {
                required.push(JsonValue::String(field.name.clone()));
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })
    }

    pub fn component_schema_file(&self, component_name: &str) -> JsonValue {
        let mut schema = self.manifest_schema();
        if let Some(obj) = schema.as_object_mut() {
            obj.insert(
                "$schema".to_string(),
                JsonValue::String("https://json-schema.org/draft/2020-12/schema".to_string()),
            );
            obj.insert(
                "title".to_string(),
                JsonValue::String(format!("{component_name} component configuration")),
            );
        }
        schema
    }

    pub fn rust_schema_ir(&self) -> String {
        if self.fields.is_empty() {
            return r#"SchemaIr::Object {
        properties: BTreeMap::new(),
        required: Vec::new(),
        additional: AdditionalProperties::Forbid,
    }"#
            .to_string();
        }

        let properties = self
            .fields
            .iter()
            .map(|field| {
                format!(
                    "(\"{}\".to_string(), {})",
                    field.name,
                    field.field_type.rust_schema_ir()
                )
            })
            .collect::<Vec<_>>()
            .join(",\n            ");
        let required = self
            .fields
            .iter()
            .filter(|field| field.required)
            .map(|field| format!("\"{}\".to_string()", field.name))
            .collect::<Vec<_>>();
        let required_expr = if required.is_empty() {
            "Vec::new()".to_string()
        } else {
            format!("vec![{}]", required.join(", "))
        };

        format!(
            r#"SchemaIr::Object {{
        properties: BTreeMap::from([
            {properties}
        ]),
        required: {required_expr},
        additional: AdditionalProperties::Forbid,
    }}"#
        )
    }
}

impl ConfigSchemaFieldType {
    fn json_schema(self) -> JsonValue {
        match self {
            Self::String => json!({ "type": "string" }),
            Self::Bool => json!({ "type": "boolean" }),
            Self::Integer => json!({ "type": "integer" }),
            Self::Number => json!({ "type": "number" }),
        }
    }

    fn rust_schema_ir(self) -> &'static str {
        match self {
            Self::String => {
                "SchemaIr::String { min_len: Some(0), max_len: None, regex: None, format: None }"
            }
            Self::Bool => "SchemaIr::Bool",
            Self::Integer => "SchemaIr::Int { min: None, max: None }",
            Self::Number => "SchemaIr::Float { min: None, max: None }",
        }
    }
}

pub fn parse_config_field(value: &str) -> Result<ConfigSchemaField, ValidationError> {
    let mut parts = value.split(':').map(str::trim);
    let name = parts.next().unwrap_or_default();
    let field_type = parts.next().unwrap_or_default();
    let required = parts.next().unwrap_or("optional");
    if name.is_empty() || field_type.is_empty() || parts.next().is_some() {
        return Err(ValidationError::InvalidConfigField(value.to_string()));
    }
    if !CONFIG_FIELD_RE.is_match(name) {
        return Err(ValidationError::InvalidConfigFieldName(name.to_string()));
    }
    let field_type = parse_config_field_type(field_type)?;
    let required = match required {
        "required" => true,
        "optional" => false,
        other => {
            return Err(ValidationError::InvalidConfigField(
                value.replace(required, other),
            ));
        }
    };
    Ok(ConfigSchemaField {
        name: name.to_string(),
        field_type,
        required,
    })
}

fn parse_config_field_type(value: &str) -> Result<ConfigSchemaFieldType, ValidationError> {
    match value {
        "string" => Ok(ConfigSchemaFieldType::String),
        "bool" | "boolean" => Ok(ConfigSchemaFieldType::Bool),
        "int" | "integer" => Ok(ConfigSchemaFieldType::Integer),
        "number" | "float" => Ok(ConfigSchemaFieldType::Number),
        other => Err(ValidationError::InvalidConfigFieldType(other.to_string())),
    }
}
