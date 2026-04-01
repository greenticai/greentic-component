use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JsonPath(String);

impl JsonPath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for JsonPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum SchemaIntrospectionError {
    #[error("schema json parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn collect_redactions(schema_json: &str) -> Vec<JsonPath> {
    try_collect_redactions(schema_json).expect("schema traversal failed")
}

pub fn try_collect_redactions(
    schema_json: &str,
) -> Result<Vec<JsonPath>, SchemaIntrospectionError> {
    let value: Value = serde_json::from_str(schema_json)?;
    Ok(collect_redactions_from_value(&value))
}

pub fn collect_default_annotations(
    schema_json: &str,
) -> Result<Vec<(JsonPath, String)>, SchemaIntrospectionError> {
    let value: Value = serde_json::from_str(schema_json)?;
    Ok(collect_default_annotations_from_value(&value))
}

pub fn collect_capability_hints(
    schema_json: &str,
) -> Result<Vec<(JsonPath, String)>, SchemaIntrospectionError> {
    let value: Value = serde_json::from_str(schema_json)?;
    Ok(collect_capability_hints_from_value(&value))
}

fn walk(value: &Value, path: &str, visitor: &mut dyn FnMut(&serde_json::Map<String, Value>, &str)) {
    let mut path_buf = path.to_string();
    walk_inner(value, &mut path_buf, visitor);
}

fn walk_inner(
    value: &Value,
    path: &mut String,
    visitor: &mut dyn FnMut(&serde_json::Map<String, Value>, &str),
) {
    if let Value::Object(map) = value {
        visitor(map, path);

        if let Some(Value::Object(props)) = map.get("properties") {
            for (key, child) in props {
                let len = path.len();
                push(path, key);
                walk_inner(child, path, visitor);
                path.truncate(len);
            }
        }

        if let Some(Value::Object(pattern_props)) = map.get("patternProperties") {
            for (key, child) in pattern_props {
                let len = path.len();
                path.push_str(".patternProperties[");
                path.push_str(key);
                path.push(']');
                walk_inner(child, path, visitor);
                path.truncate(len);
            }
        }

        if let Some(items) = map.get("items") {
            let len = path.len();
            path.push_str("[*]");
            walk_inner(items, path, visitor);
            path.truncate(len);
        }

        if let Some(Value::Array(all_of)) = map.get("allOf") {
            for (idx, child) in all_of.iter().enumerate() {
                let len = path.len();
                path.push_str(".allOf[");
                path.push_str(&idx.to_string());
                path.push(']');
                walk_inner(child, path, visitor);
                path.truncate(len);
            }
        }

        if let Some(Value::Array(any_of)) = map.get("anyOf") {
            for (idx, child) in any_of.iter().enumerate() {
                let len = path.len();
                path.push_str(".anyOf[");
                path.push_str(&idx.to_string());
                path.push(']');
                walk_inner(child, path, visitor);
                path.truncate(len);
            }
        }

        if let Some(Value::Array(one_of)) = map.get("oneOf") {
            for (idx, child) in one_of.iter().enumerate() {
                let len = path.len();
                path.push_str(".oneOf[");
                path.push_str(&idx.to_string());
                path.push(']');
                walk_inner(child, path, visitor);
                path.truncate(len);
            }
        }
    }
}

pub(crate) fn collect_redactions_from_value(value: &Value) -> Vec<JsonPath> {
    let mut hits = Vec::new();
    walk(value, "$", &mut |map, path| {
        if map
            .get("x-redact")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            hits.push(JsonPath::new(path.to_string()));
        }
    });
    hits
}

pub(crate) fn collect_default_annotations_from_value(value: &Value) -> Vec<(JsonPath, String)> {
    let mut hits = Vec::new();
    walk(value, "$", &mut |map, path| {
        if let Some(defaulted) = map.get("x-default-applied").and_then(|v| v.as_str()) {
            hits.push((JsonPath::new(path.to_string()), defaulted.to_string()));
        }
    });
    hits
}

pub(crate) fn collect_redactions_and_defaults_from_value(
    value: &Value,
) -> (Vec<JsonPath>, Vec<(JsonPath, String)>) {
    let mut redactions = Vec::new();
    let mut defaults = Vec::new();
    walk(value, "$", &mut |map, path| {
        if map
            .get("x-redact")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            redactions.push(JsonPath::new(path.to_string()));
        }
        if let Some(defaulted) = map.get("x-default-applied").and_then(|v| v.as_str()) {
            defaults.push((JsonPath::new(path.to_string()), defaulted.to_string()));
        }
    });
    (redactions, defaults)
}

pub(crate) fn collect_capability_hints_from_value(value: &Value) -> Vec<(JsonPath, String)> {
    let mut hits = Vec::new();
    walk(value, "$", &mut |map, path| {
        if let Some(cap) = map.get("x-capability").and_then(|v| v.as_str()) {
            hits.push((JsonPath::new(path.to_string()), cap.to_string()));
        }
    });
    hits
}

fn push(path: &mut String, segment: &str) {
    path.push('.');
    path.push_str(segment);
}
