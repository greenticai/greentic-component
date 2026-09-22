//! Resolve a component by store/repo/OCI URL into its operation surface.
//!
//! Glue over [`greentic_distributor_client::DistClient`] (fetch + digest-verify a
//! `store://` / `repo://` / `oci://` / `https://` / `file://` reference into a
//! cached local wasm) and [`crate::describe`] (introspect the wasm's exported
//! operations). This is the fetch+introspect half of the "ComponentExtension"
//! adapter: point at any gtc wasm by URL and discover the operations it exposes
//! as agentic-worker tools.
//!
//! Scope note (verified 2026-06-25): WIT introspection yields operation NAMES
//! (and doc comments when present — though `///` docs do not survive the component
//! build today, see `crate::prepare`). It does NOT yield per-operation input
//! schemas. Those come from the component's manifest / describe export and are a
//! later enrichment slice, so [`ResolvedOperation::input_schema`] and
//! [`ResolvedOperation::description`] are `None` here for now.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::describe;

/// One operation discovered on a resolved component.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedOperation {
    /// Operation/function name (for example `handle_message`).
    pub name: String,
    /// LLM-facing input schema. `None` until enriched from the component
    /// manifest / describe export (WIT introspection does not carry it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// Human/LLM-facing description. `None` until a description source lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A component resolved from a URL: its identity, pinned digest, and the
/// operations it exposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedComponent {
    /// Component identity reported by the distributor (the component id).
    pub component_ref: String,
    /// Pinned digest (as reported by the distributor) of the resolved artifact.
    pub digest: String,
    /// Operations the component exposes.
    pub operations: Vec<ResolvedOperation>,
}

/// Errors from [`resolve_component`].
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The distributor failed to resolve/fetch the reference.
    #[error("failed to fetch component '{reference}': {source}")]
    Fetch {
        reference: String,
        #[source]
        source: greentic_distributor_client::dist::DistError,
    },
    /// The fetched artifact exposed no readable local wasm file.
    #[error("fetched artifact for '{reference}' has no local wasm file")]
    NoWasm { reference: String },
    /// Introspecting the wasm's exported operations failed.
    #[error("failed to introspect component operations: {0}")]
    Introspect(#[from] describe::DescribeError),
}

/// Fetch a component by `reference` (a `store://` / `repo://` / `oci://` /
/// `https://` / `file://` URL or digest), verifying its digest and caching it
/// under `cache_dir`, then introspect the operations it exposes.
///
/// The returned operations carry names only; input schemas and descriptions are
/// filled by a later enrichment slice (see the module-level scope note).
pub async fn resolve_component(
    reference: &str,
    cache_dir: PathBuf,
) -> Result<ResolvedComponent, ResolveError> {
    use greentic_distributor_client::dist::{CachePolicy, ResolvePolicy};
    use greentic_distributor_client::{DistClient, DistOptions};

    let opts = DistOptions {
        cache_dir,
        ..DistOptions::default()
    };
    let client = DistClient::new(opts);

    let fetch_err = |source| ResolveError::Fetch {
        reference: reference.to_string(),
        source,
    };
    let source = client.parse_source(reference).map_err(fetch_err)?;
    let descriptor = client
        .resolve(source, ResolvePolicy)
        .await
        .map_err(fetch_err)?;
    let artifact = client
        .fetch(&descriptor, CachePolicy)
        .await
        .map_err(fetch_err)?;

    // `fetch` yields either in-memory bytes or an existing local path; prefer an
    // explicit `wasm_path`, fall back to the artifact's `local_path`. We need a
    // file on disk for WIT introspection.
    let wasm_path = artifact
        .wasm_path
        .clone()
        .filter(|p| p.exists())
        .unwrap_or_else(|| artifact.local_path.clone());
    if !wasm_path.exists() {
        return Err(ResolveError::NoWasm {
            reference: reference.to_string(),
        });
    }

    let operations = introspect_operations(&wasm_path)?;
    Ok(ResolvedComponent {
        component_ref: artifact.component_id.clone(),
        digest: artifact.resolved_digest.clone(),
        operations,
    })
}

/// Introspect the exported operations of a local component wasm.
fn introspect_operations(
    wasm_path: &Path,
) -> Result<Vec<ResolvedOperation>, describe::DescribeError> {
    // Empty preferred-world string: `from_wit_world` falls back to the world the
    // component itself declares (see `describe::build_payload_from_world`), so we
    // do not need to know the world ref ahead of time.
    let payload = describe::from_wit_world(wasm_path, "")?;
    Ok(operations_from_payload(&payload))
}

/// Map a describe payload's exported functions to [`ResolvedOperation`]s,
/// de-duplicating by name (a function exported by multiple versions appears
/// once).
fn operations_from_payload(payload: &describe::DescribePayload) -> Vec<ResolvedOperation> {
    let mut ops = Vec::new();
    let mut seen = BTreeSet::new();
    for version in &payload.versions {
        let Some(functions) = version.schema.get("functions").and_then(|f| f.as_array()) else {
            continue;
        };
        for function in functions {
            let Some(name) = function.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            let description = function
                .get("docs")
                .and_then(|d| d.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            ops.push(ResolvedOperation {
                name: name.to_string(),
                input_schema: None,
                description,
            });
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(functions: serde_json::Value) -> describe::DescribePayload {
        describe::DescribePayload {
            name: "test".into(),
            schema_id: None,
            versions: vec![describe::DescribeVersion {
                version: semver::Version::new(0, 1, 0),
                schema: json!({ "world": "greentic:component/node@0.1.0", "functions": functions }),
                defaults: None,
            }],
        }
    }

    #[test]
    fn maps_function_names_dedups_and_captures_docs() {
        let p = payload(json!([
            { "name": "validate", "key": "validate" },
            { "name": "translate", "key": "translate", "docs": "  Translate input.  " },
            { "name": "validate", "key": "validate" },
        ]));
        let ops = operations_from_payload(&p);

        assert_eq!(
            ops.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
            vec!["validate", "translate"],
            "names map in order and de-duplicate"
        );
        assert_eq!(ops[1].description.as_deref(), Some("Translate input."));
        assert!(
            ops.iter().all(|o| o.input_schema.is_none()),
            "WIT introspection carries no input schema yet"
        );
    }

    #[test]
    fn empty_or_missing_functions_yields_no_operations() {
        assert!(operations_from_payload(&payload(json!([]))).is_empty());

        let no_functions = describe::DescribePayload {
            name: "test".into(),
            schema_id: None,
            versions: vec![describe::DescribeVersion {
                version: semver::Version::new(0, 1, 0),
                schema: json!({ "world": "w" }),
                defaults: None,
            }],
        };
        assert!(operations_from_payload(&no_functions).is_empty());
    }
}
