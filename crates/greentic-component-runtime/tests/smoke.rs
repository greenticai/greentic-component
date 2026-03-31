use std::path::PathBuf;
use std::sync::Arc;

use greentic_component_runtime::{
    Bindings, CompError, ComponentRef, LoadPolicy, bind, describe, invoke, load,
};
use greentic_component_store::ComponentStore;
use greentic_types::{EnvId, TenantCtx, TenantId};
use serde_json::json;

fn fixture_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime crate parent")
        .join("greentic-component/tests/contract/fixtures/component_v0_6_0/component.wasm")
}

fn policy() -> LoadPolicy {
    let cache_dir = std::env::temp_dir().join(format!(
        "greentic-component-runtime-smoke-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&cache_dir).expect("cache dir");
    let store = Arc::new(ComponentStore::new(&cache_dir).expect("store"));
    LoadPolicy::new(store)
}

fn tenant() -> TenantCtx {
    TenantCtx::new(EnvId("dev".into()), TenantId("tenant".into()))
}

#[test]
fn loads_and_describes_contract_fixture_component() {
    let cref = ComponentRef {
        name: "fixture".into(),
        locator: fixture_wasm().display().to_string(),
    };

    let handle = load(&cref, &policy()).expect("load contract fixture");
    let info = describe(&handle).expect("describe fixture");

    assert!(info.name.is_some());
    assert!(
        info.exports
            .iter()
            .any(|export| export.operation == "handle_message")
    );
}

#[test]
fn bind_accepts_empty_config_for_contract_fixture() {
    let cref = ComponentRef {
        name: "fixture".into(),
        locator: fixture_wasm().display().to_string(),
    };
    let handle = load(&cref, &policy()).expect("load contract fixture");
    let mut secret_resolver =
        |_key: &str, _tenant: &TenantCtx| -> Result<String, CompError> { unreachable!() };

    bind(
        &handle,
        &tenant(),
        &Bindings::new(json!({}), vec![]),
        &mut secret_resolver,
    )
    .expect("empty config should bind");
}

#[test]
fn invoke_rejects_unknown_operations_before_guest_call() {
    let cref = ComponentRef {
        name: "fixture".into(),
        locator: fixture_wasm().display().to_string(),
    };
    let handle = load(&cref, &policy()).expect("load contract fixture");
    let err = invoke(&handle, "not-exported", &json!({}), &tenant())
        .expect_err("unknown operations must be rejected");

    assert!(matches!(err, CompError::OperationNotFound(name) if name == "not-exported"));
}

#[test]
fn invoke_rejects_missing_tenant_binding_before_instantiation() {
    let cref = ComponentRef {
        name: "fixture".into(),
        locator: fixture_wasm().display().to_string(),
    };
    let handle = load(&cref, &policy()).expect("load contract fixture");
    let err = invoke(&handle, "handle_message", &json!({}), &tenant())
        .expect_err("known operation should still require a tenant binding");

    assert!(matches!(err, CompError::BindingNotFound(key) if key == "dev::tenant"));
}

#[test]
fn invoke_executes_bound_contract_fixture_operation() {
    let cref = ComponentRef {
        name: "fixture".into(),
        locator: fixture_wasm().display().to_string(),
    };
    let handle = load(&cref, &policy()).expect("load contract fixture");
    let mut secret_resolver =
        |_key: &str, _tenant: &TenantCtx| -> Result<String, CompError> { unreachable!() };

    bind(
        &handle,
        &tenant(),
        &Bindings::new(json!({}), vec![]),
        &mut secret_resolver,
    )
    .expect("bind fixture");

    let output = invoke(
        &handle,
        "handle_message",
        &json!({"input": "Hello from runtime smoke"}),
        &tenant(),
    )
    .expect("invoke bound operation");

    let message = output
        .get("message")
        .and_then(|value| value.as_str())
        .expect("message output");
    assert!(message.contains("Hello from runtime smoke"));
    assert!(message.contains("handle_message"));
}
