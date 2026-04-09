PR-COMP-EMBED-IMPL-01 — Embed Canonical Manifest CBOR Into Component Wasm At Build Time
Title

Embed canonicalized component manifest as deterministic CBOR custom section during greentic-component build

Summary

Implement self-describing component artifacts by embedding the canonicalized component manifest into the final built Wasm as deterministic CBOR in a named custom section.

Locked derivation rule

The authoring manifest is canonicalized in greentic-component; from that canonical manifest, greentic-component derives both the embedded projection for artifact-local truth and, later, the describe projection for runtime-facing self-description.

This PR should be implemented in greentic-component and should not change WIT or move canonical manifest ownership into greentic-types.

The implementation should:

keep component.manifest.json as the authoring/build input

continue to validate and canonicalize the manifest in greentic-component

serialize a canonical manifest projection to deterministic CBOR

wrap it in a versioned embedded descriptor envelope

inject that payload into the final Wasm as a custom section

verify that the section can be read back

keep current authoring compatibility intact

lay the groundwork for later inspect/doctor/pack support

This PR is the first implementation step following the audits:

interfaces audit: no WIT change required

types audit: no full manifest move required

component audit: greentic-component is the right implementation owner

Goals
Primary goal

Make the final built component Wasm self-describing by embedding canonical manifest metadata into the artifact itself.

Secondary goals

define a stable embedded descriptor section contract for components

ensure deterministic CBOR generation

keep manifest canonicalization logic in one place

avoid drift between source manifest and embedded payload

preserve future compatibility with:

describe()

greentic-dev

greentic-pack

greentic-runner

Non-goals

Do not do the following in this PR:

do not change greentic-interfaces

do not revise WIT

do not move canonical manifest parsing/canonicalization into greentic-types

do not add pack/runtime-wide consumers yet

do not redesign describe() in this PR

do not compare against describe() in this PR

do not remove external component.manifest.json

do not introduce broad new CLI surfaces except minimal local verification support if needed

Architectural Decision
Vocabulary

Use these terms consistently in this PR:

- authoring manifest
- canonical manifest
- embedded projection
- embedded envelope
- artifact-local truth
- describe projection
- runtime projection

Source of truth

For build-time embedding, the source of truth remains:

the external component.manifest.json

after greentic-component validation, normalization, and canonicalization

Embedded artifact truth

Once built, the artifact-local truth becomes:

the embedded CBOR descriptor inside the Wasm

Relationship to describe()

describe() remains the runtime-facing projection.

This PR should not attempt to make describe() the full manifest, and it should stay completely independent of describe().

The only requirement here is to preserve a clean one-way derivation path from canonical manifest to embedded projection so a later PR can compare embedded projection and describe projection explicitly.

Required Design
1. Embedded section format

Use a named Wasm custom section.

Section name

Use:

greentic.component.manifest.v1

This should be treated as the stable section identifier for the MVP.

2. Embedded payload model

Embed a versioned CBOR envelope, not raw JSON and not naked manifest bytes.

Suggested model

Create a local versioned embedded descriptor type in greentic-component for this PR.

Suggested shape:

pub struct EmbeddedComponentDescriptorEnvelopeV1 {
    pub kind: String,              // "greentic.component.manifest"
    pub version: u32,              // 1
    pub encoding: String,          // "application/cbor"
    pub payload_schema: Option<String>,
    pub payload_hash_blake3: String,
    pub payload: Vec<u8>,          // canonical CBOR of manifest projection
}

And a payload model like:

pub struct EmbeddedComponentManifestV1 {
    // canonical manifest-derived data for artifact-local inspection
    // exact fields should come from canonicalized manifest output
}
Projection boundary requirement

This PR must introduce an explicit projection boundary such as:

- `build_embedded_manifest_projection(...)`
- `EmbeddedComponentManifestV1::from_canonical_manifest(...)`

`EmbeddedComponentManifestV1` should be a named curated projection derived from the canonical manifest, not the raw internal manifest struct.

