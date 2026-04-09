use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use greentic_component_store::ComponentStore;
use greentic_component_store::VerificationPolicy;

#[derive(Debug, Clone)]
pub struct HostPolicy {
    pub allow_http_fetch: bool,
    pub allow_telemetry: bool,
    pub allow_state_read: bool,
    pub allow_state_write: bool,
    pub allow_state_delete: bool,
    pub state_store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Default for HostPolicy {
    fn default() -> Self {
        Self {
            allow_http_fetch: false,
            allow_telemetry: true,
            allow_state_read: false,
            allow_state_write: false,
            allow_state_delete: false,
            state_store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadPolicy {
    pub store: Arc<ComponentStore>,
    pub verification: VerificationPolicy,
    pub host: HostPolicy,
}

impl LoadPolicy {
    pub fn new(store: Arc<ComponentStore>) -> Self {
        Self {
            store,
            verification: VerificationPolicy::default(),
            host: HostPolicy::default(),
        }
    }

    pub fn with_verification(mut self, policy: VerificationPolicy) -> Self {
        self.verification = policy;
        self
    }

    pub fn with_host_policy(mut self, host: HostPolicy) -> Self {
        self.host = host;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_component_store::{DigestPolicy, SignaturePolicy};

    #[test]
    fn default_host_policy_denies_mutating_or_network_features() {
        let policy = HostPolicy::default();
        assert!(!policy.allow_http_fetch);
        assert!(!policy.allow_state_read);
        assert!(!policy.allow_state_write);
        assert!(!policy.allow_state_delete);
        assert!(policy.allow_telemetry);
        assert!(policy.state_store.lock().expect("lock").is_empty());
    }

    #[test]
    fn load_policy_builder_preserves_store_and_overrides_fields() {
        let cache_dir = std::env::temp_dir().join(format!(
            "greentic-component-runtime-policy-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cache_dir).expect("cache dir");
        let store = Arc::new(ComponentStore::new(&cache_dir).expect("store"));
        let verification = VerificationPolicy {
            digest: Some(DigestPolicy::sha256(Some("abcd".into()), true)),
            signature: Some(SignaturePolicy::Disabled),
        };
        let host = HostPolicy {
            allow_http_fetch: true,
            ..HostPolicy::default()
        };

        let policy = LoadPolicy::new(store.clone())
            .with_verification(verification.clone())
            .with_host_policy(host.clone());

        assert!(Arc::ptr_eq(&policy.store, &store));
        assert_eq!(policy.verification.digest.unwrap().expected(), Some("abcd"));
        assert!(matches!(
            policy.verification.signature,
            Some(SignaturePolicy::Disabled)
        ));
        assert!(policy.host.allow_http_fetch);
        assert_eq!(policy.host.allow_telemetry, host.allow_telemetry);
    }
}
