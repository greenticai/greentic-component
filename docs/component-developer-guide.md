# Component Developer Guide

This guide explains what a Greentic component is, how to build one, and how to test it locally. It is written for new users and does not assume prior Greentic knowledge.

Canonical target: `greentic:component/component@0.6.0`.
Legacy compatibility guidance is tracked in `docs/vision/legacy.md`.

## 1) What is a Greentic component?

A Greentic component is a WebAssembly (Wasm) module that exposes one or more **operations**. Each operation:

- accepts **JSON input**
- returns **JSON output**
- may optionally use **capabilities** like state or secrets

Components run inside a Greentic runner in production, but you can test them locally without a runner.

## 2) Component anatomy

Every component has three core pieces:

- **Wasm binary** (`.wasm`) — the compiled component code.
- **Component manifest** (`component.manifest.json`) — metadata and capabilities.
- **Operations** — named handlers the runner (or local test harness) can invoke.

### 2.1 Contract authority in 0.6

For `component@0.6.0`, the component itself is authoritative for:

- config schema
- secret requirements
- i18n keys/catalog references
- QA spec/questions

Do not treat manifest-sourced schema overrides as valid for 0.6 flows.

### Minimal manifest example

```json
{
  "id": "com.example.hello",
  "name": "Hello Component",
  "version": "0.1.0",
  "world": "greentic:component/component@0.6.0",
  "describe_export": "describe",
  "operations": [
    { "name": "render", "input_schema": {}, "output_schema": {} }
  ],
  "config_schema": { "type": "object", "properties": {}, "required": [], "additionalProperties": false },
  "supports": ["messaging"],
  "profiles": { "default": "stateless", "supported": ["stateless"] },
  "capabilities": {
    "wasi": { "filesystem": { "mode": "none", "mounts": [] }, "env": { "allow": [] }, "random": true, "clocks": true },
    "host": {
      "state": { "read": true, "write": true, "delete": false },
      "secrets": { "required": [] }
    }
  },
  "artifacts": { "component_wasm": "bin/component.wasm" },
  "hashes": { "component_wasm": "blake3:<64-hex-digest>" }
}
```

What the key fields mean:

- `id`, `name`, `version`: identity and versioning.
- `world`: the WIT world your Wasm exports.
- `operations`: operation names and their JSON input/output schemas.
- `capabilities`: what host services your component may use (state, secrets, etc.).
- `artifacts` and `hashes`: where the Wasm lives and its hash for integrity.

Operation schemas must describe concrete JSON shapes (not just `{}`). Doctor/build enforce this by default and emit `E_OP_SCHEMA_EMPTY` unless you pass `--permissive` (which only logs `W_OP_SCHEMA_EMPTY`). Keep `schemas/io/input.schema.json` and `schemas/io/output.schema.json` populated with the shapes you expect, reference them from `manifest.schemas`, and rerun `greentic-component flow update` whenever you tweak those schemas.

## 3) Payload model (canonical)

The **payload** is simply the JSON input passed to an operation. The payload is built by the runner (or by you in local tests).

Important rules:

- Components do **not** automatically see previous node outputs.
- Components do **not** automatically receive persistent state in input.
- The payload shape is entirely up to you.

Example payload:

```json
{
  "payload": {
    "user_id": 123,
    "message": "hello"
  }
}
```

## 4) State model (canonical state-store)

Persistent state is accessed via the canonical WIT interface:

`greentic:state/store@1.0.0`

Key properties:

- State is **tenant-scoped** (env + tenant, optional team/user).
- State is keyed by **(prefix, StateKey)**. The prefix is derived from execution context.
- State is **not** injected into JSON input.
- State access is **capability-gated** (read/write/delete must be declared in the manifest).

Conceptual example:

1. Read state at the start of an operation.
2. Update the value in memory.
3. Write the new value before returning.

In local tests, the state store is in-memory only.

## 5) Secrets model

Secrets are accessed through the secrets capability. They are **never** passed inside the JSON payload.

In local testing, secrets can be provided from:

- a `.env` file (`KEY=VALUE` per line)
- a JSON map file (`{ "KEY": "VALUE" }`)
- repeated `--secret KEY=VALUE` flags

If a secret is not declared in the manifest, the test harness denies access.

### 5.1 Happy path: declare `http` + `secrets`

Manifest capability declaration example:

```json
"capabilities": {
  "host": {
    "http": { "client": true, "server": false },
    "secrets": {
      "required": [
        { "key": "API_TOKEN", "required": true, "format": "text" }
      ]
    }
  }
}
```

Generated scaffold note: capability contract fields in `describe()` come from static component-authored declarations (edit those constants/helpers in generated source).

Wizard shortcut for scaffolds:

```bash
greentic-component wizard apply --mode create \
  --answers ./answers.json \
  --emit-answers ./answers.out.json
```

### 5.2 Denial path: capability missing

If `host.state.write` is not granted and the component writes state, expect:

- code: `state.write.denied`
- message: `state store writes are disabled by manifest capability`

Fix: grant the missing capability (`capabilities.host.state.write: true`) and rebuild/retest.

## 6) Building a component

At a high level:

1. Create a project targeting `wasm32-wasip2`.
2. Implement your operation handlers.
3. Export the Greentic component interface (your WIT world).
4. Build the `.wasm` file.

You do not need to be a Rust expert to start. Use the scaffolded templates (`greentic-component new`) to get a working baseline.

## 7) Local testing with `greentic-component test`

The `test` command runs your component locally with in-memory state and secrets. It does **not** simulate flow routing or templating.

### 7.1 Basic test

```bash
greentic-component test \
  --wasm ./component.wasm \
  --op render \
  --input ./input.json
```

- `--op` selects the operation to invoke.
- `--input` provides the JSON input file.
- The output is the operation's JSON response.

### 7.2 Inline JSON

```bash
greentic-component test \
  --wasm ./component.wasm \
  --op render \
  --input-json '{"payload":{"x":1}}'
```

### 7.3 Providing execution context

Execution context controls tenant scoping and state prefixes. Use it to mirror production behavior.

```bash
greentic-component test \
  --wasm ./component.wasm \
  --op handle_interaction \
  --input ./interaction.json \
  --env dev --tenant demo \
  --flow my-flow --node card --session abc
```

### 7.4 Testing state

The test harness provides an in-memory state store. State persists across multiple invocations **within the same process** and is scoped by tenant + flow/session prefix.

```bash
greentic-component test \
  --wasm ./component.wasm \
  --step --op step1 --input ./step1.json \
  --step --op step2 --input ./step2.json \
  --state-dump
```

`--state-dump` prints the in-memory keys after execution so you can verify writes.
You can also pre-seed bytes with `--state-set KEY=BASE64` if you need to test reads without an initial write step.

### 7.5 Testing secrets

```bash
greentic-component test \
  --wasm ./component.wasm \
  --op render \
  --input ./input.json \
  --secrets ./secrets.env
```

`.env` files use `KEY=VALUE` per line. Keep secrets out of git.

## 8) Common mistakes and troubleshooting

**Why is my state always empty?**  
The manifest may not declare `host.state` or the context prefix is different than expected.

**Why is my component failing in the runner but not locally?**  
Missing capability declarations. The runner enforces capabilities more strictly.

**Why is a value a string instead of a number?**  
Input JSON is passed directly. Templating and type conversion are runner responsibilities.

## 9) How this maps to real flows

In real flows, the runner:

- builds input JSON using templating (entry, previous node output, etc.)
- passes the resulting JSON to the component

`greentic-component test` only executes the component. It does not run flow graphs, routing, or templating. This is intentional: it helps you test component logic in isolation.