Important note

Do not dump the raw unprocessed JSON file into the section.

The payload should come from the canonicalized typed manifest state already produced by greentic-component.

3. Payload scope

For this PR, embed the canonical manifest representation needed for artifact-local metadata.

That means one of these approaches is acceptable:

Preferred

Embed a canonical manifest projection that corresponds closely to the validated internal manifest model used after normalization.

Avoid

Embedding:

raw authoring JSON bytes

unstable debug serialization

non-deterministic maps/orderings

build-path-specific transient fields unless intentionally part of canonical artifact metadata

Local ownership rule

Keep the embedded envelope and embedded projection local to `greentic-component` in this PR.

Do not move them into `greentic-types` yet.

Implementation Tasks
Task 1 — Identify canonicalization boundary

Inspect the current build flow and find the exact point where:

manifest file is loaded

schema validated

normalized/defaulted

transformed into the internal typed manifest representation

Use that point as the single source for embedded payload generation.

Expected files likely include:

crates/greentic-component/src/manifest/mod.rs

crates/greentic-component/src/cmd/build.rs

Deliverable

Refactor as needed so the build flow has a clear internal value representing the canonicalized manifest ready for embedding and later reuse.

That flow should end at one explicit projection-creation boundary from canonical manifest to embedded projection.

Task 2 — Add local embedded descriptor module

Create a local module in greentic-component, for example:

crates/greentic-component/src/embedded_descriptor.rs

or

crates/greentic-component/src/embedded/
  - mod.rs
  - component_manifest_v1.rs

This module should define:

section name constant

envelope type(s)

payload type(s) if separate

deterministic encode helpers

decode helpers for verification/tests

Suggested exports

EMBEDDED_COMPONENT_MANIFEST_SECTION_V1

EmbeddedComponentDescriptorEnvelopeV1

encode_embedded_component_descriptor_v1(...)

decode_embedded_component_descriptor_v1(...)

Task 3 — Deterministic CBOR encoding

Use canonical CBOR encoding.

Prefer existing deterministic helpers already available in the repo/toolchain. Reuse current canonical CBOR utilities wherever possible.

Requirements

encoding must be deterministic

maps/field order must be stable under the chosen serializer/helper strategy

resulting bytes must be reproducible across builds given identical canonical manifest input

Deliverable

A helper that produces:

payload bytes

BLAKE3 hash of payload bytes

final envelope bytes

Task 4 — Inject custom section into final Wasm artifact

Update the build pipeline so that after the final component Wasm is built, the embedded envelope bytes are inserted into the output Wasm as a custom section named:

greentic.component.manifest.v1
Important

This should operate on the final built Wasm artifact, not merely on intermediate Rust code.

Build-mode rule

Embed in both debug and release builds by default so artifact behavior stays consistent across local development, tests, and CI.

Acceptable implementation approaches
Preferred

Patch the final .wasm during greentic-component build.

Also acceptable

If the build system naturally has a final Wasm transformation step already, integrate there.

Avoid

Relying only on proc macro-generated static bytes without guaranteeing they end up in the final final artifact as a custom section.

Task 5 — Read-back verification during build

After embedding, read the section back from the output Wasm and verify:

the section exists

it decodes correctly

the payload hash matches

the embedded payload corresponds to the manifest used for the build

Build behavior

For this PR, build should fail if:

embedding fails

read-back verification fails

payload hash verification fails

This is important so the artifact does not silently claim to be self-describing when it is not.

Task 6 — Preserve external manifest workflow

Do not remove or break the current external manifest behavior.

The build should still work with:

component.manifest.json as source

any current artifact/hash/update steps already done by greentic-component

This PR adds embedded metadata; it does not replace the existing authoring workflow.

Task 7 — Minimal extraction helper for local use

Add an internal helper to extract the embedded section from a Wasm file or bytes.

This is primarily for:

tests

build verification

later reuse by inspect/doctor PRs

