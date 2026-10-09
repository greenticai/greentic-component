//! `greentic-component store publish` — produce the Greentic Store's describe-v2
//! for a component and upload it as a `ComponentExtension`.
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
//! `--dry-run` prints the describe-v2; otherwise the component is packed into a
//! `.gtxpack` (`describe.json` + `component.wasm` + a packc-owned `component.json`
//! sidecar pointing at the embedded wasm so the `ext://<id>#component` resolver
//! can find it) and uploaded to the store's `POST /api/v1/extensions` (multipart,
//! bearer auth). The sidecar is embedded in the artifact only.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use greentic_types::SecretRequirement;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::manifest::parse_manifest;

/// ZIP entry name of the runtime component wasm inside the `.gtxpack`. Used for
/// both the archive entry and the `component.json` sidecar `asset` so the two
/// cannot drift.
const COMPONENT_WASM_ENTRY: &str = "component.wasm";

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
    /// Full secret requirements declared by the component manifest. Rendered
    /// verbatim into the canonical top-level `requiredSecrets` array (omitted
    /// when empty). These are read-requirement declarations, NOT permission
    /// grants, so they are never written into `runtime.permissions.secrets`.
    pub secret_requirements: &'a [SecretRequirement],
}

/// Map a component to the store's describe-v2 (`kind=ComponentExtension`).
pub(crate) fn build_component_describe_v2(i: &DescribeInputs) -> Value {
    let offered: Vec<Value> = i
        .operation_names
        .iter()
        .map(|op| json!({ "id": capability_id(i.component_id, op), "version": i.version }))
        .collect();

    let mut describe = json!({
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
            // `permissions` carries read-permission GRANTS only; declared
            // secret requirements live in the top-level `requiredSecrets`
            // array below, never here.
            "permissions": {},
            "components": {
                i.component_id: { "sha256": i.wasm_sha256_hex, "world": i.world }
            }
        },
        "contributions": {}
    });

    // Canonical `requiredSecrets`: the full `SecretRequirement` objects from the
    // manifest, serialized verbatim (camelCase per greentic-types). Omitted when
    // there are no declared secrets.
    if !i.secret_requirements.is_empty() {
        // `to_value` of a plain serde-derived struct is infallible in practice;
        // skip any entry that somehow fails rather than panicking on a publish path.
        let required_secrets: Vec<Value> = i
            .secret_requirements
            .iter()
            .filter_map(|req| serde_json::to_value(req).ok())
            .collect();
        if let Value::Object(map) = &mut describe {
            map.insert(
                "requiredSecrets".to_string(),
                Value::Array(required_secrets),
            );
        }
    }

    describe
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
        secret_requirements: &manifest.secret_requirements,
    });

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&describe)?);
        return Ok(());
    }

    let store_url = args
        .store_url
        .clone()
        .or_else(|| std::env::var("GREENTIC_STORE_URL").ok())
        .ok_or_else(|| anyhow!("--store-url or GREENTIC_STORE_URL is required to publish"))?;
    let token = args
        .token
        .clone()
        .or_else(|| std::env::var("GREENTIC_STORE_TOKEN").ok())
        .ok_or_else(|| anyhow!("--token or GREENTIC_STORE_TOKEN is required to publish"))?;

    // Pack `{describe.json, component.wasm, component.json}` into a .gtxpack. The
    // store cross-checks the archive's describe.json against the `metadata.describe`
    // we send and re-signs it, so both must be byte-identical — serialize once.
    let describe_bytes = serde_json::to_vec(&describe)?;
    // packc-owned sidecar pointing at the embedded `component.wasm`, so the
    // `ext://<id>#component` resolver can locate the runtime component. Embedded
    // in the artifact only — it is not part of the upload metadata.
    let component_json = build_component_sidecar(&store_id, &wasm_sha256_hex);
    let gtxpack =
        build_gtxpack(&describe_bytes, &wasm, &component_json).context("build .gtxpack")?;
    let artifact_sha256 = hex::encode(Sha256::digest(&gtxpack));

    let metadata = serde_json::to_string(&json!({
        "describe": describe,
        "artifactSha256": artifact_sha256,
    }))?;

    let url = format!("{}/api/v1/extensions", store_url.trim_end_matches('/'));
    let artifact = reqwest::blocking::multipart::Part::bytes(gtxpack)
        .file_name(format!("{component_id}.gtxpack"))
        .mime_str("application/zip")?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("metadata", metadata)
        .part("artifact", artifact);

    let resp = reqwest::blocking::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("store publish failed: HTTP {status}: {body}");
    }
    println!("published {store_id} {version} ({artifact_sha256}) to {url}");
    Ok(())
}

