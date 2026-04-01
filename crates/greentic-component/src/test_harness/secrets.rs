use std::collections::{HashMap, HashSet};

use greentic_interfaces_wasmtime::host_helpers::v1::secrets_store::SecretsError;

#[derive(Clone, Debug)]
pub struct InMemorySecretsStore {
    allow_secrets: bool,
    allowed: HashSet<String>,
    secrets: HashMap<String, Vec<u8>>,
}

impl InMemorySecretsStore {
    pub fn new(allow_secrets: bool, allowed: HashSet<String>) -> Self {
        Self {
            allow_secrets,
            allowed,
            secrets: HashMap::new(),
        }
    }

    pub fn with_secrets(mut self, secrets: HashMap<String, String>) -> Self {
        self.secrets = secrets
            .into_iter()
            .map(|(key, value)| (key, value.into_bytes()))
            .collect();
        self
    }

    pub fn get(
        &self,
        key: &str,
    ) -> Result<Option<wasmtime::component::__internal::Vec<u8>>, SecretsError> {
        if !self.allow_secrets {
            return Err(SecretsError::Denied);
        }
        if !self.allowed.contains(key) {
            return Err(SecretsError::InvalidKey);
        }
        match self.secrets.get(key) {
            Some(bytes) => Ok(Some(bytes.clone())),
            None => Err(SecretsError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(allow: bool) -> InMemorySecretsStore {
        InMemorySecretsStore::new(allow, HashSet::from(["API_TOKEN".to_string()])).with_secrets(
            HashMap::from([("API_TOKEN".to_string(), "secret".to_string())]),
        )
    }

    #[test]
    fn get_respects_permission_and_key_validation() {
        assert!(matches!(
            store(false).get("API_TOKEN"),
            Err(SecretsError::Denied)
        ));
        assert!(matches!(
            store(true).get("OTHER"),
            Err(SecretsError::InvalidKey)
        ));
    }

    #[test]
    fn get_returns_not_found_for_missing_allowed_secret() {
        let store = InMemorySecretsStore::new(true, HashSet::from(["API_TOKEN".to_string()]));
        assert!(matches!(
            store.get("API_TOKEN"),
            Err(SecretsError::NotFound)
        ));
    }

    #[test]
    fn get_returns_secret_bytes_for_present_allowed_secret() {
        let bytes = store(true).get("API_TOKEN").unwrap().expect("secret bytes");
        assert_eq!(bytes, b"secret");
    }
}
