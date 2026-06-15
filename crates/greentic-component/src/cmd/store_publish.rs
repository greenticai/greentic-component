//! `greentic-component store publish` — produce the Greentic Store's describe-v2
//! for a component and (eventually) upload it as a `ComponentExtension`.
//!
//! The store search (`find_in_store(kind="component")`) indexes an extension's
//! `describe.json`. Components carry no extension-shaped describe, so this
//! command maps a component's manifest into a valid describe-v2:
//!
//! - `metadata` from the manifest (`summary`/`author`/`license` are authored or
//!   defaulted — a component manifest has none).
//! - `capabilities.offered` = one entry per operation (`find_in_store` filters
//!   on these), id `component:<component-id>/<operation>`.
//! - `runtime.components` = the single real component (its wasm sha256 + world),
//!   which is exactly what the describe-v2 `runtimeComponent` shape wants.
//! - `compat.min_*` are permissive stubs; `contract_version` is read from the
//!   WIT world (`…@x.y.z`).
//!
//! This slice builds + emits the describe-v2 (`--dry-run`). Packaging the
//! `.gtxpack` and the multipart upload land in the next slice.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::manifest::parse_manifest;

#[derive(Args, Debug, Clone)]
pub struct StorePublishArgs {
    /// Path to the component manifest JSON.
    #[arg(long, value_name = "FILE", default_value = "component.manifest.json")]
    pub manifest: PathBuf,
    /// Path to the built component `.wasm`.
    #[arg(long, value_name = "FILE")]
    pub wasm: PathBuf,
    /// Dotted, namespace-enforced store id (`metadata.id`), e.g.
    /// `greentic.component-http`. Defaults to the manifest id when it is
    /// already dotted; otherwise required (the store enforces the namespace).
    #[arg(long, value_name = "ID")]
    pub store_id: Option<String>,
    /// Store base URL (e.g. https://store.greentic.cloud). Falls back to
    /// `GREENTIC_STORE_URL`.
    #[arg(long, value_name = "URL")]
    pub store_url: Option<String>,
    /// Publisher bearer token. Falls back to `GREENTIC_STORE_TOKEN`.
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,
    /// One-line human summary (defaults to the component name).
    #[arg(long)]
    pub summary: Option<String>,
    /// Author display string (defaults to the component id).
    #[arg(long)]
    pub author: Option<String>,
    /// SPDX license id.
    #[arg(long, default_value = "UNLICENSED")]
    pub license: String,
    /// Build and print the describe-v2 without uploading.
    #[arg(long)]
    pub dry_run: bool,
}

/// Primitive inputs for the describe-v2 mapping (kept free of manifest types so
/// the mapping is pure and unit-testable).
pub(crate) struct DescribeInputs<'a> {
    /// Dotted, namespace-enforced store identity (`metadata.id`), e.g.
    /// `greentic.component-http`. Must match the publisher's allowed prefix.
    pub store_id: &'a str,
    /// The component manifest id (used for capability ids + the runtime
    /// component key), not necessarily dotted.
    pub component_id: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub summary: &'a str,
    pub author: &'a str,
    pub license: &'a str,
    pub world: &'a str,
    pub wasm_sha256_hex: &'a str,
    pub operation_names: &'a [String],
}

/// Map a component to the store's describe-v2 (`kind=ComponentExtension`).
pub(crate) fn build_component_describe_v2(i: &DescribeInputs) -> Value {
    let offered: Vec<Value> = i
        .operation_names
        .iter()
        .map(|op| json!({ "id": capability_id(i.component_id, op), "version": i.version }))
        .collect();

    json!({
        "apiVersion": "greentic.ai/v2",
        "kind": "ComponentExtension",
        "compat": {
            "min_designer_version": "0.0.0",
            "min_runner_version": "0.0.0",
            "contract_version": contract_version_from_world(i.world),
        },
        "metadata": {
            "id": i.store_id,
            "name": i.name,
            "version": i.version,
            "summary": i.summary,
            "author": { "name": i.author },
            "license": i.license,
        },
        "capabilities": { "offered": offered, "required": [] },
        "runtime": {
            "permissions": {},
            "components": {
                i.component_id: { "sha256": i.wasm_sha256_hex, "world": i.world }
            }
        },
        "contributions": {}
    })
}

/// Build a capRef id of the form `component:<id>/<operation>` that satisfies the
/// describe-v2 capRef pattern `^[a-z][a-z0-9-]*:[a-z][a-z0-9/._-]*$`.
fn capability_id(component_id: &str, operation: &str) -> String {
    format!(
        "component:{}/{}",
        sanitize_segment(component_id),
        sanitize_segment(operation)
    )
}

/// Lowercase and coerce to the capRef path character class, guaranteeing the
/// first character is `[a-z]`.
fn sanitize_segment(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            let lc = c.to_ascii_lowercase();
            if lc.is_ascii_lowercase() || lc.is_ascii_digit() || matches!(lc, '.' | '_' | '-') {
                lc
            } else {
                '-'
            }
        })
        .collect();
    if !out.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        out.insert(0, 'c');
    }
    out
}

/// Extract a semver `contract_version` from a WIT world like
/// `greentic:component@0.6.0`; falls back to `0.6.0` when absent/unparseable.
fn contract_version_from_world(world: &str) -> String {
    match world.rsplit_once('@') {
        Some((_, ver)) if looks_like_semver(ver) => ver.to_string(),
        _ => "0.6.0".to_string(),
    }
}

