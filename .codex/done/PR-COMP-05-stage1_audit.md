# greentic-component: Stage 1 - Audit (Wizard schema/plan/execute/migrate + delegation)

**Purpose:** Produce a factual snapshot of the current wizard implementation so Stage 2 can be surgical.  
**Date:** 2026-03-02

## A. CLI surface

### Exact help output
`greentic-component --help`:

```text
Toolkit for Greentic component developers

Usage: greentic-component [OPTIONS] <COMMAND>

Commands:
  new        Scaffold a new Greentic component project
  wizard     Component wizard helpers
  templates  List available component templates
  doctor     Run component doctor checks
  inspect    Inspect manifests and describe payloads
  hash       Recompute manifest hashes
  build      Build component wasm + update config flows
  test       Invoke a component locally with an in-memory state/secrets harness
  flow       Flow utilities (config flow regeneration)
  store      Interact with the component store

Options:
      --locale <LOCALE>
  -h, --help             Print help
  -V, --version          Print version
```

`greentic-component wizard --help`:

```text
Component wizard helpers

Usage: greentic-component wizard [OPTIONS]

Options:
      --locale <LOCALE>
      --mode <MODE>                    [default: create] [possible values: create, build-test, doctor]
      --execution <EXECUTION>          [default: execute] [possible values: dry-run, execute]
      --dry-run
      --qa-answers <answers.json>
      --qa-answers-out <answers.json>
      --plan-out <plan.json>
      --project-root <PATH>            [default: .]
      --template <TEMPLATE_ID>
      --full-tests
      --json
  -h, --help                           Print help
```

Notes:
- There are no user-facing `wizard` subcommands in help output.
- Legacy compatibility accepts hidden positional `wizard new <name> --out <PATH>` and maps it into `--mode create` plus synthesized answers.

### Findings table
| Item | Current behavior | Files/lines |
|---|---|---|
| Wizard command path(s) | Top-level `Commands::Wizard(WizardArgs)` dispatches to `cmd::wizard::run`. Legacy `wizard new` is handled by hidden args + compat mapping (not a clap subcommand). | `crates/greentic-component/src/cli.rs:31-36`, `crates/greentic-component/src/cli.rs:73-76`, `crates/greentic-component/src/cmd/wizard.rs:23-54`, `crates/greentic-component/src/cmd/wizard.rs:173-213`, `crates/greentic-component/src/cli.rs:214-235` |
| Flags for locale | `--locale` is global on root CLI; wizard inherits it in help output. Locale is initialized from argv then from parsed CLI. | `crates/greentic-component/src/cli.rs:23-24`, `crates/greentic-component/src/cli.rs:63`, `crates/greentic-component/src/cli.rs:71` |
| Flags for answers import/export | Import: `--qa-answers <answers.json>`; export: `--qa-answers-out <answers.json>`. Not named `--answers`/`--emit-answers` yet. | `crates/greentic-component/src/cmd/wizard.rs:40-43`, `crates/greentic-component/src/cmd/wizard.rs:98-101`, `crates/greentic-component/src/cmd/wizard.rs:121-136` |
| Validate/apply split | No separate validate/apply commands. Split is `--execution dry-run|execute` (or `--dry-run` alias). | `crates/greentic-component/src/cmd/wizard.rs:32-39`, `crates/greentic-component/src/cmd/wizard.rs:92-96`, `crates/greentic-component/src/cmd/wizard.rs:138-164` |
| Non-zero exit handling | Uses `anyhow::bail!` and `?`; invalid schema/mode mismatch/missing required args in non-interactive flow return errors and non-zero exit. Clap parse errors exit via `err.exit()`. | `crates/greentic-component/src/cmd/wizard.rs:107-117`, `crates/greentic-component/src/cmd/wizard.rs:246-249`, `crates/greentic-component/src/cmd/wizard.rs:507-521`, `crates/greentic-component/src/cli.rs:66-69` |

## B. Schema + questions

