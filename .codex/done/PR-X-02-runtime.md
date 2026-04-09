PR-02 — Runtime Capability Authoring Alignment
# PR-02 — Align Wizard and Manifest Authoring for Existing Runtime Capability Metadata

## Depends on

PR-00 — Component Authoring Audit

Codex must read:

- `docs/component_authoring_audit.md`

before implementing this PR.

This PR must reuse the existing canonical manifest schema and capability model.

Do NOT introduce:

- a parallel configuration format
- pseudo-capability sections that do not match the schema
- product-specific capability types

---

# Purpose

The audit found that runtime capability metadata is **already present** in the canonical component manifest.

The main gap is not the absence of a capability model. The real gaps are:

- authoring flows do not capture the existing model consistently
- some capability-related concepts are split across multiple fields
- current docs do not describe the audited model clearly enough

This PR should align authoring flows, docs, and validation around the capability model that already exists.

---

# Current Audited State

The audit found that canonical runtime capability-related fields already exist in `component.manifest.json`.

## 1. Canonical capability declaration already exists

The current schema already defines:

- `capabilities.wasi.filesystem`
- `capabilities.wasi.env`
- `capabilities.wasi.random`
- `capabilities.wasi.clocks`
- `capabilities.host.secrets`
- `capabilities.host.state`
- `capabilities.host.messaging`
- `capabilities.host.events`
- `capabilities.host.http`
- `capabilities.host.telemetry`
- `capabilities.host.iac`

## 2. Additional related runtime metadata already exists

The current manifest also supports:

- `secret_requirements`
- `limits`
- top-level `telemetry`

## 3. Current ambiguity is real but narrow

The audit found the main normalization ambiguities are:

- secrets are declared both in `secret_requirements` and `capabilities.host.secrets.required`
- telemetry is split between permission (`capabilities.host.telemetry`) and config (`telemetry`)

This PR should focus on those real ambiguities, not on inventing a new capability model.

---

# Scope

This PR is about:

- documenting the current canonical capability model
- making component authoring flows capture the existing fields consistently
- normalizing known overlapping fields where appropriate
- improving validation only where the current architecture already supports it

It is not a greenfield capability-design PR.

---

# Canonical Capability Areas

The following sections describe the audited capability areas that this PR must reuse directly.

## 1. Secrets

Current capability-related fields:

- `capabilities.host.secrets.required`
- `secret_requirements`

This PR must explicitly decide and document:

- which field is canonical for authoring going forward
- whether one remains as compatibility data
- how wizard/scaffold updates keep the two aligned if both remain supported

Do not replace these fields with a new `secrets:` block.

Configuration values themselves must remain outside the manifest and continue to be handled through setup/test/runtime flows rather than manifest-stored secrets.

## 2. Filesystem access

Filesystem is already modeled canonically under:

- `capabilities.wasi.filesystem.mode`
- `capabilities.wasi.filesystem.mounts`

This PR must reuse that structure exactly.

Do not replace it with a new generic:

- `filesystem.access`
- `filesystem.paths`

model unless the schema itself is intentionally changed.

## 3. Network access

Network access is currently represented by HTTP capability fields:

- `capabilities.host.http.client`
- `capabilities.host.http.server`

There is no canonical top-level `network` object today.

This PR must reuse the existing HTTP capability structure unless the schema is intentionally expanded as a separate design change.

## 4. Telemetry

The audit found two telemetry-related surfaces:

- permission scope in `capabilities.host.telemetry.scope`
- telemetry config in top-level `telemetry`

This PR must preserve that distinction.

Do not collapse them carelessly.

If normalization is needed, document whether:

- the capability field is permission-only
- the top-level field is runtime/config metadata

## 5. State / session / storage

State access is already modeled under:

- `capabilities.host.state.read`
- `capabilities.host.state.write`
- `capabilities.host.state.delete`

This PR must reuse that structure directly.

Do not invent a new generic `state.session` / `state.durable` model unless the canonical schema is intentionally redesigned.

---

# Authoring Flow Changes

The audit found two authoring surfaces:

- `greentic-component new`
- `greentic-component wizard`

This PR must state whether capability-authoring changes apply to:

- both
- wizard only
- scaffold defaults only

Do not silently update one path and leave the other inconsistent without documenting that choice.

---

# Wizard / Scaffold Integration

Authoring flows should be able to collect the existing canonical capability metadata and write it into the manifest consistently.

Prompts and answer structures must map directly to existing schema fields.

Examples of acceptable prompt targets:

- host HTTP client access
- filesystem mode and mounts
- state read/write/delete
- telemetry scope
- secret requirements using the chosen canonical secret-authoring field

Examples of unacceptable prompt targets:

- synthetic `network.outbound`
- synthetic `state.session`
- synthetic `telemetry.logs/metrics/traces`

unless the canonical schema is intentionally changed in the same PR.

---

# Manifest Normalization

This PR should only normalize areas that actually need normalization based on the audit.

Primary candidates:

## 1. Secret declaration duplication

Codex should determine whether to:

- keep both `secret_requirements` and `capabilities.host.secrets.required`
- make one canonical and derive/synchronize the other
- document one as legacy compatibility

The result must be explicit and documented.

## 2. Telemetry split

Codex should document and preserve the distinction between:

- telemetry permission
- telemetry runtime config

If a code change is needed, it must remain compatible with the current schema and runtime model.

## 3. Existing scaffold defaults

Scaffolded manifests and wizard-generated manifests should use the same audited capability conventions unless an intentional divergence is documented.

---

# Doctor / Validation Integration

Validation changes in this PR must stay within the current architecture.

Good candidates:

- malformed capability declarations
- invalid filesystem mounts for enabled filesystem access
- invalid state declarations
- inconsistent secret declarations if both secret fields remain supported
- invalid telemetry scope/config combinations if such checks already map to existing fields

Avoid speculative rules tied to non-canonical concepts, such as:

- requiring a special setup lifecycle because secrets exist
- requiring new metadata blocks that do not exist in the schema

Doctor should validate what the current manifest and runtime model can actually express.

---

# Documentation

Add or update documentation describing the audited capability model and the chosen normalization rules.

Recommended target:

- `docs/component_runtime_capabilities.md`

That document should explain:

- the canonical capability fields
- how secrets are declared
- how filesystem access is declared
- how HTTP/network access is declared
- how state access is declared
- how telemetry permission differs from telemetry config
- how scaffold/wizard authoring writes those fields

Also keep these docs aligned where relevant:

- `docs/component_authoring_audit.md`
- `docs/component_wizard.md`
- `docs/component-developer-guide.md`
- `docs/component_state.md`

---

# Tests

Add or update tests covering the implemented capability-authoring path(s).

Examples:

- wizard/scaffold creation with capability metadata
- manifest updates for capability fields
- normalization behavior for secrets if both fields remain supported
- doctor/validation coverage for malformed capability declarations

If only one authoring path is updated, tests must reflect that scope explicitly.

---

# Guardrails

Codex must NOT:

- invent a new capability DSL
- add pseudo manifest sections that do not match the current schema
- collapse permission and configuration concepts without justification
- introduce product-specific capability categories
- break compatibility with existing manifest validation/runtime behavior

All work must extend the audited architecture rather than replacing it.

---

# Deliverables

Expected changes may include:

- authoring-flow updates for canonical capability fields
- manifest normalization for overlapping secret declarations
- documentation
- targeted doctor/validation updates
- tests
