# Semver Fix Report

## Scope
- Reviewed cargo-semver-checks failures for the workspace comparison `v0.4.74 -> v0.4.75`.
- Only one violation required action:
  - `constructible_struct_adds_field` on `WizardArgs.schema` in `crates/greentic-component/src/cmd/wizard.rs`.

## Fix Applied
- Added `#[non_exhaustive]` to the public struct `WizardArgs`.
  - File: `crates/greentic-component/src/cmd/wizard.rs`
  - Change:
    - From: `#[derive(Args, Debug, Clone)] pub struct WizardArgs { ... }`
    - To: `#[derive(Args, Debug, Clone)] #[non_exhaustive] pub struct WizardArgs { ... }`

## Why This Fix
- The violation indicates an externally constructible public struct gained a new public field.
- Marking the struct `#[non_exhaustive]` is the preferred, minimal semver-safe fix because it prevents downstream exhaustive struct literal construction while preserving existing runtime behavior and internal logic.

## Behavioral Impact
- No logic changes.
- No runtime behavior changes.
- No test files modified.
- No crate version bump required for this specific fix strategy.

## Validation Performed
- Ran compile check:
  - `cargo check -p greentic-component --features cli`
  - Result: failed due to an unrelated pre-existing error in `crates/greentic-component/src/cmd/inspect.rs`:
    - `error[E0599]: no method named len found for struct ComponentProfiles`
    - Location: `crates/greentic-component/src/cmd/inspect.rs:457`
    - This is outside the semver fix scope and was not modified.
