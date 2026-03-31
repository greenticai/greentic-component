use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use component_manifest::validate_config_schema;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use wit_parser::{Resolve, Type, TypeDefKind, TypeOwner, WorldId, WorldItem};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ConfigSchemaSource {
    Manifest,
    Wit { path: PathBuf },
    SchemaFile { path: PathBuf },
    Stub,
}

#[derive(Debug, Clone)]
pub struct ConfigInferenceOptions {
    pub allow_infer: bool,
    pub write_schema: bool,
    pub force_write_schema: bool,
    pub validate: bool,
}

impl Default for ConfigInferenceOptions {
    fn default() -> Self {
        Self {
            allow_infer: true,
            write_schema: true,
            force_write_schema: false,
            validate: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigOutcome {
    pub manifest_path: PathBuf,
    pub manifest: JsonValue,
    pub schema: JsonValue,
    pub source: ConfigSchemaSource,
    pub schema_written: bool,
    pub persist_schema: bool,
}

pub fn resolve_manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("component.manifest.json")
    } else {
        path.to_path_buf()
    }
}

pub fn load_manifest_with_schema(
    manifest_path: &Path,
    opts: &ConfigInferenceOptions,
) -> Result<ConfigOutcome> {
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let mut manifest: JsonValue = serde_json::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("manifest path has no parent: {}", manifest_path.display()))?;

    let existing_schema = manifest.get("config_schema").cloned();
    let use_existing = existing_schema.is_some() && !opts.force_write_schema;

