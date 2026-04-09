# PR-COMP-04: Scaffold QA Ops + Embedded i18n CBOR Bundle (Template + Wizard)

Date: 2026-02-28  
Repo: greentic-component  
Type: Implementation PR (follow-up to PR-COMP-03 + QA audits)  
Constraint: Scaffold-only. No runner/operator/pack/interfaces contract changes.

## Goal

Update both component scaffold paths:
1. `greentic-component new` (CLI template path)
2. wizard scaffold generation path

So that newly generated components are:
- QA-ready via conventional op dispatch:
  - `qa-spec`
  - `apply-answers`
  - `i18n-keys`
- i18n-ready by default (no raw user-facing strings in QA/setup paths)
- able to embed translations inside WASM as a single CBOR bundle (Option A)
- shipped with `tools/i18n.sh` to generate/validate translations for the standard locale list

## Non-goals

- Do not add new exported WIT functions.
- Do not replace string-op dispatch with fixed exported lifecycle ops.
- Do not change `greentic-runner`, `greentic-operator`, `greentic-pack`, `greentic-interfaces`.
- Do not modify runtime/operator contracts in this PR.

## Canonical locale list (source of truth)

Use this exact list in scaffolded output at `assets/i18n/locales.json`:

```json
["ar","ar-AE","ar-DZ","ar-EG","ar-IQ","ar-MA","ar-SA","ar-SD","ar-SY","ar-TN","ay","bg","bn","cs","da","de","el","en-GB","es","et","fa","fi","fr","fr-FR","gn","gu","hi","hr","ht","hu","id","it","ja","km","kn","ko","lo","lt","lv","ml","mr","ms","my","nah","ne","nl","nl-NL","no","pa","pl","pt","qu","ro","ru","si","sk","sr","sv","ta","te","th","tl","tr","uk","ur","vi","zh"]
```

## Deliverables

### A) Template + Wizard parity

Both scaffold paths must produce the same conventions and equivalent outputs:

```text
assets/
  i18n/
    en.json
    locales.json
src/
  lib.rs
  qa.rs
  i18n.rs
build.rs
tools/
  i18n.sh
README.md
```

### B) QA ops in generated dispatcher

Generated `src/lib.rs` must keep existing operation dispatch model and add branches for:
- `qa-spec`
- `apply-answers`
- `i18n-keys`

Semantics (demo scaffold behavior):
- `qa-spec`: input mode; output QA spec payload
- `apply-answers`: input mode + answers; output status + optional config override
- `i18n-keys`: output key list used by QA/setup messages

Mode alignment target: provision semantics (`install|update|remove`) while preserving current compatibility with existing QA mode expectations.

### C) i18n embedding (Option A)

Generate `build.rs` in scaffold that:
1. reads `assets/i18n/*.json`
2. parses to deterministic `BTreeMap`-ordered structure
3. serializes to `OUT_DIR/i18n.bundle.cbor`
4. generates `OUT_DIR/i18n_bundle.rs` exposing:

```rust
pub const I18N_BUNDLE_CBOR: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/i18n.bundle.cbor"));
```

Generated `src/i18n.rs` must:
- lazy-load/decode bundle
- resolve `t(locale, key)` fallback chain:
  - exact locale
  - base language (`fr-FR -> fr`)
  - default English fallback
  - key itself if missing

Source-of-truth locale file: `assets/i18n/en.json`.

### D) `tools/i18n.sh`

Scaffold `tools/i18n.sh` (executable), idempotent, clear failures.

Behavior:
1. ensure Codex CLI installed
   - prefer `npm i -g @openai/codex`
   - fallback `brew install codex` when brew exists
2. ensure login
   - `codex login status` (when supported)
   - fallback to `codex login` guidance/flow
3. run translation generation from `assets/i18n/en.json` using locales list from `assets/i18n/locales.json`
4. run translator validate/status
5. print build reminder (`cargo build` to embed bundle)

