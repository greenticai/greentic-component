# Security Fix Report

## Scope
- Processed provided security alerts JSON.
- `dependabot`: no alerts.
- `code_scanning`: 1 open alert (`rust/cleartext-logging`) in `crates/greentic-component/src/cmd/inspect.rs`.

## Remediation Applied
### Alert #10: `rust/cleartext-logging`
- File: `crates/greentic-component/src/cmd/inspect.rs`
- Location: line 457
- Issue: cleartext logging of potentially sensitive profile data via debug formatting.

### Change made
- Replaced direct cleartext output of profiles:
  - From: `println!("  profiles: {:?}", manifest.profiles);`
  - To: `println!("  profiles count: {}", manifest.profiles.len());`

## Security Impact
- Eliminates direct emission of profile contents to logs/stdout.
- Preserves useful operational visibility by reporting only count metadata.
- Change is minimal and low-risk, limited to output formatting.

## Files Modified
- `crates/greentic-component/src/cmd/inspect.rs`
- `SECURITY_FIX_REPORT.md`
