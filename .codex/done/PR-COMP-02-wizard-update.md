# PR-COMP-02: Replace unreleased wizard UX with unified `wizard` (create/build_test/doctor) on existing deterministic core

## Title

feat(wizard): ship unified `wizard` (create/build_test/doctor), delete unreleased wizard new/spec paths, keep deterministic plan core

## Goals

- Make one wizard UX: `greentic-component wizard`
- Reuse the repo’s existing deterministic spec→plan→execute core (no greenfield rewrite)
- Add qa-lib answers + `--qa-answers` replay + `--qa-answers-out`
- Add `ExecutionMode`: `dry_run|execute`
- Add i18n keys for all wizard output; keep single root `en.json`
- Add i18n quality tests (key existence + required locale presence)

## User-facing CLI (final)

### Command

`greentic-component wizard`

### Options

- `--mode create|build_test|doctor`
- `--execution dry_run|execute`
- `--qa-answers <answers.json>`
- `--qa-answers-out <answers.json>`
- `--locale <LOCALE>`
- `--project-root <PATH>` (build_test/doctor, default `.`)
- `--template <TEMPLATE_ID>` (create, optional if QA asks)
- `--full-tests` (build_test; optional)

## Remove (delete)

- `greentic-component wizard new`
- `greentic-component wizard spec`

Also remove any mention in help text/docs so there is no UX ambiguity.

## Implementation approach (incremental, keeps current deterministic core)

### 1) CLI rewrite (small, targeted)

Update `src/cli.rs`:

- delete subcommands `wizard new` and `wizard spec`
- make `wizard` a single command with options above
- keep other command tree unchanged

### 2) Add orchestration enums + answer schema

In existing wizard module (where deterministic plan core lives), add:

- `RunMode { Create, BuildTest, Doctor }`
- `ExecutionMode { DryRun, Execute }`

Add versioned answers contract:

- `schema: "component-wizard-run/v1"`
- `mode`
- `fields`

### 3) QA spec builder (reuse, don’t rewrite)

Extend existing spec builder to support `RunMode`:

- `Create`: template + name + output dir (+ any required metadata already supported)
- `BuildTest`: project_root, build_target (if applicable), full_tests bool
- `Doctor`: project_root (and optional profile if already supported)

All question titles/help text must be i18n keys.

### 4) Plan model extension (minimal)

Add new step variants to the existing deterministic plan model:

- `ScaffoldFromTemplate { template_id, name, output_dir, ... }`
- `BuildComponent { project_root, ... }`
- `TestComponent { project_root, full: bool }`
- `Doctor { project_root, ... }`

Keep existing plan envelope JSON stable; only add step variants.

### 5) Executor uses internal Rust commands (no shell)

Implement execution by calling existing internal entrypoints:

- `cmd::build::run(...)`
- `cmd::test::run(...)`
- `cmd::doctor::run(...)`

Create typed adapters if signatures do not match directly.

### 6) Execution behavior

- `dry_run`: emit deterministic plan JSON + optionally write `--qa-answers-out`
- `execute`: execute plan steps deterministically + emit deterministic result JSON; still support `--qa-answers-out` for reproducibility

### 7) i18n: single root catalog

- Keep `i18n/en.json` as source of truth
- Add new keys with prefixes:
  - `cli.wizard.run.*`
  - `cli.wizard.mode.*`
  - `cli.wizard.execution.*`
  - `cli.wizard.step.*`
  - `cli.wizard.result.*`
- Ensure all new output uses `tr/trf` (no raw user-facing strings)

### 8) Tests + gates (fit existing topology)

Update existing wizard tests to target unified `wizard` command.

Add i18n quality tests under `crates/greentic-component/tests/`:

- `i18n_key_exists.rs`: ensure all wizard keys referenced in spec/outputs exist in `en.json`
- `i18n_required_locales.rs`: ensure required locales exist (start with `en`; expand later)

Wire into `ci/local_check.sh` (repo pattern):

- run tests
- run i18n checks
- optionally `tools/i18n.sh status` if already present

### 9) Delete unreleased wizard code

Remove:

- old wizard new/spec command handlers
- any unused old mode enums that become dead
- dead files/modules
- docs/help text references

Keep deterministic core modules still used by unified `wizard`.

## Acceptance criteria

- `greentic-component wizard --mode create --execution dry_run` prints deterministic plan and writes answers file when requested
- `--qa-answers` runs non-interactive (no prompts)
- `--execution execute` performs real actions for all modes
- No `wizard new` / `wizard spec` remains in codebase or help output
- All new wizard strings are i18n keys in root `en.json`
- Tests cover new wizard modes and i18n key existence

## PR checklist (Codex-ready)

- [ ] Delete old wizard CLI subcommands + handlers
- [ ] Add unified `wizard` CLI plumbing + parsing tests
- [ ] Implement `RunMode` + `ExecutionMode`
- [ ] Implement `component-wizard-run/v1` answers schema + compatibility-free parser
- [ ] Extend spec builder for the 3 run modes
- [ ] Extend deterministic plan step model
- [ ] Implement executor steps using internal Rust command calls
- [ ] Add i18n keys to root `en.json` and replace raw wizard output strings
- [ ] Update wizard tests to cover modes
- [ ] Add i18n key-exists + required-locale tests
- [ ] Update README/help text for unified `wizard` only
