# Component Authoring Audit

This document is the PR-00 audit of the current `greentic-component` repository. It describes the authoring model that already exists so later work can extend it without creating parallel systems.

Canonical audit date: 2026-03-07.

## 1. Current architecture overview

The repository currently has two distinct authoring entrypoints:

- `greentic-component new`
  - Implemented in `crates/greentic-component/src/cmd/new.rs`.
  - Uses `ScaffoldEngine` in `crates/greentic-component/src/scaffold/engine.rs`.
  - Renders file-system templates from `crates/greentic-component/assets/templates/component/`.
- `greentic-component wizard`
  - CLI adapter in `crates/greentic-component/src/cmd/wizard.rs`.
  - Deterministic provider/core in `crates/greentic-component/src/wizard/mod.rs`.
  - Uses run modes `create`, `build_test`, and `doctor`.
  - Supports dry-run / execute, answer replay, and emitted answer documents.

The root CLI wiring lives in `crates/greentic-component/src/cli.rs`.

The component manifest model used by runtime-facing code lives in:

- `crates/greentic-component/src/manifest/mod.rs`
- `crates/greentic-component/schemas/v1/component.manifest.schema.json`

There is also an older and narrower manifest crate in `crates/component-manifest/`. That crate is still present for config-schema and legacy manifest helpers, but the canonical manifest model for this repo's main CLI/runtime path is the one in `crates/greentic-component/src/manifest/mod.rs`.

## 2. Wizard CLI implementation

### Entrypoints and routing

- Root CLI routes `wizard` to `cmd::wizard::run_cli` in `crates/greentic-component/src/cli.rs`.
- `cmd::wizard` supports subcommands `run`, `validate`, `apply`, plus hidden legacy `new`, in `crates/greentic-component/src/cmd/wizard.rs`.
- Interactive mode is only a frontend over existing run modes; it is not a separate command tree.

### Current wizard modes

The supported run modes today are:

- `create`
- `build_test`
- `doctor`

These are defined in `crates/greentic-component/src/cmd/wizard.rs`.

When run interactively, the current text menu presents:

- create new component
- build and test component
- doctor component

That menu is generated in `prompt_main_menu_mode` in `crates/greentic-component/src/cmd/wizard.rs`.

### Replay and answer document support

The wizard already supports deterministic replay:

- `--answers` / legacy `--qa-answers`
- `--emit-answers` / legacy `--qa-answers-out`
- schema version tagging
- plan output via `--plan-out`

Answer documents are represented in `crates/greentic-component/src/cmd/wizard.rs` by:

- `WizardRunAnswers`
- `AnswerDocument`

### Current data flow

- CLI parses arguments in `cmd/wizard.rs`.
- `build_run_output` selects the requested run mode.
- `create` mode builds a `WizardRequest` and delegates to `wizard::apply_scaffold`.
- `build_test` and `doctor` create deterministic plan envelopes with plan steps instead of directly mutating state.

## 3. Component scaffold generator

### `new` scaffold path

The `new` flow is template-driven:

- Request validation in `crates/greentic-component/src/cmd/new.rs`.
- Rendering and write path in `crates/greentic-component/src/scaffold/engine.rs`.
- Built-in template root in `crates/greentic-component/assets/templates/component/rust-wasi-p2-min/`.

The `new` scaffold creates a concrete `component.manifest.json` from the Handlebars template:

- `crates/greentic-component/assets/templates/component/rust-wasi-p2-min/component.manifest.json.hbs`

It also creates a conventional project layout including:

- `Cargo.toml`
- `Makefile`
- `README.md`
- `build.rs`
- `src/lib.rs`
- `src/qa.rs`
- `src/i18n.rs`
- `src/i18n_bundle.rs`
- `schemas/component.schema.json`
- `assets/i18n/en.json`
- `assets/i18n/locales.json`
- `tools/i18n.sh`
- `tests/conformance.rs`

The scaffolded output is covered by:

- `crates/greentic-component/tests/new_scaffold.rs`
- snapshots in `crates/greentic-component/tests/snapshots/`

