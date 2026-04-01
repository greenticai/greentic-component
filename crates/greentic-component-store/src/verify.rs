use sha2::{Digest as _, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    Sha256,
}

#[derive(Debug, Clone)]
pub struct DigestPolicy {
    algorithm: DigestAlgorithm,
    expected: Option<String>,
    required: bool,
}

impl DigestPolicy {
    pub fn sha256(expected: Option<String>, required: bool) -> Self {
        Self {
            algorithm: DigestAlgorithm::Sha256,
            expected,
            required,
        }
    }

    pub fn expected(&self) -> Option<&str> {
        self.expected.as_deref()
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<VerifiedDigest, VerificationError> {
        let computed = match self.algorithm {
            DigestAlgorithm::Sha256 => {
                let digest = Sha256::digest(bytes);
                VerifiedDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: hex::encode(digest),
                }
            }
        };

        if let Some(expected) = &self.expected {
            if !equal_digest(expected, &computed.value) {
                return Err(VerificationError::DigestMismatch {
                    expected: expected.clone(),
                    actual: computed.value,
                });
            }
        } else if self.required {
            return Err(VerificationError::DigestMissing);
        }

        Ok(computed)
    }
}

#[derive(Debug, Clone)]
pub enum SignaturePolicy {
    Disabled,
    Cosign { required: bool },
}

impl SignaturePolicy {
    pub fn cosign_required() -> Self {
        SignaturePolicy::Cosign { required: true }
    }

    pub fn cosign_optional() -> Self {
        SignaturePolicy::Cosign { required: false }
    }

    pub fn verify(&self, _bytes: &[u8]) -> Result<VerifiedSignature, VerificationError> {
        match self {
            SignaturePolicy::Disabled => Ok(VerifiedSignature::Skipped),
            SignaturePolicy::Cosign { required } => {
                if *required {
                    Err(VerificationError::SignatureNotImplemented(
                        "cosign signature verification required".into(),
                    ))
                } else {
                    Ok(VerifiedSignature::Skipped)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VerificationPolicy {
    pub digest: Option<DigestPolicy>,
    pub signature: Option<SignaturePolicy>,
}

impl VerificationPolicy {
    pub fn verify(&self, bytes: &[u8]) -> Result<VerificationReport, VerificationError> {
        let digest = match &self.digest {
            Some(policy) => Some(policy.verify(bytes)?),
            None => None,
        };
        let signature = match &self.signature {
            Some(policy) => Some(policy.verify(bytes)?),
            None => None,
        };
        Ok(VerificationReport { digest, signature })
    }
}

#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub digest: Option<VerifiedDigest>,
    pub signature: Option<VerifiedSignature>,
}

#[derive(Debug, Clone)]
pub struct VerifiedDigest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

impl VerifiedDigest {
    pub fn compute(algorithm: DigestAlgorithm, bytes: &[u8]) -> Self {
        match algorithm {
            DigestAlgorithm::Sha256 => {
                let digest = Sha256::digest(bytes);
                Self {
                    algorithm,
                    value: hex::encode(digest),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum VerifiedSignature {
    Skipped,
}

#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("digest check required but no expected value provided")]
    DigestMissing,
    #[error("digest mismatch (expected {expected}, actual {actual})")]
    DigestMismatch { expected: String, actual: String },
    #[error("signature verification not implemented: {0}")]
    SignatureNotImplemented(String),
}

fn equal_digest(expected: &str, actual: &str) -> bool {
    expected.eq_ignore_ascii_case(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn digest_policy_reports_missing_expected_digest_when_required() {
        let err = DigestPolicy::sha256(None, true)
            .verify(b"hello")
            .expect_err("missing digest should fail when required");
        assert!(matches!(err, VerificationError::DigestMissing));
    }

    #[test]
    fn digest_policy_accepts_expected_digest_case_insensitively() {
        let digest = DigestPolicy::sha256(Some(HELLO_SHA256.to_uppercase()), true)
            .verify(b"hello")
            .expect("digest should match case-insensitively");
        assert_eq!(digest.algorithm, DigestAlgorithm::Sha256);
        assert_eq!(digest.value, HELLO_SHA256);
    }

    #[test]
    fn digest_policy_reports_actual_digest_on_mismatch() {
        let err = DigestPolicy::sha256(Some("deadbeef".into()), true)
            .verify(b"hello")
            .expect_err("mismatch should fail");
        assert!(matches!(
            err,
            VerificationError::DigestMismatch { ref expected, ref actual }
                if expected == "deadbeef" && actual == HELLO_SHA256
        ));
    }

    #[test]
    fn signature_policy_only_fails_when_cosign_is_required() {
        let optional = SignaturePolicy::cosign_optional()
            .verify(b"hello")
            .expect("optional cosign should be skipped");
        assert!(matches!(optional, VerifiedSignature::Skipped));

        let required = SignaturePolicy::cosign_required()
            .verify(b"hello")
            .expect_err("required cosign should fail until implemented");
        assert!(matches!(
            required,
            VerificationError::SignatureNotImplemented(message)
                if message.contains("cosign")
        ));
    }

    #[test]
    fn verification_policy_combines_digest_and_signature_results() {
        let policy = VerificationPolicy {
            digest: Some(DigestPolicy::sha256(Some(HELLO_SHA256.into()), true)),
            signature: Some(SignaturePolicy::Disabled),
        };

        let report = policy
            .verify(b"hello")
            .expect("verification should succeed");
        assert_eq!(report.digest.expect("digest report").value, HELLO_SHA256);
        assert!(matches!(
            report.signature.expect("signature report"),
            VerifiedSignature::Skipped
        ));
    }
}
