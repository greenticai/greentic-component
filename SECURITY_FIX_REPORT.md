# Security Fix Report

## Scope
- Reviewed provided CodeQL alert: `rust/cleartext-logging` in `crates/greentic-component/src/cmd/inspect.rs`.

## Findings
- The inspect command output included a sensitive-derived detail by logging the count of redaction paths.

## Fixes Applied
- Updated `crates/greentic-component/src/cmd/inspect.rs`:
  - Replaced:
    - `println!("  redaction paths: {}", prepared.redaction_paths().len());`
  - With:
    - `println!("  redaction paths: [redacted]");`

## Security Rationale
- Even aggregated/derived values from sensitive fields (such as redaction metadata counts) can leak information patterns.
- Redacting this output removes the sensitive-derived signal while preserving inspect command usability.

## Validation
- Attempted a focused build check:
  - `cargo check -p greentic-component`
- Result:
  - Could not execute in this CI sandbox because `rustup` attempted to write under `/home/runner/.rustup/tmp` on a read-only filesystem.

## Notes
- No Dependabot alerts were provided in the input.
- Applied minimal, targeted change only for the flagged code scanning issue.
