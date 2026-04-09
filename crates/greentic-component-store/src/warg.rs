use crate::StoreError;

pub fn fetch(_reference: &str) -> Result<Vec<u8>, StoreError> {
    Err(StoreError::UnsupportedScheme("warg".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warg_fetch_is_explicitly_unimplemented() {
        let err = fetch("warg://registry.example/component@1.0.0").expect_err("warg should fail");
        assert!(matches!(err, StoreError::UnsupportedScheme(scheme) if scheme == "warg"));
    }
}
