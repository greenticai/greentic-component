# Wizard Provider

This module exposes deterministic scaffold planning for wizard flows.

## Design

- `wizard [run|validate|apply] --mode <create|build_test|doctor> [--execution <dry-run|execute>]`
- `apply_scaffold(request, dry_run) -> ApplyResult` (create mode core)
- `execute_plan(plan) -> Result<()>`

`apply_scaffold(..., dry_run=true)` is side-effect free and returns a reproducible plan.
Execution is explicit and separate via `execute_plan`.

## CLI

`greentic-component wizard` is the single wizard entrypoint. It validates inputs, builds deterministic plans, and executes steps when requested.
Prefer `--answers` and `--emit-answers` for AnswerDocument import/export; legacy `--qa-answers` and `--qa-answers-out` remain supported for compatibility.

## Plan Shape

Current plan metadata includes:

- `plan_version`
- `generator`
- `template_version`
- `template_digest_blake3`
- `requested_abi_version`

Current step kinds:

- `ensure_dir`
- `write_file`
- `build_component`
- `test_component`
- `doctor`

## Orchestrator Usage

A higher-level orchestrator (for example `greentic-dev wizard`) can:

1. request `spec_scaffold` to render prompts in any frontend,
2. submit answers/context to `apply_scaffold` in dry-run mode,
3. review or persist the plan,
4. execute the plan via `execute_plan` when approved.
