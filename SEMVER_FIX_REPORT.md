# SEMVER Fix Report

## Scope
- Crate: `greentic-component`
- Baseline: `v0.4.74`
- Current: `v0.4.75`
- Reported violation count: `1`

## Reported Violation
1. `struct_marked_non_exhaustive`
- Item: `WizardArgs`
- Location: `crates/greentic-component/src/cmd/wizard.rs:59`
- Meaning: the public struct became `#[non_exhaustive]`, which prevents external struct-literal construction and is semver-breaking.

## Fix Applied
- Removed `#[non_exhaustive]` from `WizardArgs` in:
  - `crates/greentic-component/src/cmd/wizard.rs`

## Why This Fix
- This restores the previous public API behavior (external construction via struct literal remains allowed).
- It is the minimal, behavior-preserving change and avoids a version bump.

## Additional Notes
- No logic or runtime behavior was changed.
- No tests were modified.
