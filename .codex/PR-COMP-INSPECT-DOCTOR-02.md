PR-COMP-INSPECT-DOCTOR-02 — Embedded Manifest Inspection, Comparison, and Doctor Validation
Title

Add embedded manifest inspection and doctor validation for self-describing component Wasm artifacts

Summary

This PR adds the first consumer-facing tooling for the embedded component manifest introduced by PR-COMP-EMBED-IMPL-01.

Locked derivation rule

The authoring manifest is canonicalized in greentic-component; from that canonical manifest, greentic-component derives both the embedded projection for artifact-local truth and, later, the describe projection for runtime-facing self-description.

It should extend greentic-component so developers can:

inspect a built Wasm artifact for embedded manifest metadata

decode and display the embedded descriptor

compare embedded metadata with the external component.manifest.json

compare embedded metadata with describe() on overlapping fields

make doctor validate that the artifact is self-describing and internally consistent

This PR should stay focused on inspection and validation inside greentic-component.

It should not yet expand into:

greentic-pack

greentic-dev

runtime-wide enforcement

WIT changes

moving types into greentic-types

Goals
Primary goals

Add tooling that can answer:

does this Wasm contain an embedded manifest section?

can it be decoded?

does it match the canonical external manifest?

does it agree with describe() on overlapping fields?

is the artifact healthy enough to be considered self-describing?

Secondary goals

make debugging embedded metadata easy

define comparison semantics clearly

establish doctor rules before pack/runtime consume this metadata

prevent silent drift between:

source manifest

embedded CBOR

describe()

Non-goals

Do not do the following in this PR:

do not change greentic-interfaces

do not modify WIT

do not migrate shared envelope types into greentic-types

do not require greentic-pack or greentic-runner changes

do not redesign the full CLI unless needed

do not make describe() a full manifest replacement

do not remove support for external manifests

Architectural Intent

Vocabulary

Use these terms consistently in this PR:

- authoring manifest
- canonical manifest
- embedded projection
- embedded envelope
- artifact-local truth
- describe projection
- runtime projection

This PR should reinforce the now-agreed model:

external component.manifest.json = authoring/build input

embedded CBOR custom section = passive artifact-local metadata

describe() = runtime-facing projection

The purpose of inspect and doctor is to verify that these surfaces are aligned where they overlap.

Required Functional Changes
1. Add embedded-manifest inspection support

Extend greentic-component with inspection support for built Wasm artifacts containing:

greentic.component.manifest.v1

The implementation should:

open a Wasm artifact

find the custom section

decode the embedded CBOR envelope

verify the payload hash

expose the decoded embedded manifest projection in a structured way for later comparison/printing

Expected internal capability

Add a reusable internal API roughly along these lines:

read_embedded_component_manifest_from_wasm(...)

decode_embedded_component_manifest(...)

verify_embedded_component_manifest(...)

These helpers should be used by both:

inspect flows

doctor flows

2. Extend inspect to understand embedded metadata

Update the inspect command so it can inspect a built Wasm artifact and report embedded metadata.

Recommended behavior

If a Wasm artifact is provided or resolved, inspect should report:

whether embedded section exists

section version

envelope kind

payload hash

manifest identity fields

a concise summary of operations/capabilities/setup presence

Suggested output sections
Embedded descriptor status

present / missing

section name

envelope version

hash verified / failed

Embedded metadata summary

component name

version

summary

capabilities count/list

operation count/list

setup present/absent

schema refs count

Comparison summary

When external manifest and/or describe() is available:

embedded vs manifest: match / mismatch

embedded vs describe: match / mismatch

details on differences

Suggested CLI shape

Keep the current inspect UX style, but add embedded-awareness.

Possible options:

greentic-component inspect --wasm path/to/component.wasm
greentic-component inspect --artifact path/to/component.wasm
greentic-component inspect --embedded
greentic-component inspect --compare

You do not need to implement every flag above exactly if the current CLI shape suggests a better fit.

Key requirement

There must be at least one ergonomic path to inspect the embedded metadata directly from the Wasm.

3. Extend doctor to validate embedded metadata

Update doctor so that when a built Wasm artifact is present, it validates the embedded descriptor.

Minimum checks
Embedded section presence

pass if present

warn or fail if absent, depending on doctor mode/context