/// Build the packc-owned `component.json` sidecar bytes for the embedded
/// `component.wasm`. The `asset` always equals [`COMPONENT_WASM_ENTRY`] so it
/// cannot drift from the actual ZIP entry; `digest` is `sha256:<hex>` over the
/// asset bytes (the same hash the store records).
fn build_component_sidecar(store_id: &str, wasm_sha256_hex: &str) -> Vec<u8> {
    let sidecar = json!({
        "component": {
            "id": store_id,
            "asset": COMPONENT_WASM_ENTRY,
            "digest": format!("sha256:{wasm_sha256_hex}"),
        }
    });
    // Serialization of a fixed `json!` object is infallible in practice; fall
    // back to an empty `{}` rather than panicking on a publish path.
    serde_json::to_vec(&sidecar).unwrap_or_else(|_| b"{}".to_vec())
}

/// Pack a component into a `.gtxpack` (ZIP) with `describe.json` at the root, the
/// component wasm at `component.wasm`, and the packc-owned `component.json`
/// sidecar. The store requires `describe.json` at the archive root; the resolver
/// reads `component.json`.
fn build_gtxpack(describe_json: &[u8], wasm: &[u8], component_json: &[u8]) -> Result<Vec<u8>> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    let mut buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("describe.json", opts)
            .context("write describe.json")?;
        zw.write_all(describe_json)?;
        zw.start_file(COMPONENT_WASM_ENTRY, opts)
            .context("write component.wasm")?;
        zw.write_all(wasm)?;
        zw.start_file("component.json", opts)
            .context("write component.json")?;
        zw.write_all(component_json)?;
        zw.finish().context("finalize .gtxpack")?;
    }
    Ok(buf)
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
            secret_requirements: &[],
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
    fn gtxpack_round_trips_describe_and_wasm() {
        use std::io::Read;
        let describe = br#"{"kind":"ComponentExtension"}"#;
        let wasm = b"\0asm-fake-bytes";
        let component_json = br#"{"component":{}}"#;
        let pack = build_gtxpack(describe, wasm, component_json).unwrap();

        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(pack)).unwrap();
        let mut got_describe = Vec::new();
        zip.by_name("describe.json")
            .unwrap()
            .read_to_end(&mut got_describe)
            .unwrap();
        let mut got_wasm = Vec::new();
        zip.by_name(COMPONENT_WASM_ENTRY)
            .unwrap()
            .read_to_end(&mut got_wasm)
            .unwrap();
        assert_eq!(got_describe, describe);
        assert_eq!(got_wasm, wasm);
    }

    #[test]
    fn gtxpack_embeds_component_json_sidecar() {
        use std::io::Read;
        let describe = br#"{"kind":"ComponentExtension"}"#;
        let wasm = b"\0asm-fake-bytes";
        let store_id = "greentic.component-http";
        let wasm_sha256_hex = hex::encode(Sha256::digest(wasm));
        let component_json = build_component_sidecar(store_id, &wasm_sha256_hex);
        let pack = build_gtxpack(describe, wasm, &component_json).unwrap();

        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(pack)).unwrap();

        // Read the embedded component.wasm bytes back out so the digest check
        // proves the sidecar matches the *actual* embedded asset (no drift).
        let mut embedded_wasm = Vec::new();
        zip.by_name(COMPONENT_WASM_ENTRY)
            .expect("component.wasm entry")
            .read_to_end(&mut embedded_wasm)
            .unwrap();
        let expected_digest = format!("sha256:{}", hex::encode(Sha256::digest(&embedded_wasm)));

        let mut got_sidecar = Vec::new();
        zip.by_name("component.json")
            .expect("component.json entry")
            .read_to_end(&mut got_sidecar)
            .unwrap();
        let sidecar: Value = serde_json::from_slice(&got_sidecar).unwrap();

        assert_eq!(sidecar["component"]["id"], store_id);
        assert_eq!(sidecar["component"]["asset"], "component.wasm");
        assert_eq!(sidecar["component"]["asset"], COMPONENT_WASM_ENTRY);
        assert_eq!(sidecar["component"]["digest"], expected_digest);
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

    // ── Phase 2 (S1+S2): requiredSecrets at top-level, NOT in permissions ──────

    /// Build a `SecretRequirement` via deserialization (the struct is
    /// `#[non_exhaustive]`, so foreign crates cannot use a struct literal).
    fn make_req(
        key: &str,
        required: bool,
        description: Option<&str>,
    ) -> greentic_types::SecretRequirement {
        let mut obj = serde_json::Map::new();
        obj.insert("key".into(), json!(key));
        obj.insert("required".into(), json!(required));
        if let Some(desc) = description {
            obj.insert("description".into(), json!(desc));
        }
        serde_json::from_value(Value::Object(obj)).expect("valid SecretRequirement")
    }

    #[test]
    fn required_secrets_emitted_as_top_level_array() {
        // A manifest with two secret requirements must produce a top-level
        // `requiredSecrets` array containing full SecretRequirement objects.
        let reqs = vec![
            make_req("API_KEY", true, Some("The API key")),
            make_req("OPTIONAL_TOKEN", false, None),
        ];
        let d = build_component_describe_v2(&DescribeInputs {
            store_id: "greentic.component-http",
            component_id: "component-http",
            name: "HTTP Client",
            version: "0.1.0",
            summary: "Make HTTP requests",
            author: "greentic",
            license: "Apache-2.0",
            world: "greentic:component@0.6.0",
            wasm_sha256_hex: &"ab".repeat(32),
            operation_names: &["run".to_string()],
            secret_requirements: &reqs,
        });
        let arr = d["requiredSecrets"]
            .as_array()
            .expect("requiredSecrets must be an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["key"], "API_KEY");
        assert_eq!(arr[0]["required"], true);
        assert_eq!(arr[0]["description"], "The API key");
        assert_eq!(arr[1]["key"], "OPTIONAL_TOKEN");
        assert_eq!(arr[1]["required"], false);
        assert!(arr[1].get("description").is_none() || arr[1]["description"].is_null());
    }

    #[test]
    fn permissions_secrets_not_polluted_when_secrets_present() {
        // secret-keys MUST NOT appear in runtime.permissions.secrets.
        let reqs = vec![make_req("API_KEY", true, None)];
        let d = build_component_describe_v2(&DescribeInputs {
            store_id: "greentic.component-http",
            component_id: "component-http",
            name: "HTTP Client",
            version: "0.1.0",
            summary: "Make HTTP requests",
            author: "greentic",
            license: "Apache-2.0",
            world: "greentic:component@0.6.0",
            wasm_sha256_hex: &"ab".repeat(32),
            operation_names: &["run".to_string()],
            secret_requirements: &reqs,
        });
        // permissions must be `{}` — no "secrets" key inside it.
        assert_eq!(d["runtime"]["permissions"], serde_json::json!({}));
    }

    #[test]
    fn no_required_secrets_key_when_empty() {
        // When there are no secret requirements, `requiredSecrets` must be absent.
        let d = build_component_describe_v2(&DescribeInputs {
            store_id: "greentic.component-http",
            component_id: "component-http",
            name: "HTTP Client",
            version: "0.1.0",
            summary: "Make HTTP requests",
            author: "greentic",
            license: "Apache-2.0",
            world: "greentic:component@0.6.0",
            wasm_sha256_hex: &"ab".repeat(32),
            operation_names: &["run".to_string()],
            secret_requirements: &[],
        });
        assert!(
            d.get("requiredSecrets").is_none(),
            "requiredSecrets should be absent when empty, got: {}",
            d["requiredSecrets"]
        );
        // permissions unchanged.
        assert_eq!(d["runtime"]["permissions"], serde_json::json!({}));
    }
}
