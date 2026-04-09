# CLI quick guide

Canonical docs target: `component@0.6.0`.
Legacy compatibility notes are centralized in `docs/vision/legacy.md`.

Practical notes for the main `greentic-component` subcommands: what they do, key flags, and why you might tweak them. Pair this with `--help` for the full list of options.

Global:
- `--help` shows usage for the CLI or a subcommand.
- `--version` prints the CLI version.

## new
- Purpose: scaffold a new component repo from a template (default: `rust-wasi-p2-min`).
- Usage: `greentic-component new --name hello-world --org ai.greentic [--template rust-wasi-p2-min] [--path ./hello-world] [--version 0.1.0] [--license MIT] [--wit-world greentic:component/component@0.6.0] [--operation render,sync-state] [--default-operation sync-state] [--filesystem-mode none|read_only|sandbox] [--filesystem-mount assets:assets:/assets] [--messaging-inbound] [--messaging-outbound] [--events-inbound] [--events-outbound] [--http-client] [--state-read] [--telemetry-scope tenant|pack|node] [--telemetry-span-prefix component.demo] [--telemetry-attribute key=value] [--secret-key API_TOKEN] [--secret-env dev] [--secret-tenant default] [--secret-format text] [--non-interactive] [--no-git] [--no-check] [--json]`.
- Options:
- `--version <semver>` sets the initial component version (default: `0.1.0`).
- `--license <id>` sets the license identifier embedded in generated sources (default: `MIT`).
- `--wit-world <name>` sets the exported WIT world name (default: `greentic:component/component@0.6.0`).
- `--operation <name[,name...]>` declares one or more user operations to scaffold into `operations[]`; repeat the flag or pass a comma-separated list. If omitted, `handle_message` is used.
- `--default-operation <name>` writes the canonical `default_operation` field and must match one of the declared user operations.
- Capability authoring:
- `--filesystem-mode` and `--filesystem-mount` write `capabilities.wasi.filesystem`.
- `--messaging-inbound` / `--messaging-outbound` write `capabilities.host.messaging`.
- `--events-inbound` / `--events-outbound` write `capabilities.host.events`.
- `--http-client` / `--http-server` write `capabilities.host.http`.
- `--state-read` / `--state-write` / `--state-delete` write `capabilities.host.state`.
- `--telemetry-scope` writes telemetry permission to `capabilities.host.telemetry.scope`.
- `--telemetry-span-prefix` and `--telemetry-attribute` write top-level `telemetry` config.
- `--secret-key`, `--secret-env`, `--secret-tenant`, and `--secret-format` write top-level `secret_requirements` and mirror the same requirements into `capabilities.host.secrets.required`.
- Tips: keep `--no-check` off in CI unless you already built the wasm; use `--template` to point at custom templates (listed via `templates`); `--no-git` skips the init/commit step. The CLI prints each step (scaffold, git, cargo check) and shows cargo check duration; the first check can take a while while the wasm toolchain downloads.

## templates
- Purpose: list available scaffold templates (built-in + user-provided).
- Usage: `greentic-component templates [--json]`.
- Tips: use `--json` to drive tooling/selection in scripts; template paths are shown for local overrides.

## wizard
- Purpose: run wizard workflows on the deterministic plan core (`create`, `add_operation`, `update_operation`, `build_test`, `doctor`).
- Usage: `greentic-component wizard [run|validate|apply] --mode create|add_operation|update_operation|build_test|doctor [--execution dry-run|execute] [--answers answers.json] [--emit-answers answers.json] [--schema-version x.y.z] [--migrate] [--project-root path] [--template id] [--full-tests]`.
- Tips: use `validate` (or `--validate`) to emit plan JSON without side effects; use `apply` (or `--apply`) to execute side effects; use `--answers` for non-interactive replay and `--emit-answers` to persist an AnswerDocument envelope. Legacy `--qa-answers` and `--qa-answers-out` remain supported for compatibility.
- Interactive create flow: the text wizard now asks only for name, output path, and `Advanced setup` first. If you answer `no`, the rest of the create-time authoring inputs stay at defaults.
- Operation authoring: `create` accepts authored operations from answer documents using either an `operations` array or an `operation_names` comma-separated string; `add_operation` appends a new user operation to the manifest and generated wizard scaffold source; `update_operation` renames an existing user operation while keeping `default_operation` aligned when requested. `new` now supports create-time operation scaffolding too, but `wizard` remains the richer edit surface for existing components.
- Capability authoring: `create` also accepts canonical runtime capability answer fields for filesystem, messaging, events, HTTP, state, telemetry permission/config, and secret requirements. See [component_runtime_capabilities.md](/projects/ai/greentic-ng/greentic-component/docs/component_runtime_capabilities.md).

## inspect
- Purpose: inspect a component manifest or a self-describing 0.6.0 wasm/describe artifact.
- Usage:
  - Manifest flow: `greentic-component inspect <manifest-or-dir> [--manifest path] [--json] [--strict]`
  - Describe flow: `greentic-component inspect <wasm> [--json] [--verify]` or `greentic-component inspect --describe <file.cbor> [--json] [--verify]`
