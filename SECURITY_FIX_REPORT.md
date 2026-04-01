# SECURITY_FIX_REPORT

## Summary
- Dependabot alerts provided: `0`
- Code scanning alerts provided: `1`
- Remediated alerts: `1`

Fixed CodeQL alert `rust/cleartext-logging` in `crates/greentic-component/src/cmd/inspect.rs` by removing logging of secret-related metadata derived from `manifest.secret_requirements`.

## Alert Triage
1. `rust/cleartext-logging` (high)
- File: `crates/greentic-component/src/cmd/inspect.rs:457`
- Finding: logging `manifest.secret_requirements.len()` was flagged as sensitive-data cleartext logging.
- Risk: even aggregate secret metadata should not be emitted to logs/stdout in security-sensitive paths.

## Remediation Applied
- Replaced:
  - `println!("  secret requirements: {}", manifest.secret_requirements.len());`
- With:
  - `println!("  secret requirements: [redacted]");`

This is a minimal, behavior-preserving safety fix that keeps inspect output structure while preventing exposure of secret-derived information.

## Verification
- Attempted: `cargo check -p greentic-component`
- Result: could not run in this CI sandbox due to Rustup temp-file write restrictions:
  - `could not create temp file /home/runner/.rustup/tmp/...: Read-only file system (os error 30)`

## Files Changed
- `crates/greentic-component/src/cmd/inspect.rs`
- `SECURITY_FIX_REPORT.md`
