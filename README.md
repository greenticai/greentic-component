# Greentic components

Greentic components are portable WASM building blocks that declare their capabilities up front (manifest + schemas) and expose a tiny, consistent surface area (describe + invoke). This repo ships the authoring CLI, manifest tooling, and flow generator that keep components easy to create, configure, and ship.

If you just want the CLI:

```bash
rustup target add wasm32-wasip2
cargo binstall greentic-component           # or: cargo install --path crates/greentic-component --features cli
```

Full CLI reference lives in `docs/cli.md` — skim it when you want every flag and subcommand.
Docs index lives in `docs/README.md`, including the v0.6 vision and legacy/deprecation map.

## Why components?

- **Everything is a component**: describe inputs/outputs, wire a single invoke surface, and let flows orchestrate them.
- **Predictable config**: manifests + JSON schemas drive rich prompts and defaults; config flows are regenerated for you.
- **Portable**: wasm32-wasip2 targets + explicit capabilities mean minimal host assumptions.

## Quick start (copy/paste)

```bash
greentic-component new \
  --name hello-world \
  --org ai.greentic \
  --path ./hello-world \
  --non-interactive \
  --no-check   # drop --no-check to run the initial cargo check

cd hello-world
greentic-component flow update   # regenerates dev_flows with defaults from schemas/io/input.schema.json
```

What you get:

- `component.manifest.json` with operations, schemas, and a default dev_flow in the YGTc v2 shape.
- `src/lib.rs` already wired with the exports macro.
- A WIT world (`greentic:component/component@0.6.0`) so config inference works.

## Author the logic

Scaffolds keep the glue in a macro so you only implement two functions:

```rust
use greentic_interfaces_guest::component::node::InvokeResult;
use greentic_interfaces_guest::component_entrypoint;

component_entrypoint!({
    manifest: crate::describe_payload,
    invoke: crate::handle_message,
    invoke_stream: true,
});

pub fn describe_payload() -> String {
    serde_json::json!({
        "component": {
            "name": "hello-world",
            "org": "ai.greentic",
            "version": "0.1.0",
            "world": "greentic:component/component@0.6.0",
            "schemas": {
                "component": "schemas/component.schema.json",
                "input": "schemas/io/input.schema.json",
                "output": "schemas/io/output.schema.json"
            }
        }
    })
    .to_string()
}

pub fn handle_message(operation: String, input: String) -> InvokeResult {
    InvokeResult::Ok(format!("hello-world::{operation} => {}", input.trim()))
}
```

`describe_payload` returns the manifest JSON; `handle_message` receives the resolved operation name and the raw input JSON string and returns `InvokeResult`.

## Config flows (YGTc v2)

`greentic-component flow update` (and `build`) regenerate `dev_flows` based on your manifest and input schema defaults. The default flow now emits the new YGTc v2 shape — keyed by operation, no `component.exec` wrapper:

```json
{
  "node_id": "hello-world",
  "node": {
    "handle_message": {
      "input": {
        "input": "Hello from hello-world!"
      }
    },
    "routing": [{ "to": "NEXT_NODE_PLACEHOLDER" }]
  }
}
```

Routing stays untouched so downstream tools can rewire the placeholder. If any required field in `schemas/io/input.schema.json` lacks a default, flow generation fails loudly instead of emitting an invalid stub.

## Build, test, ship

- `greentic-component build` validates the manifest, refreshes dev_flows, and builds the wasm (honoring `--cargo`/`CARGO` if you need a custom toolchain).
- `greentic-component test --wasm ./component.wasm --op <op> --input ./input.json` invokes a component locally with in-memory state/secrets (see `docs/cli.md` for secrets/state flags).
- `greentic-component doctor ./target/wasm32-wasip2/release/component.wasm --manifest component.manifest.json` prints schema/hash/world/lifecycle/capability health.
- `greentic-component templates` lists built-ins and user templates under `~/.greentic/templates/component/*`.

See `docs/cli.md` for deeper switches (offline mode, schema inference knobs, store fetch, etc.).

## Learn more

- Docs index: `docs/README.md`
- Vision (canonical v0.6): `docs/vision/v0.6.md`
- Legacy + deprecation map: `docs/vision/legacy.md`
- CLI details and doctor output: `docs/cli.md`
- Component developer guide: `docs/component-developer-guide.md`
- Manifest and flow regeneration tests live under `crates/greentic-component/tests/*` (including the README quickstart).
- Examples are exercised in CI so the snippets above stay correct.

let prepared = prepare_component("./component.manifest.json")?;
pack_builder.with_component(prepared.to_pack_entry()?);
runner.add_component(prepared.to_runner_config());
```

`PreparedComponent` exposes both `to_pack_entry()` (hashes, manifest JSON, first schema) and `to_runner_config()` (wasm path, world, capabilities/limits/telemetry, redactions/defaults, describe payload), which lets higher-level tooling plug in with almost no extra glue.

### Running Checks

```bash
# Format sources
cargo fmt