- Output: manifest flow prints id, wasm path, world match, hash, supports, profiles, lifecycle exports, capabilities, limits. Wasm inspection now also reports whether the embedded custom section `greentic.component.manifest.v1` is present, whether its hash verifies, a summary of the embedded projection, and comparison verdicts against the external manifest and `describe()` when available. Describe flow prints component info + operations + SchemaIR summaries; `--verify` checks schema_hash values.
- Tips: point `--manifest` if the wasm and manifest are not co-located; use `--describe` to inspect a prebuilt artifact without executing wasm; `--json` is CI-friendly and now includes embedded-manifest status when inspecting a wasm artifact.

## hash
- Purpose: recompute and write `hashes.component_wasm` in the manifest.
- Usage: `greentic-component hash [component.manifest.json] [--wasm path]`.
- Tips: run after rebuilding the wasm; `--wasm` overrides `artifacts.component_wasm`.

## build
- Purpose: one-stop: infer/validate config schema, regenerate dev_flows, build wasm, refresh artifacts/hashes.
- Usage: `greentic-component build [--manifest path] [--cargo path] [--no-flow] [--no-infer-config] [--no-write-schema] [--force-write-schema] [--no-validate] [--json] [--permissive]`.
- Behavior: unless `--no-flow`, calls the same regeneration as `flow update` (fails if required defaults are missing). Builds with cargo (override via `--cargo` or `CARGO`). For `component@0.6.0`, the canonical manifest is then embedded into the built Wasm as deterministic CBOR in the custom section `greentic.component.manifest.v1`, and the build fails if embed/write-back verification does not match the canonical manifest used for the build. Removes `config_schema` from the written manifest if it was only inferred and `--no-write-schema` is set. Emits `dist/<name>__<abi>.describe.cbor` + `.json` when `describe()` is available.
- Tips: keep `--no-flow` off to avoid stale dev_flows; use `--json` for CI summaries; set `CARGO` to a wrapper if you need a custom toolchain.
- Schema gate: the command refuses to build when any `operations[].input_schema`/`output_schema` is effectively empty (literal `{}`, unconstrained `{"type":"object"}`, or boolean `true`). Pass `--permissive` to keep building while emitting `W_OP_SCHEMA_EMPTY` warnings.

## test
- Purpose: invoke a component locally with an in-memory state-store and secrets harness.
- Usage: `greentic-component test --wasm ./component.wasm --op render --input ./input.json [--state inmem] [--pretty] [--state-dump] [--manifest path] [--output out.json] [--trace-out ./trace.json]`.
- Behavior: uses `greentic:state/store@1.0.0` in-memory storage scoped by tenant + flow/session prefix; secrets are loaded from `.env`, JSON, or `--secret` flags when declared in the manifest. State/secrets calls are denied when capabilities are not declared. Failures emit JSON with a stable `code`.
- Options:
- `--world <world>` overrides the component world (default: `greentic:component/component@0.6.0`).
- `--manifest <path>` overrides the manifest location (defaults to next to the wasm).
- `--input-json <json>` supplies inline JSON (repeatable; conflicts with `--input`).
- `--config <path|json>` supplies component config (file path or inline JSON).
- `--output <path>` writes the JSON result to a file.
- `--trace-out <path>` writes a trace file (overrides `GREENTIC_TRACE_OUT`).
- `--pretty` pretty-prints JSON output.
- `--raw-output` prints legacy output without the JSON envelope (deprecated compatibility flag; prefer default JSON envelope for new tooling).
- `--state <mode>` selects the state backend (only `inmem` supported).
- `--state-dump` prints the in-memory state after invocation.
- `--dry-run <bool>` toggles dry-run mode (default: true, disables HTTP and FS writes).
- `--allow-http` allows outbound HTTP when not in dry-run.
- `--allow-fs-write` allows filesystem writes when not in dry-run.
- `--timeout-ms <ms>` sets the invoke timeout (default: 2000).
- `--max-memory-mb <mb>` sets the memory limit (default: 256).
- `--state-set <key=base64>` seeds in-memory state (repeatable).
- `--step` adds a step marker for multi-step runs (repeatable).
- `--secrets <path>` loads secrets from a .env file.
- `--secrets-json <path>` loads secrets from a JSON map file.
- `--secret <key=value>` provides a secret inline (repeatable).
- `--env <id>` sets the environment id (default: `dev`).
- `--tenant <id>` sets the tenant id (default: `default`).
- `--team <id>`, `--user <id>`, `--flow <id>`, `--node <id>`, `--session <id>` set optional exec context identifiers.
- `--verbose` prints extra diagnostics (including generated session id).
- Tips: use `--input-json` for inline payloads; add `--secrets` and `--secret` to provide values; seed bytes with `--state-set KEY=BASE64`; pass `--verbose` to print the generated session id; repeat `--op`/`--input` with `--step` between them for multi-step runs; set `GREENTIC_TRACE_OUT` to capture a runner-compatible trace file.

