# Semver Fix Report

## Scope
- Reviewed cargo-semver-checks output for `greentic-component v0.4.74 -> v0.4.75`.
- Reported failure:
  - `constructible_struct_adds_field` on `WizardArgs.schema` in `crates/greentic-component/src/cmd/wizard.rs`.

## Fix Applied
1. Added `#[non_exhaustive]` to the public struct `WizardArgs`.

### File Changes
- `crates/greentic-component/src/cmd/wizard.rs`
  - Added `#[non_exhaustive]` above:
    - `pub struct WizardArgs`

## Why This Fix
- The semver violation is caused by adding a new public field to an externally constructible public struct.
- Marking the struct `#[non_exhaustive]` prevents downstream exhaustive struct literal construction assumptions, which is the preferred minimal semver-safe remediation.
- No runtime behavior or CLI logic was changed.

## Match/Wildcard Follow-up
- No additional wildcard match-arm updates were required for this change.
- `#[non_exhaustive]` was applied to a struct (not an enum), and no same-crate match exhaustiveness adjustments were needed.

## Versioning Decision
- No crate version bump was applied.
- The violation was resolved via attribute hardening (`#[non_exhaustive]`), per preferred strategy.