### E) Generated README additions

Add short sections for:
- running `tools/i18n.sh`
- translation embedding via `build.rs` CBOR bundle
- local smoke testing for `qa-spec` / `apply-answers` / `i18n-keys`

### F) Tests

Update scaffold tests to assert for both scaffold paths:
- `tools/i18n.sh` exists and is executable
- `assets/i18n/en.json` exists
- `assets/i18n/locales.json` exists and exactly matches canonical list
- dispatcher contains `qa-spec`, `apply-answers`, `i18n-keys`
- QA scaffold path avoids raw user-facing strings (key-based guard)

For i18n bundle:
- factor bundle packing into normal Rust module (callable by `build.rs`)
- unit test CBOR output decodes and contains `en`

### G) Docs

- Keep `docs/scaffold_ops/audit.md` and `docs/scaffold_ops/qa_audit.md` as audit references.
- Add `docs/scaffold_ops/qa_contract.md` with:
  - question-source precedence
  - current setup behavior (`apply-answers` may short-circuit setup flow OR answers may feed flow input)
  - explicit “no contract changes in this PR” note

## Work breakdown (implementation order)

1. Scaffold architecture refactor (internal only)
- Extract shared scaffold file-plan builder used by both `new` and wizard to prevent drift.
- Add unified generation context containing QA/i18n defaults.

2. Generated source updates
- Extend `src/lib.rs` template dispatch with `qa-spec`/`apply-answers`/`i18n-keys`.
- Extend `src/qa.rs` template with mode-aware demo QA spec + apply logic + key listing.
- Add `src/i18n.rs` bundle decode + fallback logic.

3. Build embedding
- Add bundle packer module + `build.rs` template wiring.
- Ensure deterministic CBOR output in generated project.

4. i18n tooling
- Add scaffolded `tools/i18n.sh` + chmod in generation path.
- Validate script idempotence and failure behavior in tests.

5. Test updates
- Update existing scaffold assertions for both paths.
- Add bundle packer unit tests.
- Add locale-list exact-match assertions.

6. Docs updates
- Add/refresh scaffold docs and `qa_contract.md`.

## Resolved implementation directives

Mode compatibility: implement a normalize_mode() that accepts both legacy (default/setup/update/remove) and provision-like (install/update/remove) mode strings. Map default and setup -> install. Internally generate specs for install/update/remove only.

apply-answers output: return a minimal operator-compatible JSON shape: { ok: bool, config?: object, warnings: [], errors: [] }. On validation failure, set ok=false and omit config. Keep room for future envelope expansion but do not invent a new required schema.

apply-answers response must preserve the base shape { ok, config?, warnings, errors }; additional optional fields are allowed but must be ignorable by operator and must not alter base-field semantics.

i18n fallback: lookup order must be exact locale -> base language -> en -> key. Do not depend on en-GB existing.

translator CLI: tools/i18n.sh must probe greentic-i18n-translator via --help and fail clearly if required commands are missing; do not hardcode flags that might not exist.

Additional script requirements:
- `tools/i18n.sh` must check `command -v greentic-i18n-translator`.
- Probe capabilities with `greentic-i18n-translator --help` and subcommand help as needed.
- If `validate`/`status` are unavailable, print warning and continue; if required translation capability is missing, exit non-zero.
- Do not silently install translator; print explicit install guidance.

## Constraints to preserve during implementation

- No WIT/world contract changes.
- No external repo edits.
- Keep changes surgical and shared between scaffold paths.
- Do not pre-commit generated non-English locale files in templates; generate from script.

## Checklist

- [ ] CLI `new` scaffold updated
- [ ] wizard scaffold updated
- [ ] parity file set produced
- [ ] QA dispatcher ops added
- [ ] i18n bundle embedding wired
- [ ] `tools/i18n.sh` added + executable
- [ ] tests updated/passing
- [ ] docs updated (`audit`, `qa_audit`, `qa_contract`)