Locked rule:

- fail when a built Wasm artifact exists and embedded metadata is missing
- warn in source-only / no-artifact contexts

Envelope decode

fail if section exists but cannot be decoded

Payload hash validation

fail if hash does not match payload bytes

Embedded vs external manifest comparison

compare overlapping manifest-derived fields

report mismatch clearly

Embedded vs describe() comparison

compare overlapping runtime-facing fields

report mismatch clearly

4. Define comparison semantics explicitly

The PR must not rely on naive full-struct equality unless that is truly correct.

It should define semantic comparison for overlapping fields and compare projections, not raw sources.

A. Embedded vs external manifest

This is comparing:

embedded projection

canonical manifest-derived projection

Preferred rule:

compare on the canonical projection used for embedding

not raw JSON textual equality

Comparison categories

equal

semantically different

unavailable (missing external manifest)

unsupported version

B. Embedded vs describe()

This must compare only the overlapping runtime projection.

Recommended overlapping fields:

name

version

summary

capabilities

operation names

operation schema relationships where representable

setup presence/basic structure

schema refs / schema identities where representable

Important

Do not require equality for authoring-only or packaging-only fields that do not belong in describe().

Do not compare:

- embedded bytes vs manifest JSON text
- embedded full struct vs raw internal manifest struct
- embedded full struct vs raw describe() return object

5. Add a comparison model and diagnostics

Create a structured internal comparison result rather than ad hoc string checks.

Suggested internal types:

pub enum ComparisonStatus {
    Match,
    Mismatch,
    MissingLeft,
    MissingRight,
    Unsupported,
}

pub struct FieldComparison {
    pub field: String,
    pub status: ComparisonStatus,
    pub detail: Option<String>,
}

pub struct EmbeddedManifestComparisonReport {
    pub overall: ComparisonStatus,
    pub fields: Vec<FieldComparison>,
}

This does not need to be the exact API, but the implementation should produce structured comparison data that can be:

printed by inspect

checked by doctor

reused later by pack/dev tooling

This structured comparison result model should be the shared basis for both inspect and doctor.

6. Decide doctor severity rules

This PR should introduce clear severity behavior.

Recommended doctor behavior
Error / fail

malformed embedded section

CBOR decode failure

payload hash mismatch

embedded data present but semantically inconsistent with the canonical external manifest used by the workspace

embedded data present but obviously inconsistent with describe() on overlapping required fields

Warning

missing embedded section when inspecting source workspace before build artifact is produced

missing external manifest when only Wasm is provided

inability to compare with describe() because runtime projection is unavailable

unsupported future envelope version if graceful handling is preferred before hard fail

Pass

embedded section present, valid, hash verified, and comparisons agree

Suggested Implementation Tasks
Task 1 — Add reusable extraction and verification helpers

Build on top of the module introduced in PR-01 and add:

read section from Wasm bytes/path

decode envelope

verify payload hash

return typed embedded manifest projection

Place this in the same embedded module area so inspect and doctor both use the same code path.

Task 2 — Add projection comparison helpers

Create helper functions to compare:

A. canonical manifest projection ↔ embedded projection

Suggested function:

compare_embedded_with_manifest(...)
B. describe() projection ↔ embedded projection

Suggested function:

compare_embedded_with_describe(...)

These helpers should compare semantic overlap, not unrelated fields.

Task 3 — Surface embedded metadata in inspect

Update inspect command implementation so it can:

locate/respect built Wasm artifact

report embedded metadata summary

optionally print detailed comparison results

Output quality requirements

The output should be readable and actionable.

At minimum, when embedded metadata is present, show:

section present

version

hash verified

component name/version

summary of capabilities/ops/setup

comparison verdicts

When mismatches exist, show specific fields that differ.

Task 4 — Add doctor checks

Update doctor to run embedded metadata validation when a Wasm artifact is available.

Doctor should:

detect the built artifact

validate the embedded section

compare against external manifest if available

compare against describe() if available through current doctor flow

emit structured diagnostics with pass/warn/fail status

Important

Do not make doctor depend on pack/runtime repos. Keep it self-contained within greentic-component.

Task 5 — Add internal projection for describe() comparison

If there is no clear reusable projection already, define a small internal comparison projection derived from describe().