fn looks_like_semver(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Matches the describe-v2 `metadata.id` pattern
/// `^[a-z][a-z0-9.-]*\.[a-z0-9.-]+$` — a dotted, namespaced identifier.
fn is_valid_store_id(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return false;
    }
    // Needs at least one interior dot with a non-empty trailing segment.
    matches!(s.rsplit_once('.'), Some((head, tail)) if !head.is_empty() && !tail.is_empty())
}

pub fn run(args: StorePublishArgs) -> Result<()> {
    let raw = std::fs::read_to_string(&args.manifest)
        .with_context(|| format!("read manifest {}", args.manifest.display()))?;
    let manifest = parse_manifest(&raw).map_err(|e| anyhow!("invalid component manifest: {e}"))?;

    let wasm =
        std::fs::read(&args.wasm).with_context(|| format!("read wasm {}", args.wasm.display()))?;
    let wasm_sha256_hex = hex::encode(Sha256::digest(&wasm));

    let component_id = manifest.id.as_str().to_string();
    let version = manifest.version.to_string();
    let operation_names: Vec<String> = manifest
        .operations
        .iter()
        .map(|op| op.name.clone())
        .collect();
    let summary = args
        .summary
        .clone()
        .unwrap_or_else(|| manifest.name.clone());
    let author = args.author.clone().unwrap_or_else(|| component_id.clone());

    // metadata.id must be a dotted, namespace-enforced store identity.
    let store_id = args
        .store_id
        .clone()
        .unwrap_or_else(|| component_id.clone());
    if !is_valid_store_id(&store_id) {
        bail!(
            "store id {store_id:?} is not a valid dotted identifier (e.g. \
             `greentic.{component_id}`); pass --store-id within your publisher namespace"
        );
    }

    let describe = build_component_describe_v2(&DescribeInputs {
        store_id: &store_id,
        component_id: &component_id,
        name: &manifest.name,
        version: &version,
        summary: &summary,
        author: &author,
        license: &args.license,
        world: manifest.world.as_str(),
        wasm_sha256_hex: &wasm_sha256_hex,
        operation_names: &operation_names,
    });

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&describe)?);
        return Ok(());
    }

    // Upload requires endpoint + credentials; validate them so the failure is
    // about the not-yet-wired upload, not a missing flag.
    let _store_url = args
        .store_url
        .clone()
        .or_else(|| std::env::var("GREENTIC_STORE_URL").ok())
        .ok_or_else(|| anyhow!("--store-url or GREENTIC_STORE_URL is required to publish"))?;
    let _token = args
        .token
        .clone()
        .or_else(|| std::env::var("GREENTIC_STORE_TOKEN").ok())
        .ok_or_else(|| anyhow!("--token or GREENTIC_STORE_TOKEN is required to publish"))?;
    bail!(
        "store publish upload is not yet wired; re-run with --dry-run to emit the describe-v2 \
         (gtxpack packaging + multipart upload land in the next slice)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        build_component_describe_v2(&DescribeInputs {
            store_id: "greentic.component-http",
            component_id: "component-http",
            name: "HTTP Client",
            version: "0.1.0",
            summary: "Make HTTP requests",
            author: "greentic",
            license: "Apache-2.0",
            world: "greentic:component@0.6.0",
            wasm_sha256_hex: &"ab".repeat(32),
            operation_names: &["run".to_string(), "Health Check".to_string()],
        })
    }

    #[test]
    fn header_and_kind() {
        let d = sample();
        assert_eq!(d["apiVersion"], "greentic.ai/v2");
        assert_eq!(d["kind"], "ComponentExtension");
    }

    #[test]
    fn metadata_carries_authored_fields() {
        let d = sample();
        assert_eq!(d["metadata"]["id"], "greentic.component-http");
        assert_eq!(d["metadata"]["summary"], "Make HTTP requests");
        // author is an object {name} per describe-v2.
        assert_eq!(d["metadata"]["author"]["name"], "greentic");
        assert_eq!(d["metadata"]["license"], "Apache-2.0");
    }

    #[test]
    fn store_id_validation() {
        assert!(is_valid_store_id("greentic.component-http"));
        assert!(is_valid_store_id("acme.foo.bar"));
        assert!(!is_valid_store_id("component-http")); // no dot
        assert!(!is_valid_store_id("Greentic.X")); // uppercase
        assert!(!is_valid_store_id(".leading"));
        assert!(!is_valid_store_id("trailing."));
    }

    #[test]
    fn offered_one_capref_per_operation_with_valid_id() {
        let d = sample();
        let offered = d["capabilities"]["offered"].as_array().unwrap();
        assert_eq!(offered.len(), 2);
        assert_eq!(offered[0]["id"], "component:component-http/run");
        // Spaces/upper coerced into the capRef pattern.
        assert_eq!(offered[1]["id"], "component:component-http/health-check");
        assert_eq!(offered[0]["version"], "0.1.0");
    }

    #[test]
    fn runtime_component_holds_sha_and_world() {
        let d = sample();
        let comp = &d["runtime"]["components"]["component-http"];
        assert_eq!(comp["sha256"], "ab".repeat(32));
        assert_eq!(comp["world"], "greentic:component@0.6.0");
    }

    #[test]
    fn contract_version_read_from_world() {
        assert_eq!(
            contract_version_from_world("greentic:component@0.6.0"),
            "0.6.0"
        );
        assert_eq!(contract_version_from_world("no-version-here"), "0.6.0");
        assert_eq!(contract_version_from_world("x@1.2.3-rc.1"), "1.2.3-rc.1");
    }

    #[test]
    fn sanitize_guarantees_leading_alpha() {
        assert_eq!(sanitize_segment("9lives"), "c9lives");
        assert_eq!(sanitize_segment("Foo Bar"), "foo-bar");
    }
}
