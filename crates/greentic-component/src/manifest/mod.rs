use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use jsonschema::{Validator, validator_for};
use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::capabilities::{
    Capabilities, ComponentConfigurators, ComponentProfiles, validate_capabilities,
};
use crate::limits::Limits;
use crate::provenance::Provenance;
use crate::telemetry::TelemetrySpec;
use greentic_types::component::ComponentOperation;
use greentic_types::flow::FlowKind;
use greentic_types::{SecretKey, SecretRequirement};

static RAW_SCHEMA: &str = include_str!("../../schemas/v1/component.manifest.schema.json");

static COMPILED_SCHEMA: Lazy<Validator> = Lazy::new(|| {
    let value: Value =
        serde_json::from_str(RAW_SCHEMA).expect("component manifest schema must be valid JSON");
    validator_for(&value).expect("component manifest schema must compile")
});

static OPERATION_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z][a-z0-9_.:-]*$").expect("valid operation regex"));

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ComponentManifest {
    pub id: ManifestId,
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub supports: Vec<FlowKind>,
    pub world: World,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_requirements: Vec<SecretRequirement>,
    pub profiles: ComponentProfiles,
    #[serde(default)]
    pub configurators: Option<ComponentConfigurators>,
    #[serde(default)]
    pub limits: Option<Limits>,
    #[serde(default)]
    pub telemetry: Option<TelemetrySpec>,
    pub describe_export: DescribeExport,
    pub operations: Vec<ComponentOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_operation: Option<String>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
    pub artifacts: Artifacts,
    pub hashes: Hashes,
}

