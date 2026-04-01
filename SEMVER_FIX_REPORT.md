# Semver Fix Report

## Scope
- Reviewed cargo-semver-checks output for `greentic-component v0.4.74 -> v0.4.75`.
- One violation required a fix:
  - `struct_marked_non_exhaustive` on `WizardArgs` in `crates/greentic-component/src/cmd/wizard.rs:59`.

## Violation Analysis
- `WizardArgs` is a public struct with public fields.
- Marking it `#[non_exhaustive]` is a breaking change because downstream crates can no longer construct it with a struct literal.
- This exactly matches the reported semver violation.

## Fix Applied (Minimal and Safe)
- Removed `#[non_exhaustive]` from `WizardArgs`.
- File changed:
  - `crates/greentic-component/src/cmd/wizard.rs`
- Exact change:
  - From:
    - `#[derive(Args, Debug, Clone)]`
    - `#[non_exhaustive]`
    - `pub struct WizardArgs { ... }`
  - To:
    - `#[derive(Args, Debug, Clone)]`
    - `pub struct WizardArgs { ... }`

## Why This Resolves the Violation
- The API surface returns to its prior constructibility semantics.
- No public items were removed or renamed.
- No behavior or logic changed; only attribute metadata was adjusted.

## Additional Notes
- No wildcard `match` arm updates were needed, because no enum was changed to `#[non_exhaustive]` in this fix.
- No crate version bump was necessary.
