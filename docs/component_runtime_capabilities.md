# Component Runtime Capabilities

This repo uses the existing `component.manifest.json` schema as the canonical runtime capability model.

## Canonical fields

Runtime permission declarations live under `capabilities`:

- `capabilities.wasi.filesystem.mode`
- `capabilities.wasi.filesystem.mounts`
- `capabilities.wasi.env.allow`
- `capabilities.wasi.random`
- `capabilities.wasi.clocks`
- `capabilities.host.secrets.required`
- `capabilities.host.state.read`
- `capabilities.host.state.write`
- `capabilities.host.state.delete`
- `capabilities.host.messaging`
- `capabilities.host.events`
- `capabilities.host.http.client`
- `capabilities.host.http.server`
- `capabilities.host.telemetry.scope`
- `capabilities.host.iac`

Related runtime metadata also lives at the manifest top level:

- `secret_requirements`
- `limits`
- `telemetry`

## Secrets

Secrets currently exist in two surfaces:

- `secret_requirements`
- `capabilities.host.secrets.required`

Authoring flows in this repo now treat `secret_requirements` as the primary authoring surface and write the same generated requirements into `capabilities.host.secrets.required` for compatibility.

Manifest parsing/validation accepts either surface when only one is populated. If both are populated, their shared runtime fields must agree:

- `key`
- `required`
- `scope`
- `format`
- `schema`

Secret values themselves do not belong in the manifest.

## Filesystem

Filesystem access is declared only through:

- `capabilities.wasi.filesystem.mode`
- `capabilities.wasi.filesystem.mounts`

Supported modes are:

- `none`
- `read_only`
- `sandbox`

Mounts use `{ name, host_class, guest_path }`.

## HTTP / network

Outbound and inbound HTTP access is declared through:

- `capabilities.host.http.client`
- `capabilities.host.http.server`

There is no separate top-level `network` block.

## Messaging / events

Messaging and event capabilities are declared through:

- `capabilities.host.messaging.inbound`
- `capabilities.host.messaging.outbound`
- `capabilities.host.events.inbound`
- `capabilities.host.events.outbound`

Authoring flows only write the `messaging` or `events` object when at least one direction is enabled for that capability.

## State

State permissions are declared through:

- `capabilities.host.state.read`
- `capabilities.host.state.write`
- `capabilities.host.state.delete`

`delete` implies `write` during manifest normalization.

## Telemetry

Telemetry is intentionally split:

- `capabilities.host.telemetry.scope`
  - permission granted to the component
- top-level `telemetry`
  - runtime/config metadata such as `span_prefix`, `attributes`, `emit_node_spans`

Authoring flows keep those separate. Setting telemetry permission does not automatically create top-level telemetry config. Top-level telemetry config is only written when a span prefix is provided.

## Scaffold authoring

`greentic-component new` supports create-time capability authoring with:

- `--filesystem-mode`
- `--filesystem-mount`
- `--messaging-inbound`
- `--messaging-outbound`
- `--events-inbound`
- `--events-outbound`
- `--http-client`
- `--http-server`
- `--state-read`
- `--state-write`
- `--state-delete`
- `--telemetry-scope`
- `--telemetry-span-prefix`
- `--telemetry-attribute`
- `--secret-key`
- `--secret-env`
- `--secret-tenant`
- `--secret-format`

`greentic-component wizard --mode create` supports the same capability areas through answer fields and interactive prompts, including separate inbound/outbound booleans for messaging and events. `wizard` remains the richer edit surface for existing components; `new` only authors these fields during initial scaffold creation.
