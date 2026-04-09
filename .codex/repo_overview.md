# Repository Overview

## 1. High-Level Purpose
- Rust workspace providing Greentic component authoring and packaging tooling: manifest/schema validation, capability enforcement, hashing/signing, and local inspection for WASI-Preview2 components.
- Ships a CLI (`greentic-component` plus doctor/hash/inspect tools) and supporting libraries for manifest validation, artifact fetching/verification, and a lightweight invocation library used for tests/dev (production host bindings live in runner/greentic-secrets).

## 2. Main Components and Functionality
- **Path:** `crates/greentic-component`  
  **Role:** Main public crate and CLI entrypoint. Exposes the component API (manifest parsing/validation, capability enforcement, telemetry, signing) and binaries (`greentic-component`, `component-doctor`, `component-hash`, `component-inspect`).  
  **Key functionality:** Manifest parsing/validation and schema handling; capability/limit management; provenance and security checks; signing and hash verification; prepare/loader helpers; CLI scaffolding and inspection tools (with `cli` feature).  
  **Notes:** Feature-gated modules for ABI inspection, describe payloads, loader/prepare, and CLI.

- **Path:** `crates/component-manifest`  
  **Role:** Schema and types for component manifests.  
  **Key functionality:** Validates component config schemas and exposes strongly typed manifest structures (`ComponentManifest`, exports, capabilities, compatibility metadata).

- **Path:** `crates/greentic-component-store`  
  **Role:** Artifact fetcher with caching and verification.  
  **Key functionality:** Fetches components from filesystem, HTTP (feature-gated), OCI, and Warg; computes cache keys; verifies digests/signatures; persists validated artifacts; extracts ABI/provider/capability metadata from WIT/producers metadata to enforce compatibility policies.  
  **Notes:** Provides verification policy/digest utilities reused by the main crate.

- **Path:** `crates/greentic-component-runtime`  
  **Role:** Runtime loader/invoker library built on Wasmtime components for local/test usage.  
  **Key functionality:** Loads components with policy controls, describes manifests, binds tenant configuration/secrets provided by the caller, and invokes exported operations with JSON inputs/outputs. Runtime invocation now targets `component@0.6.0` contract shapes (`InvocationEnvelope` / CBOR output decoding) and avoids `component_v0_4` tenant/impersonation paths. Secrets-store and other production host bindings belong in greentic-runner/greentic-secrets.

- **Path:** `ci/local_check.sh`, `.github/workflows/*`  
  **Role:** CI/local verification scripts and workflows (lint, tests, publish, release assets, auto-tag).  
  **Key functionality:** Mirrors CI locally; includes canonical WIT duplication guard and canonical bindings import guard (fails on `greentic_interfaces::bindings::*` and `bindings::greentic::*` usage), then build/tests, cargo publish (already-exists errors tolerated), binstall artifact builds, and creates/updates GitHub Releases using the plain version tag (e.g., `v0.4.10`). Auto-tag still bumps versions.

## 3. Work In Progress, TODOs, and Stubs
- Component manifests now allow optional `secret_requirements` (validated via `greentic-types` rules: SecretKey pattern, env/tenant scope, schema must be object). Keep downstream consumers/schema docs aligned if fields evolve.
- Runtime does not provide secrets-store; secret resolution/storage belongs to greentic-runner + greentic-secrets. HostState can carry injected secrets for tests/binder but no host bindings are exposed here.
- Templates and docs target `greentic:component/component@0.5.0` and accept expanded `supports` (`messaging`, `event`, `component_config`, `job`, `http`); keep downstream references in sync if interfaces bump again.
- Config inference + flow regeneration is integrated into `greentic-component build`; flows are embedded into `dev_flows` (FlowIR JSON) and manifests are updated with inferred `config_schema` when missing.
- Downstream consumers (packc/runner/deployer) must read `secret_requirements` from component manifests/metadata; this repo only validates and emits it.
- Component authoring now has clearer scope split:
  - `greentic-component new` can scaffold one or more canonical user operations at creation time via `--operation` and `--default-operation`.
  - `greentic-component wizard` remains the richer edit surface for existing wizard-generated components (`create`, `add_operation`, `update_operation`).
  - Interactive `wizard create` now starts with a minimal prompt set (name, output directory, advanced-setup yes/no) and only asks the richer authoring questions when advanced setup is enabled.
- Runtime capability authoring is now aligned with the canonical manifest model:
  - `greentic-component new` can scaffold filesystem/HTTP/state/telemetry/secret declarations directly into `component.manifest.json`.
  - `greentic-component wizard --mode create` accepts the same capability areas through answer fields/prompts.
  - Authoring writes `secret_requirements` and mirrors them into `capabilities.host.secrets.required`; telemetry permission and top-level telemetry config remain separate.
- Config schema authoring now has a shared scaffold path too:
  - `greentic-component new --config-field ...` and `greentic-component wizard --mode create` `config_fields` answers both write the same config shape into manifest `config_schema`, `schemas/component.schema.json`, and generated Rust `config_schema()`.
  - Supported scaffold field types are intentionally narrow (`string`, `bool`, `integer`, `number`) so the manifest JSON and exported `SchemaIr` stay aligned.

## 4. Broken, Failing, or Conflicting Areas
- No repo-wide failures are currently known from the checked surfaces; `cargo test -p greentic-component` and `ci/local_check.sh` passed after the latest authoring/runtime-capability updates.

## 5. Notes for Future Work
- If crates.io remains unreachable, publishing/packaging steps will continue to skip/fail; rerun when network is available.
- `.codex/PR-01-interfaces.md` defines a downstream policy: consumers should import WIT types from `greentic_interfaces::canonical` and avoid `greentic_interfaces::bindings::*` in app/library/tests/docs code.
