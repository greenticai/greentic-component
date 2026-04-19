# greentic-component wizard PR bundle

This bundle contains a sequential PR plan:

- PR-01-interfaces: Downstream repos must use `greentic_interfaces::canonical` (never `bindings::*`)
- PR-01: Wizard scaffold + component@0.6.0 template generator
- PR-02: Versioned WASM naming + Makefile + abi-version metadata
- PR-03: Extend `greentic-component doctor` to validate component@0.6.0
- PR-04: docs/examples only; clarify external flow mapping without manifest/runtime changes
- PR-05: Docs + E2E examples + CI guardrails

Apply PRs in order.