### `wizard` scaffold path

The `wizard` create flow is not template-directory-driven. It uses in-process generation in:

- `crates/greentic-component/src/wizard/mod.rs`

That provider emits a deterministic plan envelope containing steps like:

- `EnsureDir`
- `WriteFiles`
- `BuildComponent`
- `TestComponent`
- `Doctor`

The provider is tested in:

- `crates/greentic-component/tests/wizard_provider_tests.rs`
- `crates/greentic-component/tests/wizard_tests.rs`

### Important audit finding

There are currently two scaffold systems:

- template-based `new`
- deterministic provider-based `wizard create`

They overlap heavily, but they are separate implementations. Any later PR that changes component creation must decide whether both paths remain supported or whether one becomes authoritative.

## 4. Component manifest model

### Canonical manifest definition

The canonical manifest for this repo's main CLI/runtime path is `ComponentManifest` in:

- `crates/greentic-component/src/manifest/mod.rs`

The JSON schema is:

- `crates/greentic-component/schemas/v1/component.manifest.schema.json`

### Required fields in the current schema

The schema currently requires:

- `id`
- `name`
- `version`
- `world`
- `describe_export`
- `config_schema`
- `operations`
- `supports`
- `profiles`
- `capabilities`
- `artifacts`
- `hashes`

Optional but supported fields include:

- `default_operation`
- `secret_requirements`
- `configurators`
- `limits`
- `telemetry`
- `provenance`
- `dev_flows`

### Versioning and ABI

Manifest versioning uses:

- semantic component version in `version`
- exported WIT world in `world`
- exported describe symbol in `describe_export`

The scaffold defaults to `greentic:component/component@0.6.0`, referenced in:

- `crates/greentic-component/src/scaffold/engine.rs`
- `docs/component_wizard.md`
- `docs/component-developer-guide.md`

### Manifest parsing and validation behavior

Validation currently enforces:

- non-empty `supports`
- at least one operation
- operation name pattern validity
- `default_operation` must match a declared operation
- relative wasm artifact paths
- typed capability validation
- limits validation
- provenance validation
- secret requirement validation

This logic lives in `crates/greentic-component/src/manifest/mod.rs`.

### Extra audit note: `dev_flows`

`dev_flows` is part of the schema and is preserved by validation, but it is not materialized as a typed field on `ComponentManifest`. Flow regeneration mutates it directly as JSON in:

- `crates/greentic-component/src/cmd/flow.rs`

This is an existing mixed model:

- typed manifest for core runtime fields
- raw JSON mutation for `dev_flows`

## 5. Operation model

### Where operations are defined

Operations are explicit today.

They exist in:

- manifest JSON under `operations`
- Rust type `greentic_types::component::ComponentOperation`
- describe payloads and doctor checks
- test harness validation before invocation

Relevant files:

- `crates/greentic-component/src/manifest/mod.rs`
- `crates/greentic-component/schemas/v1/component.manifest.schema.json`
- `crates/greentic-component/src/cmd/test.rs`
- `crates/greentic-component/src/cmd/doctor.rs`

### Operation structure

Each operation currently has:

- `name`
- `input_schema`
- `output_schema`

The manifest schema requires both schemas to be objects.

### Default operation behavior

`default_operation` is optional but, when present, must match one of the declared operations.

The flow-update path depends on this when multiple operations exist. See:

- `crates/greentic-component/src/cmd/flow.rs`

### How operations are exposed at runtime

There are two related runtime-facing models:

- Manifest-declared operations used by CLI/test/build tooling.
- Runtime exports discovered from component metadata and invocation bindings.

Invocation checks the requested operation against runtime-exported metadata in:

- `crates/greentic-component-runtime/src/invoker.rs`

The local `test` command also checks the requested operation name against manifest operations before invocation in:

- `crates/greentic-component/src/cmd/test.rs`

### Schema quality

Build and doctor enforce that operation schemas are not effectively empty, via:

