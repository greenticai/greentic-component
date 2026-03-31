use std::collections::HashSet;

use crate::capabilities::CapabilityError;
use crate::capabilities::{
    Capabilities, FilesystemCapabilities, FilesystemMode, HostCapabilities, TelemetryScope,
    WasiCapabilities,
};
use crate::manifest::ComponentManifest;

/// Host profile describing the maximum capabilities granted to a component.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub allowed: Capabilities,
}

impl Profile {
    pub fn new(allowed: Capabilities) -> Self {
        Self { allowed }
    }
}

pub fn enforce_capabilities(
    manifest: &ComponentManifest,
    profile: Profile,
) -> Result<(), CapabilityError> {
    ensure_wasi(&manifest.capabilities.wasi, &profile.allowed.wasi)?;
    ensure_host(&manifest.capabilities.host, &profile.allowed.host)
}

fn ensure_wasi(
    requested: &WasiCapabilities,
    allowed: &WasiCapabilities,
) -> Result<(), CapabilityError> {
    if let Some(fs) = &requested.filesystem {
        let policy = allowed.filesystem.as_ref().ok_or_else(|| {
            CapabilityError::invalid("wasi.filesystem", "filesystem access denied")
        })?;
        ensure_filesystem(fs, policy)?;
    }

    if let Some(env) = &requested.env {
        let policy = allowed
            .env
            .as_ref()
            .ok_or_else(|| CapabilityError::invalid("wasi.env", "environment access denied"))?;
        let allowed_vars: HashSet<_> = policy.allow.iter().collect();
        for var in &env.allow {
            if !allowed_vars.contains(var) {
                return Err(CapabilityError::invalid(
                    "wasi.env.allow",
                    format!("env `{var}` not permitted by profile"),
                ));
            }
        }
    }

    if requested.random && !allowed.random {
        return Err(CapabilityError::invalid(
            "wasi.random",
            "profile denies random number generation",
        ));
    }
    if requested.clocks && !allowed.clocks {
        return Err(CapabilityError::invalid(
            "wasi.clocks",
            "profile denies clock access",
        ));
    }

    Ok(())
}

fn ensure_filesystem(
    requested: &FilesystemCapabilities,
    allowed: &FilesystemCapabilities,
) -> Result<(), CapabilityError> {
    if mode_rank(&requested.mode) > mode_rank(&allowed.mode) {
        return Err(CapabilityError::invalid(
            "wasi.filesystem.mode",
            "requested mode exceeds profile allowance",
        ));
    }

    let allowed_mounts: HashSet<_> = allowed
        .mounts
        .iter()
        .map(|mount| (&mount.name, &mount.host_class, &mount.guest_path))
        .collect();
    for mount in &requested.mounts {
        let key = (&mount.name, &mount.host_class, &mount.guest_path);
        if !allowed_mounts.contains(&key) {
            return Err(CapabilityError::invalid(
                "wasi.filesystem.mounts",
                format!("mount `{}` is not available in this profile", mount.name),
            ));
        }
    }
    Ok(())
}

fn mode_rank(mode: &FilesystemMode) -> u8 {
    match mode {
        FilesystemMode::None => 0,
        FilesystemMode::ReadOnly => 1,
        FilesystemMode::Sandbox => 2,
    }
}

