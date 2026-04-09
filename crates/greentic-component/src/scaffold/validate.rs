#![cfg(feature = "cli")]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use miette::Diagnostic;
use once_cell::sync::Lazy;
use regex::Regex;
use semver::Version;
use thiserror::Error;

static NAME_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9]+([_-][a-z0-9]+)*$").expect("valid name regex"));
static OPERATION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z][a-z0-9_.:-]*$").expect("valid operation regex"));
static ORG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$")
        .expect("valid org regex")
});

#[derive(Debug, Error, Diagnostic)]
pub enum ValidationError {
    #[error("component name may not be empty")]
    #[diagnostic(
        code = "greentic.cli.name_empty",
        help = "Provide a kebab- or snake-case name, e.g. `demo-component`."
    )]
    EmptyName,
    #[error("component name must be lowercase kebab-or-snake case (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.name_invalid",
        help = "Use lowercase letters, digits, '-' or '_' separators."
    )]
    InvalidName(String),
    #[error("organization must be reverse-DNS style (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.org_invalid",
        help = "Use values like `ai.greentic` or `dev.example.tools`."
    )]
    InvalidOrg(String),
    #[error("invalid semantic version `{input}`: {source}")]
    #[diagnostic(
        code = "greentic.cli.version_invalid",
        help = "Use standard semver such as 0.1.0 or 1.2.3-alpha.1."
    )]
    InvalidSemver {
        input: String,
        #[source]
        source: semver::Error,
    },
    #[error("operation name must match the canonical manifest pattern (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.operation_invalid",
        help = "Use lowercase names starting with a letter; allowed characters are letters, digits, '.', '_', ':', and '-'."
    )]
    InvalidOperationName(String),
    #[error("operation `{0}` was declared more than once")]
    #[diagnostic(
        code = "greentic.cli.operation_duplicate",
        help = "Pass each operation only once."
    )]
    DuplicateOperationName(String),
    #[error("default_operation `{0}` must match one of the declared operations")]
    #[diagnostic(
        code = "greentic.cli.default_operation_unknown",
        help = "Choose a default operation from the operations declared for this component."
    )]
    UnknownDefaultOperation(String),
    #[error("filesystem mode must be one of none, read_only, sandbox (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.filesystem_mode_invalid",
        help = "Use one of `none`, `read_only`, or `sandbox`."
    )]
    InvalidFilesystemMode(String),
    #[error("filesystem mount must be `name:host_class:guest_path` (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.filesystem_mount_invalid",
        help = "Pass mounts as `name:host_class:guest_path`, for example `assets:assets:/assets`."
    )]
    InvalidFilesystemMount(String),
    #[error("telemetry scope must be one of tenant, pack, node (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.telemetry_scope_invalid",
        help = "Use one of `tenant`, `pack`, or `node`."
    )]
    InvalidTelemetryScope(String),
    #[error("secret format must be one of bytes, text, json (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.secret_format_invalid",
        help = "Use one of `bytes`, `text`, or `json`."
    )]
    InvalidSecretFormat(String),
    #[error("telemetry attribute must be `key=value` (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.telemetry_attribute_invalid",
        help = "Pass telemetry attributes as `key=value`."
    )]
    InvalidTelemetryAttribute(String),
    #[error("config field must be `name:type[:required|optional]` (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.config_field_invalid",
        help = "Pass config fields as `enabled:bool:required` or `api_key:string`."
    )]
    InvalidConfigField(String),
    #[error("config field name must be lowercase snake_case (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.config_field_name_invalid",
        help = "Use lowercase field names like `enabled` or `api_key`."
    )]
    InvalidConfigFieldName(String),
    #[error("config field type must be one of string, bool, integer, number (got `{0}`)")]
    #[diagnostic(
        code = "greentic.cli.config_field_type_invalid",
        help = "Use `string`, `bool`, `integer`, or `number`."
    )]
    InvalidConfigFieldType(String),
    #[error("unable to determine working directory: {0}")]
    #[diagnostic(code = "greentic.cli.cwd_unavailable")]
    WorkingDir(#[source] io::Error),
    #[error("target path points to an existing file: {0}")]
    #[diagnostic(
        code = "greentic.cli.path_is_file",
        help = "Pick a different --path or remove the file."
    )]
    TargetIsFile(PathBuf),
    #[error("target directory {0} already exists and is not empty")]
    #[diagnostic(
        code = "greentic.cli.path_not_empty",
        help = "Provide an empty directory or omit --path to create a new one."
    )]
    TargetDirNotEmpty(PathBuf),
    #[error("failed to inspect path {0}: {1}")]
    #[diagnostic(code = "greentic.cli.path_io")]
    Io(PathBuf, #[source] io::Error),
}