# Lint (clippy is run across all targets/features)
cargo clippy --all-targets --all-features

# Run tests for all crates
cargo test
```

### Local Checks

Developers only need one entrypoint to mirror CI:

```bash
# Fast checks (quiet, online, non-strict)
bash ci/local_check.sh

# CI-equivalent (strict, verbose)
LOCAL_CHECK_ONLINE=1 LOCAL_CHECK_STRICT=1 LOCAL_CHECK_VERBOSE=1 bash ci/local_check.sh
```

Toggles remain available when you need a targeted run:

```bash
# Default: online, non-strict
bash ci/local_check.sh

# Force offline mode (skip schema drift curl)
LOCAL_CHECK_ONLINE=0 bash ci/local_check.sh

# Enable strict mode (enforces online schema + full feature builds/tests)
LOCAL_CHECK_ONLINE=1 LOCAL_CHECK_STRICT=1 bash ci/local_check.sh

# Temporarily skip the smoke scaffold (not recommended)
LOCAL_CHECK_SKIP_SMOKE=1 bash ci/local_check.sh

# Show every command
LOCAL_CHECK_VERBOSE=1 bash ci/local_check.sh
```

The script runs in online mode by default, gracefully skips network-dependent
steps when `LOCAL_CHECK_ONLINE=0`, scaffolds a fresh component (doctor +
`cargo check --target wasm32-wasip2`, `cargo build --target wasm32-wasip2 --release`,
then inspect) whenever registry access is available, and fails fast when
`LOCAL_CHECK_STRICT=1` is set (even if smoke scaffolding is skipped due to an
offline environment). Strict mode also forces workspace-wide
`cargo build/test --all-features`; otherwise those heavyweight steps are scoped
to the `greentic-component` crate for a faster inner loop.

The smoke phase runs twice with complementary dependency modes:

- `local` injects workspace `path =` overrides so regressions surface before publish.
- `cratesio` uses only published crates; lockfile/tree/build steps emit `[skip]` when
  `LOCAL_CHECK_ONLINE=0` (or the crates.io probe fails) unless strict mode is enabled,
  in which case the same conditions are treated as hard failures.

Both variants execute the exact commands the CI job uses:

```bash
GREENTIC_DEP_MODE=<mode> cargo run --locked -p greentic-component --features cli -- \
  new --name local-check --org ai.greentic --path "$TMPDIR/<mode>" \
  --non-interactive --no-check --json
(cd "$TMPDIR/<mode>" && cargo generate-lockfile)
(cd "$TMPDIR/<mode>" && cargo tree -e no-dev --locked \
    | tee target/local-check/tree-<mode>.txt >/dev/null)
cargo run --locked -p greentic-component --features cli --bin component-doctor -- "$TMPDIR/<mode>"
(cd "$TMPDIR/<mode>" && cargo check --target wasm32-wasip2 --locked)
(cd "$TMPDIR/<mode>" && cargo build --target wasm32-wasip2 --release --locked)
cargo run --locked -p greentic-component --features cli --bin component-hash -- \
  "$TMPDIR/<mode>/component.manifest.json"
cargo run --locked -p greentic-component --features cli --bin component-inspect -- \
  --json "$TMPDIR/<mode>/component.manifest.json"
```

Per-mode cargo trees are stored under `target/local-check/tree-<mode>.txt`
(override via `LOCAL_CHECK_TREE_DIR=...`) so failures always include a snapshot
of the resolved dependencies.

## Releases & Publishing

- Versions are sourced directly from each crate's `Cargo.toml`.
- Pushing to `master` tags any crate whose version changed as `<crate-name>-v<semver>`.
- The publish workflow then attempts to release updated crates to crates.io.
- Publishing is idempotent: reruns succeed even when the crate version already exists.

## Component Store

The new `greentic-component` crate exposes a `ComponentStore` that can register filesystem paths and OCI references, materialise component bytes, and persist them in a content-addressed cache (`~/.greentic/components` by default).

```rust
use greentic_component::{CompatPolicy, ComponentStore};

let policy = CompatPolicy {
    required_abi_prefix: "greentic-abi-0".into(),
    required_capabilities: vec!["messaging".into()],
};

let mut store = ComponentStore::with_cache_dir(None, policy);
store.add_fs("local", "./build/my_component.wasm");
store.add_oci("remote", "ghcr.io/acme/greentic-tools:1.2.3");

