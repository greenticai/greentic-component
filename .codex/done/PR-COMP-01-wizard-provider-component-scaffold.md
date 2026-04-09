# PR-COMP-01 — Expose component scaffolding wizard provider (spec + apply -> plan)

**Repo:** `greentic-component`  
**Theme:** Delegating QA-driven wizards with deterministic replay, multi-frontend UI, and i18n.

## Outcomes
- Adds/extends wizard capability in this repo so **`greentic-dev wizard`** can delegate to it.
- Maintains **deterministic** behavior: **`apply()` produces a plan, execution is separate**.
- Reuses existing QA/schema primitives; avoid duplicating type systems.

## Non-goals
- No breaking CLI UX unless explicitly documented.
- No new “parallel QA types” if existing ones already exist (reuse).

## Why
`greentic-dev wizard` needs to delegate into component scaffolding deterministically. This repo should:
- own component scaffold questions
- produce a deterministic plan from answers
- avoid implementing UI

## Audit
- Identify existing CLI scaffolding entrypoints (`new`, `wizard`, templates).
- `rg -n "template|scaffold|new|wizard|qa" .`
- Identify current template parameter map and defaults.
- Identify whether this repo already uses greentic-qa types or greentic-types QA schemas.

Write `docs/wizard/audit.md` with findings + chosen integration.

## Implementation
### 0) Compatibility wrapper (required)
- Keep existing `cmd/wizard` UX/flags/output behavior stable.
- Refactor internals to `spec -> apply -> plan -> execute` behind a compat adapter.

### 1) Wizard provider module
Add `src/wizard/`:
- `spec(mode=scaffold, ctx) -> QaSpec` (or component QA spec type)
- `apply(mode=scaffold, ctx, answers, dry_run=true) -> ApplyResult`
  - validates
  - fills defaults/derived
  - returns `WizardPlan` containing structured steps:
    - ensure dirs
    - write template files
    - (optional) run fmt / tests as separate steps

### 2) Determinism requirements
- `apply(dry_run=true)` must not write files.
- Template selection must be pinned via a template version/digest (plan metadata).
- Plan model must be forward-compatible with cross-repo orchestration (`greentic-dev`).
  - Prefer shared plan types from `greentic-types` when available.
  - If unavailable, use a minimal local shim intentionally aligned to a shared model.

### 3) Answer mapping
- stable question IDs namespaced, e.g. `component.name`, `component.path`, `component.kind`, `component.features.*`.

## Tests
- golden: given answers fixture -> plan JSON matches expected snapshot
- smoke: execute plan in temp dir (when allowed) and ensure expected files exist

## Docs
- `docs/wizard/README.md`: how to call provider from orchestrator and direct CLI usage.

## Decisions (Locked for this PR)
1) Local `greentic-types` path during development
- Use crates-io dependency plus local patch override in this repo:
  - `greentic-types = "0.4"`
  - `[patch.crates-io] greentic-types = { path = "../greentic-types" }`
- This is the correct local workflow for pre-push, deterministic cross-repo iteration.

2) Generated scaffold dependency policy
- Generated scaffold `Cargo.toml` must remain publish-safe and portable.
- Do not emit path overrides in generated projects.
- Keep generated dependency as version-pinned (`greentic-types = "0.4"` or template-targeted pinned version).

3) Commit policy for workspace path overrides
- Commit sibling path overrides only if CI/dev standards guarantee sibling checkout layout.
- If CI must build standalone, prefer dev-only local override via Cargo patch config and docs.
- Recommended current policy:
  - Keep CI clean/standalone by default.
  - Document local fast-path using `.cargo/config.toml` + `[patch.crates-io]`.

4) `apply()` output type strategy
- Prefer shared `WizardPlan`/`WizardStep` types from `greentic-types` if available.
- If shared types are not available yet, use a temporary local plan shim with convergence fields:
  - `plan_version`
  - `metadata` (template version/digest, tool version)
  - `steps: Vec<WizardStep>` aligned to future shared shapes (`EnsureDir`, `WriteFile`, `CopyTree`, optional `RunCommand`).

## Scope notes
- Work list remains valid and aligned with deterministic provider direction.
- Additional required constraints:
  - Keep CLI compatibility via wrapper.
  - Avoid bespoke plan modeling that blocks later `greentic-dev` sharing.

## Codex prompt (copy/paste)

You are implementing **PR-COMP-01**.  
**Pre-authorized:** create/update files, add tests, add docs, run formatting, add CI checks if needed.  
**Avoid destructive actions:** do not delete large subsystems; prefer additive refactors; keep backward compatibility unless the PR explicitly says otherwise.

Steps:
1) Perform the **Audit** tasks first and summarize findings in PR notes.
2) Implement the change list with minimal diffs aligned to the current repo patterns.
3) Add tests (unit + one integration/smoke test) and update docs.
4) Ensure `cargo fmt` + `cargo test` pass.

Repo-specific guidance:
- Reuse existing scaffolding/template code; do not rewrite generators.
- Keep wizard provider as a thin wrapper that produces a plan.
- Add one fixture with deterministic output paths.