impl ValidationError {
    pub fn code(&self) -> &'static str {
        match self {
            ValidationError::EmptyName => "greentic.cli.name_empty",
            ValidationError::InvalidName(_) => "greentic.cli.name_invalid",
            ValidationError::InvalidOrg(_) => "greentic.cli.org_invalid",
            ValidationError::InvalidSemver { .. } => "greentic.cli.version_invalid",
            ValidationError::InvalidOperationName(_) => "greentic.cli.operation_invalid",
            ValidationError::DuplicateOperationName(_) => "greentic.cli.operation_duplicate",
            ValidationError::UnknownDefaultOperation(_) => "greentic.cli.default_operation_unknown",
            ValidationError::InvalidFilesystemMode(_) => "greentic.cli.filesystem_mode_invalid",
            ValidationError::InvalidFilesystemMount(_) => "greentic.cli.filesystem_mount_invalid",
            ValidationError::InvalidTelemetryScope(_) => "greentic.cli.telemetry_scope_invalid",
            ValidationError::InvalidSecretFormat(_) => "greentic.cli.secret_format_invalid",
            ValidationError::InvalidTelemetryAttribute(_) => {
                "greentic.cli.telemetry_attribute_invalid"
            }
            ValidationError::InvalidConfigField(_) => "greentic.cli.config_field_invalid",
            ValidationError::InvalidConfigFieldName(_) => "greentic.cli.config_field_name_invalid",
            ValidationError::InvalidConfigFieldType(_) => "greentic.cli.config_field_type_invalid",
            ValidationError::WorkingDir(_) => "greentic.cli.cwd_unavailable",
            ValidationError::TargetIsFile(_) => "greentic.cli.path_is_file",
            ValidationError::TargetDirNotEmpty(_) => "greentic.cli.path_not_empty",
            ValidationError::Io(_, _) => "greentic.cli.path_io",
        }
    }
}

pub type ValidationResult<T> = std::result::Result<T, ValidationError>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ComponentName(String);

impl ComponentName {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        if !NAME_RE.is_match(trimmed) {
            return Err(ValidationError::InvalidName(trimmed.to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

pub fn is_valid_name(value: &str) -> bool {
    ComponentName::parse(value).is_ok()
}

pub fn normalize_operation_name(value: &str) -> ValidationResult<String> {
    let trimmed = value.trim();
    if OPERATION_RE.is_match(trimmed) {
        Ok(trimmed.to_string())
    } else {
        Err(ValidationError::InvalidOperationName(trimmed.to_string()))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrgNamespace(String);

impl OrgNamespace {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let trimmed = value.trim();
        if ORG_RE.is_match(trimmed) {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(ValidationError::InvalidOrg(trimmed.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

pub fn normalize_version(value: &str) -> ValidationResult<String> {
    Version::parse(value)
        .map(|v| v.to_string())
        .map_err(|source| ValidationError::InvalidSemver {
            input: value.to_string(),
            source,
        })
}

pub fn resolve_target_path(
    name: &ComponentName,
    provided: Option<&Path>,
) -> Result<PathBuf, ValidationError> {
    let relative = provided
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name.as_str()));
    if relative.is_absolute() {
        return Ok(relative);
    }
    let cwd = env::current_dir().map_err(ValidationError::WorkingDir)?;
    Ok(cwd.join(relative))
}

pub fn ensure_path_available(path: &Path) -> Result<(), ValidationError> {
    match fs::metadata(path) {
        Ok(metadata) => {
            if metadata.is_file() {
                return Err(ValidationError::TargetIsFile(path.to_path_buf()));
            }
            let mut entries =
                fs::read_dir(path).map_err(|err| ValidationError::Io(path.to_path_buf(), err))?;
            if entries.next().is_some() {
                return Err(ValidationError::TargetDirNotEmpty(path.to_path_buf()));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(ValidationError::Io(path.to_path_buf(), err)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;

    #[test]
    fn rejects_invalid_names() {
        let err = ComponentName::parse("HelloWorld").unwrap_err();
        assert!(matches!(err, ValidationError::InvalidName(_)));
    }

    #[test]
    fn resolves_default_path_relative_to_cwd() {
        let name = ComponentName::parse("demo-component").unwrap();
        let path = resolve_target_path(&name, None).unwrap();
        assert!(path.ends_with("demo-component"));
    }

    #[test]
    fn detects_non_empty_directories() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("demo");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("file.txt"), "data").unwrap();
        let err = ensure_path_available(&target).unwrap_err();
        assert!(matches!(err, ValidationError::TargetDirNotEmpty(_)));
    }

    #[test]
    fn rejects_invalid_orgs() {
        let err = OrgNamespace::parse("NoDots").unwrap_err();
        assert!(matches!(err, ValidationError::InvalidOrg(_)));
    }

    #[test]
    fn accepts_valid_orgs() {
        let org = OrgNamespace::parse("ai.greentic").unwrap();
        assert_eq!(org.as_str(), "ai.greentic");
    }

    #[test]
    fn detects_invalid_semver() {
        assert!(matches!(
            normalize_version("01.0.0").unwrap_err(),
            ValidationError::InvalidSemver { .. }
        ));
    }

    #[test]
    fn normalizes_semver() {
        let normalized = normalize_version("1.2.3-alpha.1").unwrap();
        assert_eq!(normalized, "1.2.3-alpha.1");
    }
}