### Findings table
| Item | Current approach | Files/lines |
|---|---|---|
| Schema identity | Answers file uses `schema: "component-wizard-run/v1"` (string). QA interactive spec id is `component.wizard.run.<mode>`. No `wizard_id`/`schema_id` envelope fields. | `crates/greentic-component/src/cmd/wizard.rs:74-79`, `crates/greentic-component/src/cmd/wizard.rs:512-517`, `crates/greentic-component/src/cmd/wizard.rs:526`, `crates/greentic-component/src/cmd/wizard.rs:699` |
| Schema versioning | QA interactive spec includes `"version": "1.0.0"`. Answers schema is versioned only by literal string suffix `/v1`. | `crates/greentic-component/src/cmd/wizard.rs:701`, `crates/greentic-component/src/cmd/wizard.rs:512-517` |
| Question model | Defined in Rust JSON builder (`build_qa_spec`) using `serde_json::json!`; provider-side scaffold spec also exists as typed `ComponentQaSpec`/`Question` with namespaced IDs. | `crates/greentic-component/src/cmd/wizard.rs:622-705`, `crates/greentic-component/src/wizard/mod.rs:106-174` |
| Validation rules | Input answers validated by schema equality + mode match. Field-level validation includes `ComponentName::parse`, ABI normalization, output path checks, required prompt validation, and enum validation. | `crates/greentic-component/src/cmd/wizard.rs:107-117`, `crates/greentic-component/src/cmd/wizard.rs:270-305`, `crates/greentic-component/src/cmd/wizard.rs:507-521`, `crates/greentic-component/src/cmd/wizard.rs:785-800`, `crates/greentic-component/src/cmd/wizard.rs:840-857`, `crates/greentic-component/src/cmd/wizard.rs:986-1049` |
| Defaults | Mode default `create`; execution default `execute`; many question defaults (`component`, output dir, `0.6.0`, template default, full tests flag). | `crates/greentic-component/src/cmd/wizard.rs:30-33`, `crates/greentic-component/src/cmd/wizard.rs:635`, `crates/greentic-component/src/cmd/wizard.rs:642`, `crates/greentic-component/src/cmd/wizard.rs:650`, `crates/greentic-component/src/cmd/wizard.rs:664`, `crates/greentic-component/src/cmd/wizard.rs:685`, `crates/greentic-component/src/cmd/wizard.rs:881-905` |
| i18n keys | Questions and outputs use i18n keys (`cli.wizard.*`). QA schema includes `title_i18n` fields, and locale-resolved catalog is wired through `greentic-qa-lib`. Catalog presence tested. | `crates/greentic-component/src/cmd/wizard.rs:533-542`, `crates/greentic-component/src/cmd/wizard.rs:631-704`, `crates/greentic-component/src/cmd/wizard.rs:731-733`, `crates/greentic-component/tests/i18n_key_exists.rs:6-43` |

## C. Plan/execute/migrate

### Findings table
| Item | Current approach | Files/lines |
|---|---|---|
| Plan representation | Uses `WizardPlanEnvelope { plan_version, metadata, target_root, plan(meta, steps) }` plus `WizardStep` enum. | `crates/greentic-component/src/wizard/mod.rs:16`, `crates/greentic-component/src/wizard/mod.rs:52-104` |
| Apply/execution | `run()` builds output plan then executes per-step in execute mode. Scaffold file effects happen via `wizard::execute_plan`; build/doctor/test steps run via internal command calls / `cargo test`. | `crates/greentic-component/src/cmd/wizard.rs:119`, `crates/greentic-component/src/cmd/wizard.rs:150-152`, `crates/greentic-component/src/cmd/wizard.rs:433-491`, `crates/greentic-component/src/wizard/mod.rs:202-257` |
| Validation-only path | Dry-run writes plan JSON and does not execute side effects. In non-interactive dry-run, `--plan-out` is mandatory. | `crates/greentic-component/src/cmd/wizard.rs:138-149`, `crates/greentic-component/src/cmd/wizard.rs:236-249` |
| Migration | No explicit answer-document migration mechanism (`--migrate` absent). Only minor normalization of legacy answer keys (`component.features.enabled`/`enabled`) when generating scaffold files. | `crates/greentic-component/src/cmd/wizard.rs:23-54`, `crates/greentic-component/src/wizard/mod.rs:290-320`, `crates/greentic-component/src/wizard/mod.rs:322-351` |
| Locks/reproducibility | No `locks` field in answers docs. Reproducibility is plan-centric: deterministic step ordering/map encoding plus template digest; guarded by snapshot tests. | `crates/greentic-component/src/wizard/mod.rs:398-444`, `crates/greentic-component/src/wizard/mod.rs:474-483`, `crates/greentic-component/tests/wizard_provider_tests.rs:29-115`, `crates/greentic-component/tests/snapshots/wizard_provider_tests__scaffold_plan_snapshot.snap:1-15` |

## D. Tests

### Findings table
| Test | What it covers | Command |
|---|---|---|
| `tests/wizard_tests.rs` | Wizard create execute, dry-run behavior, `qa-answers-out` file writing. | `cargo test -p greentic-component wizard_tests -- --nocapture` |
| `tests/doctor_wizard_tests.rs` | Wizard scaffold + doctor integration path; verifies doctor failure before wasm exists. | `cargo test -p greentic-component doctor_wizard_tests -- --nocapture` |
| `tests/wizard_provider_tests.rs` + snapshot | Deterministic scaffold plan snapshot, execute_plan writes files, namespaced question IDs in provider spec. | `cargo test -p greentic-component wizard_provider_tests -- --nocapture` |
| `tests/i18n_key_exists.rs` | Required wizard i18n keys exist in root `i18n/en.json`. | `cargo test -p greentic-component i18n_key_exists -- --nocapture` |
| `tests/i18n_required_locales.rs` | Crate/root locale catalogs remain in sync. | `cargo test -p greentic-component i18n_required_locales -- --nocapture` |
| CLI parser tests in `src/cli.rs` | Parses wizard options and legacy `wizard new` argument shape. | `cargo test -p greentic-component parses_wizard -- --nocapture` |

## Constraints and compatibility notes for Stage 2
- Preserve legacy `wizard new` compatibility path (`LEGACY_COMMAND/LEGACY_NAME/--out`) while introducing new flags/aliases.
- Current user-facing answers format is `WizardRunAnswers { schema, mode, fields }`; Stage 2 should add envelope compatibility rather than hard break.
- Existing semantics tie validation/apply to `dry-run|execute`; adding `validate/apply` subcommands should keep aliases or behavioral parity.
- Current interactive detection is TTY-based only (`stdin` and `stdout` both terminals).
- `--locale` is global; do not duplicate conflicting locale flags at subcommand level.