## flow update
- Purpose: regenerate `dev_flows.default/custom` from manifest + input schema using YGTc v2 shape.
- Usage: `greentic-component flow update [--manifest path] [--no-infer-config] [--no-write-schema] [--force-write-schema] [--no-validate]`.
- Behavior: picks the operation via `default_operation` (or only op), uses node_id = manifest.name, operation-keyed node with `input` and routing to `NEXT_NODE_PLACEHOLDER`; fails if required fields lack defaults or if `mode/kind` is `tool`.
- Tips: run after editing schemas/operations; leave `--no-write-schema` off when you want inferred schemas persisted.

## store fetch
- Purpose: fetch a component artifact into a local directory using the distributor resolver.
- Usage: `greentic-component store fetch --out <dir|file.wasm> <source> [--cache-dir dir]`.
- Tips: `<source>` may be `file://`, `oci://`, `repo://`, `store://`, or a local path (including a directory containing `component.manifest.json` or `component.wasm`); if the source provides `component.manifest.json`, it is written alongside the wasm; use `--cache-dir` for repeated fetches.

## doctor
- Purpose: validate a wasm + manifest pair and print a health report.
- Usage: `greentic-component doctor <wasm-or-dir> [--manifest path] [--permissive]`.
- Output highlights:
  - `manifest schema: ok` — manifest conforms to schema; fix missing/invalid fields otherwise.
  - `hash verification: ok` — manifest hash matches wasm bytes; run `greentic-component hash` or `build` after rebuilding wasm.
  - `world check: ok` — wasm metadata matches manifest `world`; rebuild with correct WIT world if it fails.
  - `embedded_manifest` — built artifacts are expected to contain `greentic.component.manifest.v1`. Missing, malformed, or hash-mismatched embedded metadata is an error when doctor is run against a built Wasm.
  - `lifecycle exports: init=<bool> health=<bool> shutdown=<bool>` — optional lifecycle hooks present in the wasm. Implement `on_start`/`on_stop`/health in your guest bindings if your host expects them; omit if not needed.
  - `describe payload versions` — number of describe payloads embedded (typically 1).
  - `redaction hints` — `x-redact` markers. Logs/inspectors can leak secrets/PII if fields aren’t redacted; add `x-redact` to sensitive fields so hosts/tooling can mask them. “none” means nothing will be redacted automatically.
- `defaults applied` — config defaults auto-applied; set defaults on required fields inside the selected operation’s `input_schema` so dev flows can be regenerated.
  - `supports` — flow kinds declared; adjust `supports` in the manifest.
  - `capabilities declared` — wasi/host surfaces requested; keep minimal for least privilege.
  - `limits configured` — whether resource limits are present; set `limits` for guardrails.
- Tips: run after `build` to catch hash/world drift; point `--manifest` if wasm and manifest differ; errors on validation/hash/world/lifecycle issues; pass `--permissive` to treat empty operation schemas as warnings (`W_OP_SCHEMA_EMPTY`).
- Embedded metadata rule: if a built wasm exists, doctor now treats the embedded manifest as required artifact-local truth and compares it with the canonical external manifest and `describe()` on overlapping fields. In source-only / no-artifact contexts, the older “no wasm available” behavior still applies.

### Lifecycle exports (how-to)
The doctor report surfaces lifecycle booleans based on your wasm. To expose them, implement the generated guest trait for your world (or use a macro) to provide `on_start`/`on_stop`/health handlers. If your host expects these hooks, add implementations; otherwise they can remain false.

Doctor output reference
-----------------------
- `manifest schema: ok` — Manifest JSON validated against the published schema; fix missing/invalid fields if not ok.
- `hash verification: ok (blake3:...)` — Manifest hash matches wasm; run `greentic-component hash`/`build` after rebuilding wasm to refresh.
- `world check: ok (...)` — Wasm exports/metadata match manifest `world`; rebuild with the correct WIT world if it fails.
- `embedded_manifest: ok` — The embedded `greentic.component.manifest.v1` section exists, decodes, and matches the expected payload hash.
- `lifecycle exports: init=<bool> health=<bool> shutdown=<bool>` — Optional lifecycle hooks detected; implement guest bindings if the host expects startup/health/shutdown.
- `describe payload versions: N` — Number of embedded describe payloads (typically 1).
- `redaction hints: ...` — `x-redact` paths; add to sensitive fields to prevent leaking secrets/PII in logs/inspectors.
- `defaults applied: ...` — Config defaults applied; set defaults in the selected operation’s `input_schema` (required fields should usually have defaults).
- `supports: [...]` — Flow kinds declared; set in manifest.
- `capabilities declared: ...` — Requested wasi/host surfaces; keep minimal for least privilege.
- `limits configured: true/false` — Resource limits present; set `limits` to give hosts guardrails.
- `operation schemas` — Empty `operations[].input_schema`/`output_schema` cause doctor to fail unless `--permissive` is used, which emits `W_OP_SCHEMA_EMPTY` warnings instead.