    let (schema, source) = if use_existing {
        (
            existing_schema.expect("guarded above"),
            ConfigSchemaSource::Manifest,
        )
    } else {
        if !opts.allow_infer {
            bail!("config_schema missing and --no-infer-config set");
        }

        let wit_candidate = if let Some(world) = manifest.get("world").and_then(|v| v.as_str()) {
            match infer_from_wit(manifest_dir, world) {
                Ok(found) => found,
                Err(err) => {
                    eprintln!(
                        "warning: failed to infer config_schema from WIT: {err:?}; falling back"
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(inferred) = wit_candidate {
            inferred
        } else if let Some(local_schema) = try_read_local_schema(manifest_dir)? {
            local_schema
        } else {
            (stub_schema(), ConfigSchemaSource::Stub)
        }
    };

    if opts.validate {
        validate_config_schema(&schema)
            .map_err(|err| anyhow!("config_schema failed validation: {err}"))?;
    }

    let mut schema_written = false;
    let persist_schema = opts.write_schema || use_existing;
    manifest["config_schema"] = schema.clone();

    let should_write = opts.write_schema && (!use_existing || opts.force_write_schema);
    if should_write {
        let formatted = serde_json::to_string_pretty(&manifest)?;
        fs::write(manifest_path, formatted + "\n")
            .with_context(|| format!("failed to write {}", manifest_path.display()))?;
        schema_written = true;
    }

    Ok(ConfigOutcome {
        manifest_path: manifest_path.to_path_buf(),
        manifest,
        schema,
        source,
        schema_written,
        persist_schema,
    })
}

fn try_read_local_schema(manifest_dir: &Path) -> Result<Option<(JsonValue, ConfigSchemaSource)>> {
    let candidate = manifest_dir.join("schemas/component.schema.json");
    if !candidate.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&candidate)
        .with_context(|| format!("failed to read {}", candidate.display()))?;
    let json: JsonValue = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", candidate.display()))?;
    Ok(Some((
        json,
        ConfigSchemaSource::SchemaFile { path: candidate },
    )))
}

fn infer_from_wit(
    manifest_dir: &Path,
    manifest_world: &str,
) -> Result<Option<(JsonValue, ConfigSchemaSource)>> {
    let wit_dir = manifest_dir.join("wit");
    if !wit_dir.exists() {
        return Ok(None);
    }

    let mut resolve = Resolve::default();
    let (pkg, _) = resolve
        .push_dir(&wit_dir)
        .with_context(|| format!("failed to parse WIT in {}", wit_dir.display()))?;

    let world_id = select_world(&resolve, pkg, manifest_world)
        .context("failed to locate WIT world for config inference")?;
    let config_id = find_config_type(&resolve, world_id)?;

    let schema = schema_from_record(&resolve, config_id)?;
    Ok(Some((schema, ConfigSchemaSource::Wit { path: wit_dir })))
}

fn select_world(
    resolve: &Resolve,
    pkg: wit_parser::PackageId,
    manifest_world: &str,
) -> Result<WorldId> {
    let target = parse_world_name(manifest_world);
    if let Some(target_name) = target
        && let Some((id, _)) = resolve
            .worlds
            .iter()
            .find(|(_, world)| world.package == Some(pkg) && world.name == target_name)
    {
        return Ok(id);
    }

    resolve
        .worlds
        .iter()
        .find(|(_, world)| world.package == Some(pkg))
        .map(|(id, _)| id)
        .ok_or_else(|| anyhow!("no world found in {}", resolve.packages[pkg].name.name))
}

fn parse_world_name(raw: &str) -> Option<String> {
    let after_slash = raw.split('/').nth(1)?;
    let without_version = after_slash.split('@').next()?;
    Some(without_version.to_string())
}

fn find_config_type(resolve: &Resolve, world_id: WorldId) -> Result<wit_parser::TypeId> {
    let interfaces = interfaces_in_world(resolve, world_id);
    resolve
        .types
        .iter()
        .find_map(|(id, ty)| {
            let owned_here = match ty.owner {
                TypeOwner::World(w) => w == world_id,
                TypeOwner::Interface(i) => interfaces.contains(&i),
                TypeOwner::None => false,
            };
            (owned_here && ty.name.as_deref() == Some("config")).then_some(id)
        })
        .ok_or_else(|| anyhow!("no `config` record found in WIT"))
}

fn interfaces_in_world(resolve: &Resolve, world_id: WorldId) -> HashSet<wit_parser::InterfaceId> {
    let mut ids = HashSet::new();
    let world = &resolve.worlds[world_id];
    for item in world.imports.values().chain(world.exports.values()) {
        if let WorldItem::Interface { id, .. } = item {
            ids.insert(*id);
        }
    }
    ids
}

fn schema_from_record(resolve: &Resolve, type_id: wit_parser::TypeId) -> Result<JsonValue> {
    let type_def = &resolve.types[type_id];
    let record = match &type_def.kind {
        TypeDefKind::Record(record) => record,
        TypeDefKind::Type(inner) => {
            let shape = map_type(resolve, inner)?;
            return Ok(shape.schema);
        }
        _ => bail!("config type must be a record"),
    };

    let mut properties = JsonMap::new();
    let mut required = Vec::new();

    for field in &record.fields {
        let directives = DocDirectives::from_docs(&field.docs);
        let shape = map_type(resolve, &field.ty)?;

        let mut prop = shape.schema;
        if let Some(desc) = directives.description {
            prop["description"] = JsonValue::String(desc);
        }
        if let Some(default) = directives.default {
            prop["default"] = default;
        }
        if directives.hidden {
            prop["x_flow_hidden"] = JsonValue::Bool(true);
        }

        properties.insert(field.name.clone(), prop);
        if !shape.optional {
            required.push(JsonValue::String(field.name.clone()));
        }
    }

    let mut schema = JsonMap::new();
    schema.insert("type".into(), JsonValue::String("object".into()));
    schema.insert("additionalProperties".into(), JsonValue::Bool(false));
    schema.insert("properties".into(), JsonValue::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), JsonValue::Array(required));
    }

    Ok(JsonValue::Object(schema))
}

struct TypeShape {
    schema: JsonValue,
    optional: bool,
}

fn map_type(resolve: &Resolve, ty: &Type) -> Result<TypeShape> {
    match ty {
        Type::Bool => Ok(TypeShape {
            schema: json_type("boolean"),
            optional: false,
        }),
        Type::String | Type::Char => Ok(TypeShape {
            schema: json_type("string"),
            optional: false,
        }),
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::S8
        | Type::S16
        | Type::S32
        | Type::S64 => Ok(TypeShape {
            schema: json_type("integer"),
            optional: false,
        }),
        Type::F32 | Type::F64 => Ok(TypeShape {
            schema: json_type("number"),
            optional: false,
        }),
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::Type(inner) => map_type(resolve, inner),
            TypeDefKind::Option(inner) => {
                let inner_shape = map_type(resolve, inner)?;
                Ok(TypeShape {
                    schema: inner_shape.schema,
                    optional: true,
                })
            }
            TypeDefKind::Enum(e) => {
                let values = e
                    .cases
                    .iter()
                    .map(|case| JsonValue::String(case.name.clone()))
                    .collect();
                Ok(TypeShape {
                    schema: JsonValue::Object(
                        [
                            ("type".into(), JsonValue::String("string".into())),
                            ("enum".into(), JsonValue::Array(values)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    optional: false,
                })
            }
            TypeDefKind::List(inner) => {
                let mapped = map_type(resolve, inner)?;
                Ok(TypeShape {
                    schema: JsonValue::Object(
                        [
                            ("type".into(), JsonValue::String("array".into())),
                            ("items".into(), mapped.schema),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    optional: false,
                })
            }
            TypeDefKind::Record(record) => {
                let mut properties = JsonMap::new();
                let mut required = Vec::new();
                for field in &record.fields {
                    let shape = map_type(resolve, &field.ty)?;
                    properties.insert(field.name.clone(), shape.schema);
                    if !shape.optional {
                        required.push(JsonValue::String(field.name.clone()));
                    }
                }
                let mut schema = JsonMap::new();
                schema.insert("type".into(), JsonValue::String("object".into()));
                schema.insert("properties".into(), JsonValue::Object(properties));
                if !required.is_empty() {
                    schema.insert("required".into(), JsonValue::Array(required));
                }
                Ok(TypeShape {
                    schema: JsonValue::Object(schema),
                    optional: false,
                })
            }
            _ => Ok(TypeShape {
                schema: json_type("string"),
                optional: false,
            }),
        },
        _ => Ok(TypeShape {
            schema: json_type("string"),
            optional: false,
        }),
    }
}

fn json_type(kind: &str) -> JsonValue {
    JsonValue::Object(
        [("type".into(), JsonValue::String(kind.to_string()))]
            .into_iter()
            .collect(),
    )
}

#[derive(Debug, Default)]
struct DocDirectives {
    description: Option<String>,
    default: Option<JsonValue>,
    hidden: bool,
}

impl DocDirectives {
    fn from_docs(docs: &wit_parser::Docs) -> Self {
        let Some(raw) = docs.contents.as_deref() else {
            return Self::default();
        };
        let default = extract_default(raw);
        let hidden = raw.contains("@flow:hidden");
        let description = render_description(raw);
        Self {
            description,
            default,
            hidden,
        }
    }
}

fn extract_default(raw: &str) -> Option<JsonValue> {
    let marker = "@default(";
    let start = raw.find(marker)?;
    let after = &raw[start + marker.len()..];
    let end = after.find(')')?;
    let body = after[..end].trim();
    if body.is_empty() {
        return None;
    }
    serde_json::from_str(body)
        .ok()
        .or_else(|| Some(JsonValue::String(body.to_string())))
}

fn render_description(raw: &str) -> Option<String> {
    let lines = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with('@'))
        .map(str::trim_end)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn stub_schema() -> JsonValue {
    JsonValue::Object(
        [
            ("type".into(), JsonValue::String("object".into())),
            ("properties".into(), JsonValue::Object(JsonMap::new())),
            ("required".into(), JsonValue::Array(Vec::new())),
            ("additionalProperties".into(), JsonValue::Bool(false)),
        ]
        .into_iter()
        .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_manifest(dir: &tempfile::TempDir, manifest: &JsonValue) -> PathBuf {
        let path = dir.path().join("component.manifest.json");
        fs::write(
            &path,
            serde_json::to_string_pretty(manifest).expect("manifest json"),
        )
        .expect("write manifest");
        path
    }

    #[test]
    fn resolve_manifest_path_appends_default_for_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            resolve_manifest_path(dir.path()),
            dir.path().join("component.manifest.json")
        );
    }

    #[test]
    fn load_manifest_prefers_existing_schema_without_rewriting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_manifest(
            &dir,
            &json!({
                "world": "greentic:component/component@0.6.0",
                "config_schema": {
                    "type": "object",
                    "properties": { "enabled": { "type": "boolean" } },
                    "required": ["enabled"],
                    "additionalProperties": false
                }
            }),
        );

        let outcome =
            load_manifest_with_schema(&manifest_path, &ConfigInferenceOptions::default()).unwrap();
        assert_eq!(outcome.source, ConfigSchemaSource::Manifest);
        assert!(!outcome.schema_written);
        assert!(outcome.persist_schema);
    }

    #[test]
    fn load_manifest_uses_local_schema_file_when_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_manifest(
            &dir,
            &json!({
                "world": "greentic:component/component@0.6.0"
            }),
        );
        let schema_dir = dir.path().join("schemas");
        fs::create_dir_all(&schema_dir).expect("schema dir");
        let schema_path = schema_dir.join("component.schema.json");
        fs::write(
            &schema_path,
            serde_json::to_string_pretty(&json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": [],
                "additionalProperties": false
            }))
            .unwrap(),
        )
        .expect("schema write");

        let outcome =
            load_manifest_with_schema(&manifest_path, &ConfigInferenceOptions::default()).unwrap();
        assert_eq!(
            outcome.source,
            ConfigSchemaSource::SchemaFile { path: schema_path }
        );
        assert!(outcome.schema_written);
        assert_eq!(outcome.schema["properties"]["name"]["type"], "string");
    }

