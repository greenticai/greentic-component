PR-01 — Wizard and Scaffold Operation Authoring Alignment
# PR-01 — Extend Component Authoring Flows for Operation and Manifest Editing

## Depends on

PR-00 — Component authoring audit.

Codex must read:

- `docs/component_authoring_audit.md`

before implementing this PR.

---

# Goal

Extend component authoring flows so authors can add and update operations using the **existing canonical manifest model**.

This PR must work with the current repository architecture described in the audit:

- `greentic-component new` is template-based
- `greentic-component wizard` is deterministic and mode-based
- operations already exist canonically in `component.manifest.json`

Do NOT introduce:

- a second operation definition system
- a second manifest format
- Greentic-X-specific behavior

---

# Current Audited State

The audit found the following current behavior:

## 1. Two authoring entrypoints exist today

- `greentic-component new`
  - template-driven scaffold path
- `greentic-component wizard`
  - deterministic plan-based path
  - current modes: `create`, `build_test`, `doctor`

This PR must explicitly decide whether a change applies to:

- both authoring flows
- wizard only
- scaffold templates only

Codex must not accidentally update one path while leaving the other inconsistent unless the PR explicitly documents that choice.

## 2. Operations already exist in the canonical manifest

The audited operation model is already explicit:

- `operations[]`
- `default_operation`

Each operation currently has:

- `name`
- `input_schema`
- `output_schema`

This PR must extend that model rather than inventing a parallel authoring model.

## 3. Flow compatibility already exists

Flow compatibility is already represented by:

- `supports`

This PR must reuse that field.

## 4. QA and lifecycle are distinct concepts

The audit found three separate concepts:

- manifest operations
- QA-oriented exports such as `qa-spec` / `apply-answers` / `i18n-keys`
- wasm lifecycle exports `init` / `health` / `shutdown`

This PR must not conflate them.

---

# Scope

This PR is about **operation authoring and manifest editing**.

It is not a general redesign of the wizard.

It should cover:

- collecting operation metadata during component creation
- adding operations to an existing component
- updating operation metadata for an existing component
- keeping scaffolds and manifests aligned with the canonical manifest schema

It may also include:

- updating `supports`
- updating `default_operation`
- keeping schema scaffolding consistent with existing repo conventions

It must not require inventing new top-level manifest sections.

---

# Authoring Surface

Codex must first choose and document one of these implementation approaches:

## Option A

Extend both:

- `greentic-component new`
- `greentic-component wizard`

so both authoring flows can create/update operations consistently.

## Option B

Make `greentic-component wizard` the richer operation-authoring surface and leave `new` as a simpler baseline scaffold.

If Codex chooses Option B, the PR must clearly document:

- that `new` remains a baseline scaffold
- that richer operation editing lives in `wizard`
- how users are expected to move from scaffold creation to operation editing

Do not silently diverge the two flows without documenting the intended long-term model.

---

# Wizard / CLI Changes

The current wizard is mode-based, not a CRUD submenu system.

Any new behavior must integrate with the existing CLI architecture.

Codex may implement this via:

- new wizard run modes
- extended create-mode questions
- additional deterministic plan steps
- dedicated command handlers that still fit the current CLI shape

Do not implement this PR as a fake menu spec disconnected from the current code.

If an interactive menu is extended, it must remain a frontend over the actual CLI/run-mode architecture.

---

# Create Component Flow

When creating a component, authoring flows should be able to collect and write canonical manifest data using the audited model.

## Required create-time metadata

Use existing canonical fields where they already exist.

Examples:

- component name
- output directory
- version
- ABI / world-related scaffold defaults already used by the repo
- `supports`
- operations
- `default_operation`

Only add metadata that has a canonical home in the current manifest/schema.

Do not add ad hoc fields such as maintainer/description unless this PR also changes the canonical manifest schema intentionally.

## Operation creation during scaffold

The authoring flow should allow defining one or more operations during creation.

Each authored operation must map to the existing manifest model.

Minimum collected information:

- operation name
- input schema scaffold
- output schema scaffold

Optional metadata may be collected only if it maps cleanly to existing generated code or docs.

If the current canonical manifest does not store fields such as display label or operation description, do not invent them here.

## Repeated entry

The create flow may support:

- create first operation
- optionally add another

but the resulting data must still be written into the single canonical `operations[]` list.

---

# Add Operation to Existing Component

This PR should support adding an operation to an existing component.

That flow must:

- locate the canonical manifest
- append a new operation under `operations`
- update `default_operation` only when necessary
- keep generated scaffold/runtime code consistent where the current scaffold expects explicit operation wiring

Codex must not update only the manifest if the scaffolded component source also requires explicit operation registration.

The audit found that scaffolded code contains explicit operation wiring and extension points, so manifest edits alone may be insufficient for scaffold-generated components.

---

# Update Existing Operation

This PR should support updating an existing operation's canonical metadata.

Allowed targets include:

- operation name
- input schema
- output schema
- default operation selection

If source code wiring or generated operation lists must also be updated, do so consistently with the existing scaffold/runtime model.

Avoid broad source rewrites when a focused manifest/code update is enough.

---

# Directory and Schema Conventions

The audit found these current conventions:

- generated components use `schemas/io/` today
- scaffold manifests currently embed operation schemas inline
- the repo does not currently establish `examples/<operation>/...` as a standard generated layout

Therefore:

- do not assume a per-operation examples directory already exists
- do not force a new layout unless this PR intentionally standardizes it

If Codex introduces per-operation schema or example files, it must:

- document the new convention
- update all affected authoring paths consistently
- preserve compatibility with current manifest/build/doctor behavior

---

# Replayability

The wizard already supports deterministic replay through answer documents and plan output.

This PR must reuse that existing replay model.

If create-mode operation authoring is added, answer documents and plan generation must include:

- operation definitions
- `supports`
- `default_operation`

Do not add a separate replay framework.

---

# QA / Lifecycle Guardrail

This PR must not blur the distinction between:

- authoring a normal operation
- authoring QA behavior
- exposing wasm lifecycle exports

In particular:

- do not treat `setup`, `update`, or `remove` as automatically canonical manifest operations unless the current scaffold/runtime actually models them that way
- do not invent new lifecycle models

If this PR touches QA-oriented operations, it must do so explicitly and in terms of the audited existing constructs.

---

# Tests

Add or update tests covering the implemented path(s).

Examples:

- component creation with authored operations
- add-operation flow for an existing component
- update-operation flow
- manifest updates
- any required source-code wiring updates
- replay/answer document support for operation authoring

If only one authoring path is updated, tests must make that limitation explicit.

---

# Documentation

Update the relevant docs to reflect the final implemented authoring model.

At minimum, keep these aligned:

- `docs/component_authoring_audit.md`
- `docs/component_wizard.md`
- `docs/component-developer-guide.md`

If `new` and `wizard` intentionally diverge in capability, document that clearly.

---

# Guardrails

Codex must NOT:

- invent a second operation definition system
- introduce a new manifest format
- conflate operations with lifecycle exports
- add unsupported metadata fields without intentionally extending the canonical manifest schema
- implement a fake menu abstraction that ignores the current CLI architecture
- break existing scaffolded components

All work must extend the architecture documented in the audit.

---

# Deliverables

Expected changes may include:

- wizard CLI updates and/or `new` scaffold updates
- manifest update logic for operations
- scaffold/runtime source updates where operation registration is explicit
- replay/answer document updates
- tests
- documentation updates