- `crates/greentic-component/src/schema_quality.rs`
- `docs/cli.md`

## 6. Lifecycle / QA support

### Runtime lifecycle

Runtime lifecycle is modeled as wasm lifecycle exports:

- `init`
- `health`
- `shutdown`

This is represented by `Lifecycle` in:

- `crates/greentic-component/src/lifecycle.rs`

Lifecycle detection is performed during component preparation and doctor checks.

### QA lifecycle / authoring workflow

Separately from runtime lifecycle, scaffolded components expose QA-oriented operations such as:

- `qa-spec`
- `apply-answers`
- `i18n-keys`

These are visible in:

- `crates/greentic-component/assets/templates/component/rust-wasi-p2-min/component.manifest.json.hbs`
- `crates/greentic-component/tests/snapshots/new_scaffold__scaffold_manifest.snap`
- generated code snapshots in `crates/greentic-component/tests/snapshots/new_scaffold__scaffold_lib.snap`

Doctor validates those exported QA operations directly in:

- `crates/greentic-component/src/cmd/doctor.rs`

### Current lifecycle semantics

The repo currently has three distinct concepts that should not be conflated:

- wasm lifecycle exports (`init`, `health`, `shutdown`)
- QA modes (`default`, `setup`, `update`, `remove`)
- manifest operations (`handle_message`, `qa-spec`, `apply-answers`, `i18n-keys`, etc.)

The wizard provider also has internal QA-mode normalization in:

- `crates/greentic-component/src/wizard/mod.rs`

## 7. i18n support

The repository already has localized CLI support and component-scaffold i18n assets.

### CLI i18n

CLI translation support lives in:

- `crates/greentic-component/src/cmd/i18n.rs`

Catalog files live in:

- `crates/greentic-component/i18n/*.json`

The CLI:

- detects locale from `--locale`, environment, or system locale
- resolves against a fixed supported-locale list
- falls back to English

### Scaffolded component i18n

Both scaffolds generate component-side i18n assets such as:

- `assets/i18n/en.json`
- `assets/i18n/locales.json`
- `src/i18n.rs`
- `src/i18n_bundle.rs`
- `tools/i18n.sh`

Doctor validates i18n keys referenced by QA exports in:

- `crates/greentic-component/src/cmd/doctor.rs`

Tests cover locale completeness and key presence in:

- `crates/greentic-component/tests/i18n_required_locales.rs`
- `crates/greentic-component/tests/i18n_key_exists.rs`

## 8. Runtime capability declarations

### Canonical location

Runtime requirements are already declared canonically in the manifest under:

- `capabilities`
- `secret_requirements`
- optional top-level `telemetry`
- optional `limits`

The canonical schema is in:

- `crates/greentic-component/schemas/v1/component.manifest.schema.json`

Typed validation is in:

- `crates/greentic-component/src/capabilities.rs`
- `crates/greentic-component/src/manifest/mod.rs`

### Supported capability areas in current schema

`capabilities.wasi` supports:

- `filesystem`
- `env`
- `random`
- `clocks`

`capabilities.host` supports:

- `secrets`
- `state`
- `messaging`
- `events`
- `http`
- `telemetry`
- `iac`

### Secrets

There are currently two related secret declarations:

- top-level `secret_requirements`
- `capabilities.host.secrets.required`

Both are supported today. This is the main normalization ambiguity in the current model.

### Filesystem

Filesystem access is already structured under:

- `capabilities.wasi.filesystem.mode`
- `capabilities.wasi.filesystem.mounts`

Validation requires mounts for non-`none` filesystem modes in `crates/greentic-component/src/capabilities.rs`.

### Network

Network access is represented today as host HTTP capability:

- `capabilities.host.http.client`
- `capabilities.host.http.server`

There is no generic `network` object in the canonical schema.

### Telemetry

Telemetry exists in two places:

- `capabilities.host.telemetry.scope`
- top-level `telemetry` configuration (`span_prefix`, attributes, `emit_node_spans`)

This is another area where later PRs must be careful to distinguish:

