PR A — greentic-component: Emit operations as ComponentOperation[] objects (not strings)

Repo: greentic-component
Branch: feat/manifest-operations-objects
Problem: greentic-dev flow add-step fails to parse component.manifest.json because operations is currently emitted as ["handle_message"] (strings), but upstream consumers expect Vec<ComponentOperation> (objects).
Goal: Make greentic-component new and greentic-component build always write operations as objects matching the greentic_types::component_manifest::ComponentOperation schema.

0) Ground truth: import the type from greentic-types

Find the ComponentOperation struct definition in greentic-types (the version used by greentic-dev). It will define the exact fields. You must conform to it.

Typical shape (example only—use the real one):

pub struct ComponentOperation {
  pub name: String,
  pub description: Option<String>,
  pub input_schema: Option<serde_json::Value>,
  pub output_schema: Option<serde_json::Value>,
}


Action in this PR: update greentic-component to write the JSON that serde can parse into that struct.

1) Update the JSON schema published by greentic-component

You have a schema referenced in the manifest:

"$schema": "https://greenticai.github.io/greentic-component/schemas/v1/component.manifest.schema.json"

Required change

In the schema file inside this repo (likely under something like schemas/v1/component.manifest.schema.json), update:

operations from:

type: array, items: { type: string }

to:

type: array

items: { $ref: "#/$defs/componentOperation" } (or equivalent)

define $defs/componentOperation to match ComponentOperation

Also:

If default_operation exists, validate it matches one of the operation names (optional but nice; can be done as a separate validation step if JSON schema can’t express it cleanly).

Deliverable: schema now rejects string operations and requires operation objects.

2) Fix scaffolding: greentic-component new must write operation objects

Right now your scaffold produces:

"operations": ["handle_message"]

Required change

In the scaffold template file(s) that generate component.manifest.json (look in these usual places):

templates/

src/scaffold/

src/commands/new.rs or src/cmd/new.rs

anything named component.manifest.json template

Change it to something like:

"operations": [
  {
    "name": "handle_message",
    "input_schema": {},
    "output_schema": {}
  }
]


Important: use the actual required fields from ComponentOperation. If input_schema/output_schema are optional in Rust, you can omit them; but to keep things deterministic and helpful, include {}.

Also ensure:

default_operation: "handle_message" remains present and consistent.

3) Fix build-time mutation: anywhere build writes/normalizes operations

You showed greentic-component build regenerates flows, infers schema, and “refreshes artifacts/hashes in component.manifest.json”.

That pipeline likely reads the manifest into a struct, modifies fields, and writes it back.

Required change

Search in repo for:

"operations" (string literal)

operations.push(

Vec<String>

Operation / ComponentOperation

default_operation

You must fix two categories:

A) Parsing/writing structs

If greentic-component has its own manifest struct separate from greentic-types, update it so:

operations: Vec<ComponentOperation> not Vec<String>

B) Any logic that adds operations

If build infers/ensures ops, make it create objects:

operations.push(ComponentOperation {
  name: "handle_message".into(),
  input_schema: Some(json!({})),
  output_schema: Some(json!({})),
  // other fields default
});

4) Update config-flow emitter to pull operation from the right place

Your dev_flows template currently embeds:

"operation": "handle_message"


Good.

But make sure the generator that decides which op to embed now uses:

manifest.default_operation if set

else if operations.len() == 1 use that operation’s name

else error with a clear message: “multiple operations; set default_operation”

This should already exist but might currently assume Vec<String>. Update it accordingly.

5) Add regression tests inside greentic-component
Test 1 — Scaffolded manifest parses as greentic-types ComponentManifest

Add a test that runs the scaffold (in a temp dir) and then:

reads component.manifest.json

deserializes it using the same type greentic-dev uses (prefer greentic_types::...::ComponentManifest if greentic-component can depend on greentic-types, or replicate exact struct if not possible)

Assertion:

manifest.operations.len() >= 1

first op has name handle_message

default_operation == "handle_message"

Test 2 — Schema validation rejects string operations

If you have JSON schema validation tests, add one:

manifest with "operations": ["handle_message"] should fail schema validation

Test 3 — Build preserves operations object shape

Run greentic-component build on scaffold output and re-read manifest:

Ensure operations is still an array of objects, not rewritten to strings.

6) Backward-incompatibility policy (strict v1)

Since you want to drop legacy:

Do not support parsing string operations in greentic-component (optional, but recommended for strictness)

If you do want a short-lived migration, do it behind a feature flag; otherwise fail with a clear error.

Definition of Done

greentic-component new produces component.manifest.json where operations is an array of objects.

greentic-component build preserves that format and uses it to select default_operation.

greentic-dev flow add-step --manifest ... parses the manifest successfully (your exact repro command works past parsing).

Tests cover scaffold + build + schema.

Quick pointers (how to find the exact files fast)

In the greentic-component repo, run:

rg -n '"operations"\s*:' -S
rg -n 'default_operation|ComponentOperation|Vec<String>\s*operations' -S
rg -n 'component\.manifest\.json' -S


You will almost certainly find:

the scaffold template file

the manifest struct definition or serde model

the build rewrite logic

Those are the exact targets for this PR.
