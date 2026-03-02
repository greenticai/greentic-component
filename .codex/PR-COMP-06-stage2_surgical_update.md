# greentic-component: Stage 2 — Surgical Update (apply audit inputs)

    **Pre-req:** Stage 1 audit completed and its findings copied into the “Audit Inputs” section below.

    ## Objective
    Implement the minimal set of changes to support:
    - Stable AnswerDocument import/export (envelope)
    - Schema identity + version (`schema_id`, `schema_version`) for this wizard
    - Non-interactive execution via `--answers <file>`
    - Optional migration via `--migrate`
    - i18n keys (schema uses keys; labels resolved by locale)
    - Preserve/alias existing CLI paths where required

    ## Repo description
    Owns component-level wizards (schema/plan/execute/migrate) + i18n prompting

    ## Audit Inputs (paste from Stage 1)
    Fill these **before coding**:

    - Wizard command path(s):
      - `crates/greentic-component/src/cli.rs:31-36`, `crates/greentic-component/src/cli.rs:73-76`
      - Runtime handler: `crates/greentic-component/src/cmd/wizard.rs:90-170`
      - Legacy compat (`wizard new <name> --out ...` hidden args): `crates/greentic-component/src/cmd/wizard.rs:23-29`, `crates/greentic-component/src/cmd/wizard.rs:173-213`, parser coverage `crates/greentic-component/src/cli.rs:214-235`
    - Current flags (locale/answers):
      - Locale is global: `--locale <LOCALE>` on root CLI (`crates/greentic-component/src/cli.rs:23-24`)
      - Answers in/out are currently `--qa-answers` and `--qa-answers-out` (`crates/greentic-component/src/cmd/wizard.rs:40-43`)
      - Current execution split is `--execution dry-run|execute` and `--dry-run` alias (`crates/greentic-component/src/cmd/wizard.rs:32-39`, `crates/greentic-component/src/cmd/wizard.rs:138-164`)
      - No `--answers`, `--emit-answers`, `--schema-version`, or `--migrate` yet
    - Schema location/model:
      - Answers file struct: `WizardRunAnswers { schema, mode, fields }` in `crates/greentic-component/src/cmd/wizard.rs:74-79`
      - Required schema string today: `"component-wizard-run/v1"` in `crates/greentic-component/src/cmd/wizard.rs:512-517`
      - Interactive QA schema built in code via JSON (`build_qa_spec`) in `crates/greentic-component/src/cmd/wizard.rs:622-705`
      - Plan model is `WizardPlanEnvelope`/`WizardStep` in `crates/greentic-component/src/wizard/mod.rs:52-104`
    - Execution model (plan/apply):
      - `run()` builds a plan then either writes plan JSON (dry-run) or executes steps (`crates/greentic-component/src/cmd/wizard.rs:119`, `crates/greentic-component/src/cmd/wizard.rs:138-164`)
      - Execution side-effects in `execute_run_plan` for build/doctor/test and `wizard::execute_plan` for file ops (`crates/greentic-component/src/cmd/wizard.rs:433-491`, `crates/greentic-component/src/wizard/mod.rs:202-257`)
      - No migration pathway except light key normalization in provider normalize step (`crates/greentic-component/src/wizard/mod.rs:290-320`)
    - Tests to update/add:
      - Update CLI/wizard behavior tests: `crates/greentic-component/tests/wizard_tests.rs`, `crates/greentic-component/src/cli.rs` (parser tests)
      - Keep integration coverage with doctor: `crates/greentic-component/tests/doctor_wizard_tests.rs`
      - Keep deterministic plan snapshot/provider coverage: `crates/greentic-component/tests/wizard_provider_tests.rs` + `tests/snapshots/wizard_provider_tests__scaffold_plan_snapshot.snap`
      - Update i18n key coverage if new keys introduced: `crates/greentic-component/tests/i18n_key_exists.rs`

    ## Proposed changes (minimal)
    ### 1) Add/standardize AnswerDocument envelope
    - Implement (or adapt) a small struct matching:
      - `wizard_id`, `schema_id`, `schema_version`, `locale`, `answers`, `locks`
    - Ensure read/write JSON is stable and documented.
    - Do **not** centralize in a shared repo unless later desired.

    ### 2) CLI flags + semantics (surgical)
    Implement/alias:
    - `--answers <FILE>`: load AnswerDocument; run non-interactive validate/apply
    - `--emit-answers <FILE>`: write AnswerDocument produced (interactive or merged)
    - `--schema-version <VER>`: pin version for interactive mode
    - `--migrate`: if AnswerDocument version older, migrate (and optionally re-emit)

    **Compatibility rule:** if existing flags already exist, keep them and add aliases.

    ### 3) Schema identity + versioning
    - Define stable identifiers:
      - `wizard_id`: e.g. `greentic-component.wizard.<purpose>`
      - `schema_id`: e.g. `greentic-component.<purpose>`
      - `schema_version`: start at `1.0.0` unless audit shows existing versioning
    - Ensure interactive renders and validators emit these IDs/versions into AnswerDocument.

    ### 4) Validate vs apply split
    Prefer separate subcommands:
    - `wizard validate --answers ...`
    - `wizard apply --answers ...`
    If existing model differs, adapt with minimal surface changes but keep semantics.

    ### 5) Migration
    - If breaking changes are present or anticipated, add a migration function:
      - input: old AnswerDocument
      - output: new AnswerDocument
    - If no breaking change yet, implement a stub that returns identity but wires the mechanism.

    ### 6) i18n wiring
    - Ensure schema/question definitions use i18n keys
    - Ensure `--locale` controls resolution only (answers stay stable)

    ## Acceptance criteria
    - [x] `wizard run` interactive still works
    - [x] `wizard validate --answers answers.json` works (no side effects)
    - [x] `wizard apply --answers answers.json` works (side effects)
    - [x] `wizard run --emit-answers out.json` produces AnswerDocument with correct ids/versions
    - [x] `wizard validate --answers old.json --migrate` succeeds (if old version) and can re-emit migrated doc
    - [x] Tests updated/added per audit notes, minimal and focused

    ## Implementation notes (apply from audit)
    - **Files to touch (from audit):**
      - <LIST FILES>
    - **Tests to touch/add (from audit):**
      - <LIST TESTS>

    ## Risk controls
    - No large refactors; keep changes localized
    - Preserve existing UX defaults
    - Avoid schema mega-merges; keep nested docs for composition

    ## Common target behavior (all repos)

