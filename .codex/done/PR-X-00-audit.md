PR-00-audit — greentic-component wizard capabilities audit
# PR-00 — Audit: greentic-component Wizard, Manifest, and Component Authoring Model

## Purpose

Before implementing any new functionality, perform a **full audit of the current greentic-component repository**.

The goal is to understand the **existing component authoring model** and **wizard capabilities** so that future PRs extend the system **without introducing parallel or conflicting implementations**.

Codex **must not implement new features in this PR**.  
Only produce an audit document.

---

# Audit Scope

Inspect the following areas of the repository.

## 1. Wizard CLI implementation

Identify where the component wizard is implemented.

Current wizard entrypoint example:


greentic-component wizard

create new component

build and test component

doctor component


Audit:

- CLI entrypoints
- wizard modules
- menu structure
- command routing
- replay/answer document support (if any)

Document:

- file paths
- command flow
- data structures used

---

## 2. Component scaffold generator

Inspect the logic used by:


create new component


Audit:

- scaffold templates
- generated directory structure
- default files created
- how component manifest is created

Document:

- scaffold layout
- template sources
- manifest generation logic

---

## 3. Component manifest model

Identify the canonical definition of a component.

Document:

- manifest format
- schema definitions
- versioning model
- ABI compatibility declarations

Identify where operations are defined (if present).

---

## 4. Operation model

Determine whether the codebase already has:

- explicit operation definitions
- operation metadata
- operation schemas
- operation bindings

Audit:

- where operations live in code
- how operations are registered
- how the component runtime exposes operations

If operations are implicit rather than structured, document this.

---

## 5. Lifecycle / QA support

Inspect support for lifecycle hooks such as:

- default
- setup
- update
- remove
- validate
- diagnose

Document:

- existing QA mechanisms
- how lifecycle operations are defined
- how wizard interacts with them (if at all)

---

## 6. i18n support

Inspect whether the repository already supports:

- localized CLI text
- translation bundles
- i18n tooling

Document:

- where i18n assets live
- CLI text strategy
- tooling integration

---

## 7. Runtime capability declarations

Audit how a component declares runtime requirements.

Examples to check:

- secrets
- filesystem
- network
- telemetry
- state/session usage

Document:

- where these declarations live
- how they are expressed
- whether wizard currently captures them

---

## 8. Flow compatibility metadata

Audit whether components can declare compatibility with:

- messaging flows
- event flows

Document:

- existing metadata
- whether this concept already exists

---

## 9. Profiles

Check if components support runtime profiles.

Possible examples:

- dev
- prod
- minimal
- full

Document:

- whether profiles exist
- where they are defined
- how they affect runtime behavior

---

## 10. Directory structure conventions

Document the expected structure for a component.

Examples:


src/
schemas/
assets/
examples/
qa/
i18n/


Confirm actual layout used.

---

# Deliverable

Produce a document:


docs/component_authoring_audit.md


The document must contain:

- current architecture overview
- wizard command flow
- scaffold layout
- manifest model
- lifecycle hooks
- capability declarations
- flow compatibility metadata
- profiles
- operation model

Include **code references and file paths**.

---

# Critical Guardrails

Codex must **not**:

- invent new manifest formats
- create new lifecycle models
- introduce a second operation definition system
- implement features before the audit is complete

All later PRs must reference this audit.

---

# Output

Commit only:


docs/component_authoring_audit.md


No functional changes.