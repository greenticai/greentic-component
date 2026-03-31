use crate::StoreError;

pub fn fetch(_reference: &str) -> Result<Vec<u8>, StoreError> {
    Err(StoreError::UnsupportedScheme("oci".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oci_fetch_is_explicitly_unimplemented() {
        let err = fetch("oci://registry.example/component:1.0.0")
            .expect_err("oci support should fail explicitly");
        assert!(matches!(err, StoreError::UnsupportedScheme(scheme) if scheme == "oci"));
    }
}