**Goal:** Standardize wizard execution and portability via a stable AnswerDocument envelope and consistent CLI semantics, while keeping schema ownership local to each wizard.

### AnswerDocument envelope (portable JSON)
```json
{
  "wizard_id": "greentic.pack.wizard.new",
  "schema_id": "greentic.pack.new",
  "schema_version": "1.1.0",
  "locale": "en-GB",
  "answers": { "...": "..." },
  "locks": { "...": "..." }
}
```

### Required CLI semantics
All wizards should converge on these flags and semantics (names can vary only if you provide compatibility aliases):
- `--locale <LOCALE>`: affects i18n rendering only; **answers must remain stable IDs/values**
- `--answers <FILE>`: non-interactive input (AnswerDocument)
- `--emit-answers <FILE>`: write AnswerDocument produced (interactive or merged)
- `--schema-version <VER>`: pin schema version used for interactive rendering/validation
- `--migrate`: allow automatic migration of older AnswerDocuments (including nested ones where applicable)
- Separate `validate` vs `apply` paths (subcommands or flags), recommended:
  - `wizard validate --answers ...`
  - `wizard apply --answers ...`

### Versioning rules
- Patch/minor: backwards compatible additions (defaults) only
- Major: breaking changes require migration logic
- Avoid flattening composed schemas into one mega-schema; prefer nested AnswerDocuments for composed flows.

### i18n rules
- Schema uses i18n keys; runtime resolves by locale
- Answers never depend on localized labels; only stable values/IDs

Date: 2026-03-02