impl ComponentManifest {
    pub fn wasm_artifact_path(&self, root: &Path) -> PathBuf {
        root.join(&self.artifacts.component_wasm)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ManifestId(String);

impl ManifestId {
    fn parse(id: String) -> Result<Self, ManifestError> {
        if id.trim().is_empty() {
            return Err(ManifestError::EmptyField("id"));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ManifestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct World(String);

impl World {
    fn parse(world: String) -> Result<Self, ManifestError> {
        if world.trim().is_empty() {
            return Err(ManifestError::InvalidWorld { world });
        }
        Ok(Self(world))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct DescribeExport(String);

impl DescribeExport {
    fn parse(export: String) -> Result<Self, ManifestError> {
        if export.trim().is_empty() {
            return Err(ManifestError::InvalidDescribeExport {
                export,
                reason: "describe_export cannot be empty".into(),
            });
        }
        Ok(Self(export))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> DescribeKind {
        if self.0.contains(':') && self.0.contains('/') {
            DescribeKind::WitWorld
        } else {
            DescribeKind::Export
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescribeKind {
    Export,
    WitWorld,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Artifacts {
    component_wasm: PathBuf,
}

impl Artifacts {
    pub fn component_wasm(&self) -> &Path {
        &self.component_wasm
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Hashes {
    pub component_wasm: WasmHash,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct WasmHash(String);

impl WasmHash {
    fn parse(hash: String) -> Result<Self, ManifestError> {
        let Some(rest) = hash.strip_prefix("blake3:") else {
            return Err(ManifestError::InvalidHashFormat { hash });
        };
        if rest.len() != 64 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ManifestError::InvalidHashFormat {
                hash: format!("blake3:{rest}"),
            });
        }
        Ok(Self(format!("blake3:{rest}")))
    }

    pub fn algorithm(&self) -> &str {
        "blake3"
    }

    pub fn digest(&self) -> &str {
        &self.0[7..]
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn schema() -> &'static str {
    RAW_SCHEMA
}

pub fn parse_manifest(raw: &str) -> Result<ComponentManifest, ManifestError> {
    let mut value: Value = serde_json::from_str(raw)?;
    normalize_state_delete(&mut value);
    validate_value(&value)?;
    let raw_manifest: RawManifest = serde_json::from_value(value)?;
    raw_manifest.try_into()
}

pub fn validate_manifest(raw: &str) -> Result<(), ManifestError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_value(&value)
}

fn validate_value(value: &Value) -> Result<(), ManifestError> {
    let errors: Vec<String> = COMPILED_SCHEMA
        .iter_errors(value)
        .map(|err| err.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Schema(errors.join(", ")))
    }
}

fn normalize_state_delete(value: &mut Value) {
    let state = value
        .get_mut("capabilities")
        .and_then(|caps| caps.get_mut("host"))
        .and_then(|host| host.get_mut("state"));
    if let Some(state) = state {
        let delete_enabled = state
            .get("delete")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if delete_enabled {
            state
                .as_object_mut()
                .map(|obj| obj.insert("write".to_string(), Value::Bool(true)));
        }
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest json parse failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manifest schema validation failed: {0}")]
    Schema(String),
    #[error("world identifier is invalid: `{world}`")]
    InvalidWorld { world: String },
    #[error("manifest field `{0}` cannot be empty")]
    EmptyField(&'static str),
    #[error("component must expose at least one operation")]
    MissingOperations,
    #[error("operation `{operation}` is invalid")]
    InvalidOperation { operation: String },
    #[error("duplicate operation `{0}` detected")]
    DuplicateOperation(String),
    #[error("default_operation `{operation}` must match one of the declared operations")]
    InvalidDefaultOperation { operation: String },
    #[error("component must support at least one flow kind")]
    MissingSupports,
    #[error("profiles.supported must include at least one profile identifier")]
    MissingProfiles,
    #[error("profiles.default `{default}` must be one of the supported profiles")]
    InvalidProfileDefault { default: String },
    #[error("invalid semantic version `{version}`: {source}")]
    InvalidVersion {
        version: String,
        #[source]
        source: semver::Error,
    },
    #[error("invalid describe export `{export}`: {reason}")]
    InvalidDescribeExport { export: String, reason: String },
    #[error("component wasm path must be relative (got `{path}`)")]
    InvalidArtifactPath { path: String },
    #[error("component wasm hash must be blake3:<hex> (got `{hash}`)")]
    InvalidHashFormat { hash: String },
    #[error("capability validation failed: {0}")]
    Capability(String),
    #[error("duplicate secret requirement `{0}` detected")]
    DuplicateSecretRequirement(String),
    #[error(
        "secret declarations disagree between `secret_requirements` and `capabilities.host.secrets.required`"
    )]
    InconsistentSecretRequirements,
    #[error("secret requirement `{key}` is invalid: {reason}")]
    InvalidSecretRequirement { key: String, reason: String },
    #[error("limits invalid: {0}")]
    Limits(String),
    #[error("provenance invalid: {0}")]
    Provenance(String),
}

#[derive(Debug, serde::Deserialize)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    world: String,
    #[serde(default)]
    supports: Vec<FlowKind>,
    #[serde(default)]
    capabilities: Capabilities,
    #[serde(default)]
    secret_requirements: Vec<SecretRequirement>,
    #[serde(default)]
    profiles: ComponentProfiles,
    #[serde(default)]
    configurators: Option<ComponentConfigurators>,
    #[serde(default)]
    limits: Option<Limits>,
    #[serde(default)]
    telemetry: Option<TelemetrySpec>,
    describe_export: String,
    operations: Vec<ComponentOperation>,
    #[serde(default)]
    default_operation: Option<String>,
    #[serde(default)]
    provenance: Option<Provenance>,
    artifacts: RawArtifacts,
    hashes: RawHashes,
}

impl TryFrom<RawManifest> for ComponentManifest {
    type Error = ManifestError;

    fn try_from(raw: RawManifest) -> Result<Self, Self::Error> {
        let mut raw = raw;
        if raw.name.trim().is_empty() {
            return Err(ManifestError::EmptyField("name"));
        }

        let id = ManifestId::parse(raw.id)?;
        let world = World::parse(raw.world)?;
        let version =
            Version::parse(&raw.version).map_err(|source| ManifestError::InvalidVersion {
                version: raw.version,
                source,
            })?;
        let describe_export = DescribeExport::parse(raw.describe_export)?;
        let artifacts = Artifacts::try_from(raw.artifacts)?;
        let hashes = Hashes::try_from(raw.hashes)?;

        if raw.supports.is_empty() {
            return Err(ManifestError::MissingSupports);
        }

        validate_profiles(&raw.profiles)?;

        if let Some(configurators) = &raw.configurators {
            validate_configurators(configurators)?;
        }

        let secret_requirements =
            normalize_secret_requirements(&mut raw.capabilities, &raw.secret_requirements)?;

        validate_capabilities(&raw.capabilities)
            .map_err(|err| ManifestError::Capability(err.to_string()))?;

        validate_secret_requirements(&secret_requirements)?;

        if let Some(limits) = &raw.limits {
            limits
                .validate()
                .map_err(|err| ManifestError::Limits(err.to_string()))?;
        }

        if let Some(provenance) = &raw.provenance {
            provenance
                .validate()
                .map_err(|err| ManifestError::Provenance(err.to_string()))?;
        }

        if raw.operations.is_empty() {
            return Err(ManifestError::MissingOperations);
        }
        let mut seen_operations = HashSet::new();
        for operation in &raw.operations {
            if !seen_operations.insert(&operation.name) {
                return Err(ManifestError::DuplicateOperation(operation.name.clone()));
            }
            if !OPERATION_PATTERN.is_match(&operation.name) {
                return Err(ManifestError::InvalidOperation {
                    operation: operation.name.clone(),
                });
            }
        }
        if let Some(default_operation) = &raw.default_operation
            && !raw
                .operations
                .iter()
                .any(|op| op.name == *default_operation)
        {
            return Err(ManifestError::InvalidDefaultOperation {
                operation: default_operation.clone(),
            });
        }

        Ok(Self {
            id,
            name: raw.name,
            version,
            world,
            supports: raw.supports,
            capabilities: raw.capabilities,
            secret_requirements,
            profiles: raw.profiles,
            configurators: raw.configurators,
            limits: raw.limits,
            telemetry: raw.telemetry,
            describe_export,
            operations: raw.operations,
            default_operation: raw.default_operation,
            provenance: raw.provenance,
            artifacts,
            hashes,
        })
    }
}

fn normalize_secret_requirements(
    capabilities: &mut Capabilities,
    top_level: &[SecretRequirement],
) -> Result<Vec<SecretRequirement>, ManifestError> {
    let host_required = capabilities
        .host
        .secrets
        .as_ref()
        .map(|secrets| secrets.required.clone())
        .unwrap_or_default();
    if top_level.is_empty() && !host_required.is_empty() {
        return Ok(host_required);
    }
    if !top_level.is_empty() && host_required.is_empty() {
        capabilities.host.secrets = Some(crate::capabilities::SecretsCapabilities {
            required: top_level.to_vec(),
        });
        return Ok(top_level.to_vec());
    }
    if !top_level.is_empty()
        && !host_required.is_empty()
        && !secret_requirement_sets_match(top_level, &host_required)
    {
        return Err(ManifestError::InconsistentSecretRequirements);
    }
    Ok(top_level.to_vec())
}

fn secret_requirement_sets_match(left: &[SecretRequirement], right: &[SecretRequirement]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right.iter()).all(|(lhs, rhs)| {
        lhs.key == rhs.key
            && lhs.required == rhs.required
            && lhs.scope == rhs.scope
            && lhs.format == rhs.format
            && lhs.schema == rhs.schema
    })
}

#[derive(Debug, serde::Deserialize)]
struct RawArtifacts {
    component_wasm: String,
}

impl TryFrom<RawArtifacts> for Artifacts {
    type Error = ManifestError;

    fn try_from(value: RawArtifacts) -> Result<Self, Self::Error> {
        ensure_relative(&value.component_wasm)?;
        Ok(Artifacts {
            component_wasm: PathBuf::from(value.component_wasm),
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct RawHashes {
    component_wasm: String,
}

impl TryFrom<RawHashes> for Hashes {
    type Error = ManifestError;

    fn try_from(value: RawHashes) -> Result<Self, Self::Error> {
        Ok(Hashes {
            component_wasm: WasmHash::parse(value.component_wasm)?,
        })
    }
}

fn ensure_relative(path: &str) -> Result<(), ManifestError> {
    let path_buf = PathBuf::from(path);
    if path_buf.is_absolute() {
        return Err(ManifestError::InvalidArtifactPath {
            path: path.to_string(),
        });
    }
    if matches!(path_buf.components().next(), Some(Component::Prefix(_))) {
        return Err(ManifestError::InvalidArtifactPath {
            path: path.to_string(),
        });
    }
    Ok(())
}

fn validate_secret_requirements(requirements: &[SecretRequirement]) -> Result<(), ManifestError> {
    let mut seen = std::collections::HashSet::new();
    for req in requirements {
        if !seen.insert(req.key.as_str().to_string()) {
            return Err(ManifestError::DuplicateSecretRequirement(
                req.key.as_str().to_string(),
            ));
        }

        SecretKey::new(req.key.as_str()).map_err(|err| {
            ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: err.to_string(),
            }
        })?;

        let scope = req
            .scope
            .as_ref()
            .ok_or_else(|| ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "scope must include env and tenant".into(),
            })?;

        if scope.env.trim().is_empty() {
            return Err(ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "scope.env must not be empty".into(),
            });
        }
        if scope.tenant.trim().is_empty() {
            return Err(ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "scope.tenant must not be empty".into(),
            });
        }
        if let Some(team) = &scope.team
            && team.trim().is_empty()
        {
            return Err(ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "scope.team must not be empty when provided".into(),
            });
        }

        if req.format.is_none() {
            return Err(ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "format must be specified".into(),
            });
        }

        if let Some(schema) = &req.schema
            && !schema.is_object()
        {
            return Err(ManifestError::InvalidSecretRequirement {
                key: req.key.as_str().to_string(),
                reason: "schema must be an object when provided".into(),
            });
        }
    }
    Ok(())
}

fn validate_profiles(profiles: &ComponentProfiles) -> Result<(), ManifestError> {
    if profiles.supported.is_empty() {
        return Err(ManifestError::MissingProfiles);
    }
    if let Some(default) = &profiles.default
        && !profiles.supported.iter().any(|entry| entry == default)
    {
        return Err(ManifestError::InvalidProfileDefault {
            default: default.clone(),
        });
    }
    Ok(())
}

fn validate_configurators(_configurators: &ComponentConfigurators) -> Result<(), ManifestError> {
    // Flow identifiers are validated by greentic-types, so no additional checks are required.
    Ok(())
}