- permission to emit telemetry
- telemetry configuration metadata

### State / session / storage

State access is declared under:

- `capabilities.host.state.read`
- `capabilities.host.state.write`
- `capabilities.host.state.delete`

The canonical state-store contract is documented in:

- `docs/component_state.md`

Current parsing normalizes `delete: true` to imply `write: true` in `crates/greentic-component/src/manifest/mod.rs`.

### Whether the wizard captures capabilities today

The current wizard CLI accepts `required_capabilities` and `provided_capabilities` as answer fields for scaffold generation, but this is not the same as a full manifest capability editor.

The scaffolded component manifest itself already includes default capability declarations.

Relevant files:

- `crates/greentic-component/src/cmd/wizard.rs`
- `crates/greentic-component/src/wizard/mod.rs`
- `docs/component_wizard.md`

## 9. Flow compatibility metadata

Flow compatibility already exists today via the manifest `supports` field.

Allowed values in the schema are:

- `messaging`
- `event`
- `component_config`
- `job`
- `http`

This is defined in:

- `crates/greentic-component/schemas/v1/component.manifest.schema.json`

The default scaffold currently sets `supports` to `["messaging"]`.

There are also host capability declarations that overlap conceptually with flow integration:

- `capabilities.host.messaging`
- `capabilities.host.events`

These are not the same thing:

- `supports` declares flow-kind compatibility
- host capability declarations express runtime permissions/surfaces

## 10. Profiles

Profiles already exist in the canonical manifest as:

- `profiles.default`
- `profiles.supported`

They are validated in:

- `crates/greentic-component/src/manifest/mod.rs`

The schema requires at least one supported profile.

The default scaffold uses:

- `default: "stateless"`
- `supported: ["stateless"]`

Current CLI/runtime code surfaces profiles through inspect/reporting, but this repo does not currently define a rich authoring workflow for profile-specific overrides inside the wizard.

There is also a separate runtime policy concept called `Profile` in capability enforcement code:

- `crates/greentic-component/src/security.rs`

That `Profile` is an execution-policy object, not the same thing as manifest `profiles`.

## 11. Directory structure conventions

The repository's scaffolded component convention today is closer to:

- `src/`
- `schemas/`
- `assets/i18n/`
- `tools/`
- `tests/`
- `component.manifest.json`
- `Cargo.toml`
- `Makefile`

Notably:

- `examples/` is not part of the default generated component layout.
- `qa/` is not a top-level directory; QA logic is scaffolded into `src/qa.rs`.
- i18n assets live under `assets/i18n/`, not a top-level `i18n/` directory in generated components.

The root repo itself does contain CLI i18n catalogs in `crates/greentic-component/i18n/`, but generated components use `assets/i18n/`.

## 12. Known architectural tensions

The audit found several existing tensions that later PRs must treat carefully.

### Two scaffold systems

There is no single scaffold implementation today:

- `new` uses Handlebars templates
- `wizard create` uses the provider module

### Two secret-related declaration surfaces

Secrets are represented both as:

- top-level `secret_requirements`
- `capabilities.host.secrets.required`

### Capability vs configuration telemetry split

Telemetry permission and telemetry config are separate today:

- `capabilities.host.telemetry`
- top-level `telemetry`

### Typed manifest plus raw `dev_flows`

Core manifest fields are parsed into a typed Rust struct, but `dev_flows` is manipulated as raw JSON by flow tooling.

## 13. Practical implications for later PRs

Later work should assume the following current truths:

- operations are already explicit and canonical
- flow compatibility already exists as `supports`
- profiles already exist
- runtime capability declarations already exist in a canonical manifest schema
- lifecycle, QA modes, and operations are related but distinct concepts
- the wizard is currently mode-based, not a CRUD submenu system

Any PR that introduces:

- new manifest sections
- new lifecycle terminology
- new flow-compat metadata
- a parallel capability format
- a separate operation-definition system

would be extending beyond the current architecture and should be treated as a design change, not as a simple normalization of existing behavior.
