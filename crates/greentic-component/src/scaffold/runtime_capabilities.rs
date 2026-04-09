#![cfg(feature = "cli")]

use std::collections::BTreeMap;

use serde_json::{Value as JsonValue, json};

use super::validate::ValidationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilitiesInput {
    pub filesystem_mode: String,
    pub filesystem_mounts: Vec<RuntimeFilesystemMount>,
    pub messaging_inbound: bool,
    pub messaging_outbound: bool,
    pub events_inbound: bool,
    pub events_outbound: bool,
    pub http_client: bool,
    pub http_server: bool,
    pub state_read: bool,
    pub state_write: bool,
    pub state_delete: bool,
    pub telemetry_scope: String,
    pub telemetry_span_prefix: Option<String>,
    pub telemetry_attributes: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
    pub secret_env: String,
    pub secret_tenant: String,
    pub secret_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFilesystemMount {
    pub name: String,
    pub host_class: String,
    pub guest_path: String,
}

impl Default for RuntimeCapabilitiesInput {
    fn default() -> Self {
        Self {
            filesystem_mode: "none".to_string(),
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
            telemetry_scope: "node".to_string(),
            telemetry_span_prefix: None,
            telemetry_attributes: BTreeMap::new(),
            secret_keys: Vec::new(),
            secret_env: "dev".to_string(),
            secret_tenant: "default".to_string(),
            secret_format: "text".to_string(),
        }
    }
}

impl RuntimeCapabilitiesInput {
    fn effective_filesystem_mounts(&self) -> &[RuntimeFilesystemMount] {
        if self.filesystem_mode == "none" {
            &[]
        } else {
            &self.filesystem_mounts
        }
    }

    pub fn manifest_secret_requirements(&self) -> JsonValue {
        JsonValue::Array(
            self.secret_keys
                .iter()
                .map(|key| {
                    json!({
                        "key": key,
                        "required": true,
                        "scope": {
                            "env": self.secret_env,
                            "tenant": self.secret_tenant
                        },
                        "format": self.secret_format
                    })
                })
                .collect(),
        )
    }

    pub fn manifest_capabilities(&self) -> JsonValue {
        let mut wasi = serde_json::Map::new();
        wasi.insert(
            "filesystem".to_string(),
            json!({
                "mode": self.filesystem_mode,
                "mounts": self.effective_filesystem_mounts().iter().map(|mount| {
                    json!({
                        "name": mount.name,
                        "host_class": mount.host_class,
                        "guest_path": mount.guest_path
                    })
                }).collect::<Vec<_>>()
            }),
        );
        wasi.insert("random".to_string(), JsonValue::Bool(true));
        wasi.insert("clocks".to_string(), JsonValue::Bool(true));

        let mut host = serde_json::Map::new();
        if self.messaging_inbound || self.messaging_outbound {
            host.insert(
                "messaging".to_string(),
                json!({
                    "inbound": self.messaging_inbound,
                    "outbound": self.messaging_outbound
                }),
            );
        }
        if self.events_inbound || self.events_outbound {
            host.insert(
                "events".to_string(),
                json!({
                    "inbound": self.events_inbound,
                    "outbound": self.events_outbound
                }),
            );
        }
        host.insert(
            "telemetry".to_string(),
            json!({
                "scope": self.telemetry_scope
            }),
        );
        host.insert(
            "secrets".to_string(),
            json!({
                "required": self.manifest_secret_requirements()
            }),
        );
        if self.http_client || self.http_server {
            host.insert(
                "http".to_string(),
                json!({
                    "client": self.http_client,
                    "server": self.http_server
                }),
            );
        }
        let state_write = self.state_write || self.state_delete;
        if self.state_read || state_write || self.state_delete {
            host.insert(
                "state".to_string(),
                json!({
                    "read": self.state_read,
                    "write": state_write,
                    "delete": self.state_delete
                }),
            );
        }

        let mut capabilities = serde_json::Map::new();
        capabilities.insert("wasi".to_string(), JsonValue::Object(wasi));
        capabilities.insert("host".to_string(), JsonValue::Object(host));
        JsonValue::Object(capabilities)
    }

    pub fn manifest_telemetry(&self) -> Option<JsonValue> {
        self.telemetry_span_prefix.as_ref().map(|prefix| {
            json!({
                "span_prefix": prefix,
                "attributes": self.telemetry_attributes,
                "emit_node_spans": true
            })
        })
    }
}

pub fn parse_filesystem_mode(value: &str) -> Result<String, ValidationError> {
    match value.trim() {
        "none" | "read_only" | "sandbox" => Ok(value.trim().to_string()),
        other => Err(ValidationError::InvalidFilesystemMode(other.to_string())),
    }
}

pub fn parse_telemetry_scope(value: &str) -> Result<String, ValidationError> {
    match value.trim() {
        "tenant" | "pack" | "node" => Ok(value.trim().to_string()),
        other => Err(ValidationError::InvalidTelemetryScope(other.to_string())),
    }
}

pub fn parse_secret_format(value: &str) -> Result<String, ValidationError> {
    match value.trim() {
        "bytes" | "text" | "json" => Ok(value.trim().to_string()),
        other => Err(ValidationError::InvalidSecretFormat(other.to_string())),
    }
}

pub fn parse_filesystem_mount(value: &str) -> Result<RuntimeFilesystemMount, ValidationError> {
    let mut parts = value.splitn(3, ':').map(str::trim);
    let name = parts.next().unwrap_or_default();
    let host_class = parts.next().unwrap_or_default();
    let guest_path = parts.next().unwrap_or_default();
    if name.is_empty() || host_class.is_empty() || guest_path.is_empty() {
        return Err(ValidationError::InvalidFilesystemMount(value.to_string()));
    }
    Ok(RuntimeFilesystemMount {
        name: name.to_string(),
        host_class: host_class.to_string(),
        guest_path: guest_path.to_string(),
    })
}

pub fn parse_telemetry_attributes(
    values: &[String],
) -> Result<BTreeMap<String, String>, ValidationError> {
    let mut attributes = BTreeMap::new();
    for value in values {
        let Some((key, attr_value)) = value.split_once('=') else {
            return Err(ValidationError::InvalidTelemetryAttribute(value.clone()));
        };
        let key = key.trim();
        let attr_value = attr_value.trim();
        if key.is_empty() || attr_value.is_empty() {
            return Err(ValidationError::InvalidTelemetryAttribute(value.clone()));
        }
        attributes.insert(key.to_string(), attr_value.to_string());
    }
    Ok(attributes)
}