    #[test]
    fn load_manifest_falls_back_to_stub_when_inference_sources_are_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_manifest(&dir, &json!({}));

        let outcome =
            load_manifest_with_schema(&manifest_path, &ConfigInferenceOptions::default()).unwrap();
        assert_eq!(outcome.source, ConfigSchemaSource::Stub);
        assert_eq!(outcome.schema, stub_schema());
    }

    #[test]
    fn load_manifest_errors_when_inference_is_disabled_and_schema_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest_path = write_manifest(&dir, &json!({}));
        let opts = ConfigInferenceOptions {
            allow_infer: false,
            ..ConfigInferenceOptions::default()
        };

        let err =
            load_manifest_with_schema(&manifest_path, &opts).expect_err("inference should fail");
        assert!(err.to_string().contains("--no-infer-config"));
    }

    #[test]
    fn infer_from_wit_reads_doc_directives_and_optional_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wit_dir = dir.path().join("wit");
        fs::create_dir_all(&wit_dir).expect("wit dir");
        fs::write(
            wit_dir.join("component.wit"),
            r#"
package greentic:component@0.6.0;
interface cfg {
  record config {
    /// Human description
    /// @default("hello")
    title: string,
    /// Secret field
    /// @flow:hidden
    secret: option<string>,
  }
}
world component {
  import cfg;
}
"#,
        )
        .expect("write wit");

        let inferred = infer_from_wit(dir.path(), "greentic:component/component@0.6.0")
            .expect("infer")
            .expect("schema present");

        assert_eq!(
            inferred.1,
            ConfigSchemaSource::Wit {
                path: wit_dir.clone()
            }
        );
        assert_eq!(inferred.0["properties"]["title"]["default"], "hello");
        assert_eq!(
            inferred.0["properties"]["title"]["description"],
            "Human description"
        );
        assert_eq!(inferred.0["properties"]["secret"]["x_flow_hidden"], true);
        assert_eq!(inferred.0["required"], json!(["title"]));
    }

    #[test]
    fn parse_world_name_extracts_name_from_world_reference() {
        assert_eq!(
            parse_world_name("greentic:component/component@0.6.0"),
            Some("component".into())
        );
        assert_eq!(parse_world_name("component-only"), None);
    }
}