For example:

struct DescribeComparableProjection {
    name: String,
    version: String,
    summary: Option<String>,
    capabilities: BTreeSet<String>,
    ops: BTreeMap<String, ComparableOp>,
    setup_present: bool,
    schema_ids: BTreeSet<String>,
}

Then compare this with an equivalent embedded projection.

This keeps the comparison logic explicit and future-proof.

Task 6 — Add helpful diagnostics text

Doctor and inspect output should clearly distinguish:

missing embedded metadata

malformed embedded metadata

hash failure

mismatch with external manifest

mismatch with describe()

Examples of useful messages:

embedded manifest section greentic.component.manifest.v1 found and verified

embedded manifest payload hash mismatch

embedded manifest version differs from external canonical manifest

embedded manifest capabilities differ from describe() projection

describe() omits runtime op present in embedded manifest projection

Use precise language so drift is easy to debug.

Testing Requirements
1. Inspect integration test

Add an integration test that:

builds a fixture component

runs inspect logic on the output Wasm

confirms the embedded section is found

confirms expected summary fields appear or expected structured result is returned

2. Doctor happy-path test

Add a test where:

external manifest exists

built Wasm exists

embedded section exists and is valid

describe() agrees on overlapping fields

Expected result:

doctor passes

3. Doctor mismatch tests

Add tests for at least:

A. Missing section

doctor warns/fails according to intended mode

B. Malformed section

doctor fails

C. Hash mismatch

doctor fails

D. Embedded vs manifest mismatch

doctor fails with field-level diagnostic

E. Embedded vs describe mismatch

doctor fails with field-level diagnostic

4. Comparison unit tests

Add focused unit tests for semantic comparison behavior, including:

capabilities equal but differently ordered

ops equal by semantic content

optional summary presence/absence

setup present vs absent

schema reference equality where supported

ignored non-overlapping fields

5. Unsupported version test

If the decoder is versioned, add a test for unknown envelope version and ensure the intended doctor/inspect behavior is exercised.

Suggested File/Module Changes
Likely touched files

crates/greentic-component/src/cmd/inspect.rs

crates/greentic-component/src/cmd/doctor.rs

embedded descriptor module(s) introduced in PR-01

crates/greentic-component/src/describe.rs only if needed for comparison plumbing

test files

Suggested new files

crates/greentic-component/src/embedded_compare.rs

crates/greentic-component/tests/inspect_embedded.rs

crates/greentic-component/tests/doctor_embedded.rs

These exact paths are optional; use the repo’s conventions.

Acceptance Criteria

This PR is complete when:

 inspect can read and decode greentic.component.manifest.v1 from a built Wasm artifact

 inspect reports embedded manifest summary fields clearly

 doctor validates section presence, decode, and payload hash

 doctor compares embedded metadata against external manifest on overlapping canonical fields

 doctor compares embedded metadata against describe() on overlapping runtime fields

 mismatches produce actionable diagnostics

 tests cover happy path and failure cases

Suggested PR Description
Title

Add inspect and doctor support for embedded component manifest metadata

Description

This PR adds embedded-manifest inspection and validation support to greentic-component.

Following the previous embedding work, built Wasm artifacts may now contain canonical manifest metadata in the greentic.component.manifest.v1 custom section. This PR teaches inspect and doctor to read, verify, summarize, and compare that embedded metadata against the external manifest and describe() projection.

The implementation keeps the agreed separation of concerns:

external manifest = authoring/build input

embedded CBOR = passive artifact metadata

describe() = runtime-facing projection

No WIT changes are introduced in this PR.

Other suggestions

Yes — a few good follow-ups after PR-02.

1. Add a --strict mode to doctor

That gives you a migration path:

normal mode: warn for some missing cases

strict mode: fail on any inconsistency

2. Add machine-readable inspect output

A JSON mode for inspect would be useful later for CI and pack/dev tooling, for example:

greentic-component inspect --wasm x.wasm --format json
3. Add a “derivation path” note in docs

Document this explicitly:

source manifest → canonical manifest

canonical manifest → embedded projection

canonical manifest → describe() projection

This will reduce future confusion.

4. Add a tiny consistency test fixture repo

One or two sample components with:

valid embedded metadata

intentional mismatch cases

That will help future refactors.