This helper does not need to become a public CLI command in this PR unless it is nearly free.

Suggested File/Module Changes

These are indicative, not mandatory.

Likely touched files

crates/greentic-component/src/cmd/build.rs

crates/greentic-component/src/manifest/mod.rs

crates/greentic-component/src/describe.rs only if needed for shared canonicalization plumbing

new embedded descriptor module(s)

relevant tests

Potential new files

crates/greentic-component/src/embedded_descriptor.rs

crates/greentic-component/tests/embed_manifest.rs

Testing Requirements
1. Unit tests for encoding/decoding

Add tests covering:

canonical envelope encoding

decode roundtrip

payload hash correctness

deterministic byte stability for fixed fixture input

2. Build integration test

Add an integration test that:

builds a fixture component from component.manifest.json

reads the produced Wasm

extracts custom section greentic.component.manifest.v1

decodes the envelope

verifies expected manifest-derived fields

3. Failure-path tests

Add tests for:

missing custom section

malformed CBOR

hash mismatch

unexpected envelope version

4. Determinism test

Add a test to ensure that building the same fixture twice produces equivalent embedded payload bytes, or at minimum equivalent decoded payload and matching deterministic hash.

If full byte-for-byte Wasm determinism is too broad for this PR, test at least deterministic section payload output for the same canonical input.

Precedence Rules for This PR

This PR only implements embedding, but it should follow these intended rules internally.

During build

The canonical source is the normalized/validated manifest inside greentic-component.

The embedded section must be generated from that exact canonical state.

After build

The embedded section becomes the artifact-local metadata source.

Mismatch policy

If the embedded payload cannot be verified against the canonical source used during build, fail the build.

Acceptance Criteria

This PR is complete when:

 greentic-component build embeds a custom section named greentic.component.manifest.v1 into the final Wasm

 the embedded section contains deterministic CBOR, not raw JSON

 the payload is produced from canonicalized manifest data

 a versioned envelope is used

 the payload hash is included and verified

 build fails if embedding or verification fails

 current external manifest authoring flow still works

 automated tests cover encode/decode, build embedding, and failure cases

Suggested PR Description
Title

Embed canonical component manifest CBOR into built Wasm artifacts

Description

This PR makes Greentic component Wasm artifacts self-describing by embedding canonicalized manifest metadata into the final built Wasm as deterministic CBOR in a custom section.

The implementation is intentionally centered in greentic-component, which already owns manifest parsing, validation, normalization, and build orchestration.

This PR does not change WIT, does not remove external manifests, and does not move canonical manifest ownership into greentic-types. It establishes the artifact-local embedded metadata foundation needed for later inspect/doctor/pack/runtime follow-up work.

Other suggestions

Yes — a few, and they are worth doing in this order.

1. Add a follow-up PR to compare embedded manifest vs describe()

Not to force byte equality, but to check semantic agreement on overlapping fields:

name

version

capabilities

ops

schemas

setup

That will catch drift early.

2. Add greentic-component inspect --embedded

A very small follow-up PR that prints:

section present/missing

envelope version

payload hash

decoded summary fields

That will make debugging much easier.

3. Add a strict doctor check later

doctor should eventually warn/error when:

external manifest and embedded metadata disagree

embedded metadata is missing

embedded envelope version is unknown

4. Decide whether to embed in debug builds too

My recommendation: yes by default, unless it causes real DX pain.
Having the same behavior in debug and release usually avoids confusion.

5. Keep the first embedded payload local to greentic-component

Even if you expect shared reuse later, it is cleaner to prove the shape in one repo first.
Then, if greentic-pack and greentic-dev need it immediately, promote the envelope type into greentic-types as a small shared projection.

6. Document one-way derivation

This is important:

authoring manifest -> canonicalized manifest in greentic-component

canonicalized manifest -> embedded CBOR

canonicalized manifest -> describe() projection

But not the other way around.

That rule will prevent a lot of confusion later.
