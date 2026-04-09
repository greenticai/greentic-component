> Superseded by `.codex/PR-COMP-04-scaffold-qa-ops-i18n-cbor.md` (2026-02-28).
> Keep this file for historical context only.

You are implementing a template + interface alignment change in greentic-component.

Goal:
When creating a new Greentic component, the scaffold must include FOUR standard operations:
- default
- setup
- update
- remove

These ops must be reflected canonically in greentic-types and/or greentic-interfaces (do not invent names).
If the canonical names differ, follow the canonical ones and update the scaffold accordingly.

Pre-authorized:
- Read repo code, update templates, add new files, update docs, add tests, run fmt/tests.
- Use web browsing only to read greentic-ai(-org)/greentic-* repos if needed for truth.

Hard constraints:
- Keep changes minimal/surgical; do not rewrite unrelated systems.
- Ensure the scaffold is i18n-ready by default: no raw user-facing strings in the generated component.
- Generated component must include tools/i18n.sh which:
  1) installs Codex CLI if missing,
  2) runs “codex login” using browser flow first (or reports a clear instruction if headless),
  3) runs greentic-i18n-translator to translate i18n/en.json into target languages,
  4) ensures translations are included into the final wasm build.

References in this workspace:
- Follow the deterministic wizard/scaffold guidance patterns from PR-COMP-01. :contentReference[oaicite:2]{index=2}
- Follow the unified wizard/i18n key discipline from PR-COMP-02. :contentReference[oaicite:3]{index=3}:contentReference[oaicite:4]{index=4}the i18n “key-based CLI” playbook conventions (key naming, locale :contentReference[oaicite:5]{index=5}n approach). :contentReference[oaicite:6]{index=6}
- For Codex CLI install/login behavior use official docs:
  - install: npm :contentReference[oaicite:7]{index=7}r brew install codex)
  - login: codex login ; check: codex login status :contentReference[oaicite:8]{index=8}

Work steps:

A) Audit (must do first)
1) In greentic-interfaces:
   - locate component@0.6.0 descriptor definition and how “ops” are declared/encoded.
   - locate any existing lifecycle/setup/update/remove conventions or op registries.
2) In greentic-types:
   - locate any “component operation” enums/strings used by runner/operator.
3) In greentic-component:
   - locate the “new component” template files and how operation routing is scaffolded today.

Write a short audit note: docs/scaffold_ops/audit.md
Include file paths + the canonical op names you found.

B) Align types/interfaces (only if needed)
If the 4 ops are NOT already canonically modeled:
- Add the smallest shared representation (enum/consts/schema) in the correct repo module.
- Ensure greentic-interfaces + greentic-types remain consistent (one canonical source; other reexports/maps).
- Add a unit test that asserts the canonical op list matches exactly [default, setup, update, remove] (or the canonical names discovered).

C) Update the scaffold template
Update the default template produced by `greentic-component new` so a generated component contains:

1) A manifest/descriptor that declares the 4 ops (and any schemas required).
2) src/lib.rs:
   - a match/dispatch for all 4 ops (even if setup/update/remove are “TODO” stubs).
   - use i18n tags by default:
     - define i18n keys in i18n/en.json
     - use a t()/tf() helper (or existing greentic i18n helper) to emit messages via keys, not literals.
3) i18n layout:
   - i18n/en.json exists as source-of-truth
   - optional empty language json files can be created by the translation script (not required at scaffold time if policy says so)
4) tools/i18n.sh (executable):
   - checks for codex; installs if missing:
       - prefer npm global install: npm i -g @openai/codex
       - fallback to brew install codex when brew exists
   - checks auth:
       - codex login status (exit 0 means logged in)
       - if not logged in: codex login (browser flow)
   - runs translations:
       - greentic-i18n-translator translate (or the repo’s standard command)
       - greentic-i18n-translator validate/status after translation
   - exits non-zero with clear messages when prerequisites are missing.

D) Ensure translations are included “into the wasm”
Implement one of these (prefer whichever is already used in-repo):
Option 1 (recommended if available): build.rs packs i18n/*.json into a generated Rust file and includes it, or embeds in a wasm custom section during build.
Option 2: runtime include_bytes!(...) on i18n files (works as long as build includes them in crate).
Document which option you chose in the template README.

E) Tests
1) Template generation test:
   - generate into a temp dir
   - assert the generated project includes: tools/i18n.sh, i18n/en.json, and 4 op handlers wired.
2) “No raw strings” test (basic):
   - grep the generated src/lib.rs for a small set of banned literals (or enforce that user-facing output uses t()/tf()).

F) Docs
Update greentic-component docs:
- mention the 4 standard ops and what they mean
- show how to run tools/i18n.sh
- mention codex install/login requirements (with official commands)

Deliverables checklist:
- [ ] docs/scaffold_ops/audit.md
- [ ] template updated + regenerated fixtures if any
- [ ] tools/i18n.sh in template
- [ ] tests + docs updated
- [ ] cargo fmt + cargo test pass