let component = store.get("local").await?;
println!("id={} size={}", component.id.0, component.meta.size);
```

- Cache keys are `sha256:<digest>`; a locator index speeds up repeated fetches.
- OCI layers are selected when the media type advertises `application/wasm` or `application/octet-stream`.
- Capability and ABI compatibility checks are enforced before cache writes succeed.

## Testing Overview

Automated tests cover multiple layers:

- **Manifest validation** (`crates/component-manifest/tests/manifest_valid.rs`): ensures well-formed manifests pass and malformed manifests (duplicate capabilities, invalid secret requirements) fail.
- **Component store** (`crates/greentic-component-store/tests/*.rs`): verifies filesystem listings, caching behaviour, and HTTP fetching via a lightweight test server.
- **Runtime binding** (`crates/greentic-component-runtime/src/binder.rs` tests): validates schema enforcement and secret resolution logic.
- **Host imports** (`crates/greentic-component-runtime/src/host_imports.rs` tests): exercises telemetry gating plus the HTTP fetch host import, including policy denial and successful request/response handling.

Add new tests alongside the relevant crate to keep runtime guarantees tight.

## Component Manifest v1

`crates/greentic-component` now owns the canonical manifest schema (`schemas/v1/component.manifest.schema.json`) and typed parser. Manifests describe an opaque `id`, human name, semantic `version`, the exported WIT `world`, and the function to call for describing configuration. Artifact metadata captures the relative wasm path plus a required `blake3` digest. Optional sections describe enforced `limits`, `telemetry` attributes, and build `provenance` (builder, commit, toolchain, timestamp).

- **Capabilities** — structured WASI + host declarations (filesystem/env/random/clocks plus secrets/state/messaging/events/http/telemetry/IaC). The `security::enforce_capabilities` helper compares a manifest against a runtime `Profile` and produces precise denials (e.g. `host.secrets.required[OPENAI_API_KEY]`). Component manifests optionally declare structured `secret_requirements` for pack tooling while keeping backwards compatibility when no secrets are needed.
- **Describe loading order** — `describe::load` first tries to decode the embedded WIT world from the wasm, falls back to a JSON blob emitted by an exported symbol (e.g. `describe`), and finally searches `schemas/v1/*.json` for provider-supplied payloads. The resulting `DescribePayload` snapshots all known schema versions.
- **Redaction hints** — schema utilities walk arbitrary JSON Schema documents and surface paths tagged with `x-redact`, `x-default-applied`, and `x-capability`. These hints are used by greentic-dev/runner to scrub transcripts or explain defaulted fields.

See `greentic_component::manifest` and `greentic_component::describe` for the Rust APIs, and consult the workspace tests for concrete usage.

The schema is published at <https://greenticai.github.io/greentic-component/schemas/v1/component.manifest.schema.json>. A minimal manifest looks like:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://greenticai.github.io/greentic-component/schemas/v1/component.manifest.schema.json",
  "id": "com.greentic.examples.echo",
  "name": "Echo",
  "version": "0.1.0",
  "world": "greentic:component/node@0.1.0",
  "describe_export": "describe",
  "supports": ["messaging", "event"],
  "profiles": {
    "default": "stateless",
    "supported": ["stateless"]
  },
  "capabilities": {
    "wasi": {
      "filesystem": {
        "mode": "none",
        "mounts": []
      },
      "random": true,
      "clocks": true
    },
    "host": {
      "messaging": {
        "inbound": true,
        "outbound": true
      }
    }
  },
  "artifacts": {"component_wasm": "component.wasm"},
  "hashes": {"component_wasm": "blake3:..."}
}
```

### Command-line tools (optional `cli` feature)

```
cargo run --features cli --bin component-inspect ./component.manifest.json --json
cargo run --features cli --bin component-doctor ./component.manifest.json
```

`component-inspect` emits a structured JSON report with manifest metadata, BLAKE3 hashes, lifecycle detection, describe payloads, and redaction hints sourced from `x-redact` annotations. Add `--strict` when warnings should become hard failures (default mode only exits non-zero on actual errors so smoke jobs can keep running while still surfacing warnings on stderr). `component-doctor` executes the full validation pipeline (schema validation, hash verification, world/ABI probe, lifecycle detection, describe resolution, and redaction summary) and exits non-zero on any failure—perfect for CI gates.

Further CLI details: see docs/cli.md.

## Host HTTP Fetch

The runtime now honours `HostPolicy::allow_http_fetch`. When enabled, host imports will perform outbound HTTP requests via `reqwest`, propagate headers, and base64-encode response bodies for safe transport back to components.

## Future Work

- Implement OCI/Warg store backends.
- Expand integration coverage with real Wasm components once fixtures are available.
- Support streaming invocations via the Greentic component interface.

Contributions welcome—please run `cargo fmt`, `cargo clippy --all-targets --all-features`, and `cargo test` before submitting changes.

## Security

See [SECURITY.md](SECURITY.md) for guidance on `x-redact`, capability declarations, and protecting operator logs.

