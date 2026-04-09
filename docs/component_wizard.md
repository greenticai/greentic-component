# Component Wizard

The component wizard generates a ready-to-edit component@0.6.0 scaffold with Greentic conventions baked in. It focuses on deterministic templates and leaves runtime integration to follow-up work.

Legacy naming/compatibility details are in `docs/vision/legacy.md`.

**Quickstart**
1. `greentic-component wizard apply --mode create --project-root .`
2. `cd hello-component`
3. `make wasm`
4. `greentic-component doctor ./dist/hello-component__0_6_0.wasm`

**What You Get**
- `Cargo.toml` with ABI metadata.
- `src/lib.rs` with guest trait wiring and `export_component_v060!`.
- `src/descriptor.rs` for `get-component-info` and `describe`.
- `src/schema.rs` for SchemaIR and canonical CBOR helpers.
- `src/runtime.rs` for CBOR run handling.
- `src/qa.rs` with QA specs and `apply-answers`.
- `src/i18n.rs` key registry.
- `assets/i18n/en.json` default bundle for i18n keys.
- A `Makefile` with `build`, `test`, `fmt`, `clippy`, `wasm`, and `doctor` targets.
- The generated `wasm` target delegates through `greentic-component build`, so the built artifact gets the embedded manifest custom section `greentic.component.manifest.v1`.

**ABI Versioning + WASM Naming**
The wizard stores ABI version in `Cargo.toml` under `[package.metadata.greentic]` and uses it to name the wasm artifact:
- Output: `dist/<name>__<abi_with_underscores>.wasm`
- Example: `dist/hello-component__0_6_0.wasm`

**Wizard Modes**
The CLI supports `--mode create|add_operation|update_operation|build_test|doctor` and command aliases `run|validate|apply`.
- `validate` (or `--validate`) is validation-only / dry-run.
- `apply` (or `--apply`) performs side effects.
- `run` keeps legacy execution behavior and still accepts `--execution dry-run|execute`.

Use `--answers` for deterministic non-interactive replay, and `--emit-answers` to persist an AnswerDocument envelope. Legacy `--qa-answers` and `--qa-answers-out` remain available for compatibility.

**Operation Authoring**
- Interactive `create` now starts with a minimum setup: component name, output location, and an `Advanced setup` yes/no prompt.
- If `Advanced setup` is `no`, the remaining scaffold inputs stay on defaults.
- If `Advanced setup` is `yes`, the wizard asks the richer authoring questions for operations, runtime capabilities, secrets, and config schema fields.
- Within advanced setup, secrets are also gated: the wizard asks whether secrets are needed and only then prompts for secret keys/scope/format.
- `create` can scaffold multiple user operations when `answers.json` includes an `operations` array or an `operation_names` comma-separated string.
- `create` can also scaffold canonical runtime capability metadata for filesystem, messaging, events, HTTP, state, telemetry permission/config, and secret requirements.
- `add_operation` appends a new user operation to `component.manifest.json` and updates the generated `src/lib.rs` operation metadata block.
- `update_operation` renames an existing user operation and keeps manifest/default-operation metadata aligned.

Current guardrail: the richer operation-edit workflow is implemented on the wizard path. `greentic-component new` now supports create-time user operation scaffolding with `--operation` and `--default-operation`, but it does not provide add/update flows for existing components.

Capability notes:
- Telemetry permission is written to `capabilities.host.telemetry.scope`.
- Top-level telemetry config is only written when a span prefix is supplied.
- Secret authoring writes `secret_requirements` and mirrors those entries into `capabilities.host.secrets.required`.

Example:
`greentic-component wizard apply --mode create --answers ./answers.json --emit-answers ./answers.out.json`

**Doctor Validation**
`greentic-component doctor` validates the built wasm artifact for:
- required WIT exports
- QA modes and i18n coverage
- strict SchemaIR + schema hash
- presence and validity of the embedded manifest section on built artifacts

When a wizard-generated component has been built successfully, `doctor` now expects the Wasm to contain the embedded manifest section and treats it as required artifact-local truth.

**Flow Integration**
After implementing your component, use Greentic Flow tooling to connect the component to a distribution client and flow registry. This keeps the wizard focused on scaffolding while flow integration is handled in the flow repo.
