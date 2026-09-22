//! End-to-end resolve+introspect over a real committed component fixture via a
//! `file://` reference — no network. Proves `resolve_component` fetches through
//! the distributor (digest-verified, cached) and introspects the wasm's exported
//! operations.

#![cfg(all(feature = "store", feature = "describe"))]

use std::path::PathBuf;

use greentic_component::resolve::resolve_component;

/// A committed v0.6.0 component wasm fixture.
fn fixture_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contract/fixtures/component_v0_6_0/component.wasm")
}

#[tokio::test]
async fn resolves_local_component_and_lists_operations() {
    let wasm = fixture_wasm();
    assert!(
        wasm.exists(),
        "fixture wasm must exist at {}",
        wasm.display()
    );

    let cache = tempfile::tempdir().expect("temp cache dir");
    let reference = format!("file://{}", wasm.display());

    let resolved = resolve_component(&reference, cache.path().to_path_buf())
        .await
        .expect("resolve_component must succeed for a local component wasm");

    assert!(
        !resolved.digest.is_empty(),
        "a pinned digest must be reported"
    );
    assert!(
        !resolved.operations.is_empty(),
        "the component must expose at least one operation; got none"
    );
    assert!(
        resolved.operations.iter().all(|op| !op.name.is_empty()),
        "every operation must carry a name"
    );
    // input_schema/description enrichment is a later slice; assert the current
    // (names-only) contract holds so a future change is a conscious one.
    assert!(
        resolved
            .operations
            .iter()
            .all(|op| op.input_schema.is_none()),
        "input schemas are not populated by WIT introspection yet"
    );
}

#[tokio::test]
async fn unknown_reference_is_a_fetch_error() {
    let cache = tempfile::tempdir().expect("temp cache dir");
    let result = resolve_component(
        "file:///nonexistent/path/to/component.wasm",
        cache.path().to_path_buf(),
    )
    .await;
    assert!(
        result.is_err(),
        "a missing local artifact must surface an error, not an empty success"
    );
}