fn ensure_host(
    requested: &HostCapabilities,
    allowed: &HostCapabilities,
) -> Result<(), CapabilityError> {
    if let Some(secrets) = &requested.secrets {
        let policy = allowed
            .secrets
            .as_ref()
            .ok_or_else(|| CapabilityError::invalid("host.secrets", "secrets access denied"))?;
        let allowed_set: HashSet<_> = policy.required.iter().map(|req| req.key.as_str()).collect();
        for key in secrets.required.iter().map(|req| req.key.as_str()) {
            if !allowed_set.contains(key) {
                return Err(CapabilityError::invalid(
                    "host.secrets.required",
                    format!("secret `{key}` is not available"),
                ));
            }
        }
    }

    if let Some(state) = &requested.state {
        let policy = allowed
            .state
            .as_ref()
            .ok_or_else(|| CapabilityError::invalid("host.state", "state access denied"))?;
        if state.read && !policy.read {
            return Err(CapabilityError::invalid(
                "host.state.read",
                "profile denies state reads",
            ));
        }
        if state.write && !policy.write {
            return Err(CapabilityError::invalid(
                "host.state.write",
                "profile denies state writes",
            ));
        }
    }

    ensure_io_capability(
        requested
            .messaging
            .as_ref()
            .map(|m| (m.inbound, m.outbound)),
        allowed.messaging.as_ref().map(|m| (m.inbound, m.outbound)),
        "host.messaging",
    )?;
    ensure_io_capability(
        requested.events.as_ref().map(|m| (m.inbound, m.outbound)),
        allowed.events.as_ref().map(|m| (m.inbound, m.outbound)),
        "host.events",
    )?;
    ensure_io_capability(
        requested.http.as_ref().map(|h| (h.client, h.server)),
        allowed.http.as_ref().map(|h| (h.client, h.server)),
        "host.http",
    )?;

    if let Some(telemetry) = &requested.telemetry {
        let policy = allowed
            .telemetry
            .as_ref()
            .ok_or_else(|| CapabilityError::invalid("host.telemetry", "telemetry access denied"))?;
        if !telemetry_scope_allowed(&policy.scope, &telemetry.scope) {
            return Err(CapabilityError::invalid(
                "host.telemetry.scope",
                format!(
                    "requested scope `{:?}` exceeds profile allowance `{:?}`",
                    telemetry.scope, policy.scope
                ),
            ));
        }
    }

    if let Some(iac) = &requested.iac {
        let policy = allowed
            .iac
            .as_ref()
            .ok_or_else(|| CapabilityError::invalid("host.iac", "iac access denied"))?;
        if iac.write_templates && !policy.write_templates {
            return Err(CapabilityError::invalid(
                "host.iac.write_templates",
                "profile denies template writes",
            ));
        }
        if iac.execute_plans && !policy.execute_plans {
            return Err(CapabilityError::invalid(
                "host.iac.execute_plans",
                "profile denies plan execution",
            ));
        }
    }

    Ok(())
}

fn ensure_io_capability(
    requested: Option<(bool, bool)>,
    allowed: Option<(bool, bool)>,
    label: &'static str,
) -> Result<(), CapabilityError> {
    if let Some((req_in, req_out)) = requested {
        let Some((allow_in, allow_out)) = allowed else {
            return Err(CapabilityError::invalid(
                label,
                "profile denies this capability",
            ));
        };
        if req_in && !allow_in {
            return Err(CapabilityError::invalid(
                label,
                "inbound access denied by profile",
            ));
        }
        if req_out && !allow_out {
            return Err(CapabilityError::invalid(
                label,
                "outbound access denied by profile",
            ));
        }
    }
    Ok(())
}

fn telemetry_scope_allowed(allowed: &TelemetryScope, requested: &TelemetryScope) -> bool {
    scope_rank(allowed) >= scope_rank(requested)
}

