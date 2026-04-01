# Security Fix Report

## Context
Addressed CodeQL alert for cleartext logging in the Rust CLI inspector.

## Alert Remediated
- Alert: `rust/cleartext-logging` (CodeQL alert #10)
- Severity: high
- File: `crates/greentic-component/src/cmd/inspect.rs`
- Location: line 457
- Finding: logged `manifest.profiles.len()` to output/log stream.

## Fix Applied
- Removed the `println!` statement that emitted the profile count:
  - Removed: `println!("  profiles count: {}", manifest.profiles.len());`

## Why This Is Safe
- Eliminates logging of data derived from potentially sensitive manifest profile content.
- No behavioral impact to core processing logic; only diagnostic output was reduced.

## Validation
- Performed source inspection to confirm the sensitive-derived log line was removed.
- No additional sensitive logging was introduced by this change.