fn scope_rank(scope: &TelemetryScope) -> u8 {
    match scope {
        TelemetryScope::Tenant => 0,
        TelemetryScope::Pack => 1,
        TelemetryScope::Node => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest;
    use greentic_types::component::{
        ComponentCapabilities, EnvCapabilities, FilesystemMount, HttpCapabilities, IaCCapabilities,
        SecretsCapabilities, StateCapabilities, TelemetryCapabilities,
    };
    use greentic_types::{SecretFormat, SecretKey, SecretRequirement, SecretScope};
    use serde_json::json;

    fn manifest_with_caps(capabilities: Capabilities) -> ComponentManifest {
        const DUMMY_HASH: &str =
            "blake3:0000000000000000000000000000000000000000000000000000000000000000";
        parse_manifest(&json!({
            "id": "com.greentic.test.component",
            "name": "test",
            "version": "0.1.0",
            "world": "greentic:component/component@0.6.0",
            "describe_export": "describe",
            "operations": [{
                "name": "run",
                "input_schema": {
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                },
                "output_schema": {
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }
            }],
            "default_operation": "run",
            "config_schema": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            },
            "supports": ["messaging"],
            "profiles": { "default": "stateless", "supported": ["stateless"] },
            "secret_requirements": [],
            "capabilities": capabilities,
            "limits": { "memory_mb": 64, "wall_time_ms": 1000 },
            "artifacts": { "component_wasm": "component.wasm" },
            "hashes": { "component_wasm": DUMMY_HASH },
            "dev_flows": {
                "default": {
                    "format": "flow-ir-json",
                    "graph": {
                        "nodes": [{ "id": "start", "type": "start" }, { "id": "end", "type": "end" }],
                        "edges": [{ "from": "start", "to": "end" }]
                    }
                }
            }
        })
        .to_string())
        .expect("manifest fixture")
    }

    fn baseline_caps() -> Capabilities {
        ComponentCapabilities {
            wasi: WasiCapabilities {
                filesystem: None,
                env: None,
                random: false,
                clocks: false,
            },
            host: HostCapabilities {
                messaging: None,
                events: None,
                http: None,
                secrets: None,
                state: None,
                telemetry: None,
                iac: None,
            },
        }
    }

    fn secret_requirement(key: &str) -> SecretRequirement {
        let mut requirement = SecretRequirement::default();
        requirement.key = SecretKey::new(key).expect("valid secret key");
        requirement.required = true;
        requirement.scope = Some(SecretScope {
            env: "dev".into(),
            tenant: "tenant-a".into(),
            team: None,
        });
        requirement.format = Some(SecretFormat::Text);
        requirement
    }

    #[test]
    fn denies_unlisted_environment_variables() {
        let mut requested = baseline_caps();
        requested.wasi.env = Some(EnvCapabilities {
            allow: vec!["SAFE".into(), "UNSAFE".into()],
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.wasi.env = Some(EnvCapabilities {
            allow: vec!["SAFE".into()],
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("profile should reject undeclared env var");
        assert_eq!(err.path, "wasi.env.allow");
        assert!(err.message.contains("UNSAFE"));
    }

    #[test]
    fn denies_telemetry_scope_escalation() {
        let mut requested = baseline_caps();
        requested.host.telemetry = Some(TelemetryCapabilities {
            scope: TelemetryScope::Node,
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.host.telemetry = Some(TelemetryCapabilities {
            scope: TelemetryScope::Tenant,
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("tenant-only telemetry should reject node scope");
        assert_eq!(err.path, "host.telemetry.scope");
    }

    #[test]
    fn denies_http_server_when_profile_only_allows_client() {
        let mut requested = baseline_caps();
        requested.host.http = Some(HttpCapabilities {
            client: false,
            server: true,
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.host.http = Some(HttpCapabilities {
            client: true,
            server: false,
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("server capability should be denied");
        assert_eq!(err.path, "host.http");
        assert!(err.message.contains("outbound access denied") || err.message.contains("inbound"));
    }

    #[test]
    fn denies_state_write_when_profile_is_read_only() {
        let mut requested = baseline_caps();
        requested.host.state = Some(StateCapabilities {
            read: true,
            write: true,
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.host.state = Some(StateCapabilities {
            read: true,
            write: false,
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("write access should be denied");
        assert_eq!(err.path, "host.state.write");
    }

    #[test]
    fn denies_random_and_clock_access_when_profile_disallows_them() {
        let mut requested = baseline_caps();
        requested.wasi.random = true;
        requested.wasi.clocks = true;
        let manifest = manifest_with_caps(requested);

        let err = enforce_capabilities(&manifest, Profile::new(baseline_caps()))
            .expect_err("random access should be denied first");
        assert_eq!(err.path, "wasi.random");

        let mut requested = baseline_caps();
        requested.wasi.clocks = true;
        let manifest = manifest_with_caps(requested);
        let err = enforce_capabilities(&manifest, Profile::new(baseline_caps()))
            .expect_err("clock access should be denied");
        assert_eq!(err.path, "wasi.clocks");
    }

    #[test]
    fn denies_filesystem_mode_escalation() {
        let mount = FilesystemMount {
            name: "data".into(),
            host_class: "data".into(),
            guest_path: "/data".into(),
        };
        let mut requested = baseline_caps();
        requested.wasi.filesystem = Some(FilesystemCapabilities {
            mode: FilesystemMode::Sandbox,
            mounts: vec![mount.clone()],
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.wasi.filesystem = Some(FilesystemCapabilities {
            mode: FilesystemMode::ReadOnly,
            mounts: vec![mount],
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("sandbox access should exceed read-only profile");
        assert_eq!(err.path, "wasi.filesystem.mode");
    }

    #[test]
    fn denies_secret_access_when_profile_has_no_secret_capability() {
        let mut requested = baseline_caps();
        requested.host.secrets = Some(SecretsCapabilities {
            required: vec![secret_requirement("api-key")],
        });
        let manifest = manifest_with_caps(requested);

        let err = enforce_capabilities(&manifest, Profile::new(baseline_caps()))
            .expect_err("secrets should be denied");

        assert_eq!(err.path, "host.secrets");
    }

    #[test]
    fn denies_iac_plan_execution_when_profile_forbids_it() {
        let mut requested = baseline_caps();
        requested.host.iac = Some(IaCCapabilities {
            write_templates: false,
            execute_plans: true,
        });
        let manifest = manifest_with_caps(requested);

        let mut allowed = baseline_caps();
        allowed.host.iac = Some(IaCCapabilities {
            write_templates: true,
            execute_plans: false,
        });

        let err = enforce_capabilities(&manifest, Profile::new(allowed))
            .expect_err("plan execution should be denied");

        assert_eq!(err.path, "host.iac.execute_plans");
    }
}
