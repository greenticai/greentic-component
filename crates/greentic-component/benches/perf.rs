#![cfg(feature = "cli")]

use std::fs;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use greentic_component::config::{ConfigInferenceOptions, load_manifest_with_schema};
use greentic_component::{
    collect_default_annotations, collect_redactions, discover, prepare_component_with_manifest,
};

#[path = "../tests/support/mod.rs"]
mod support;

const REDACTION_BATCHES: usize = 24;

fn bench_schema_introspection(c: &mut Criterion) {
    let fixture = fs::read_to_string(schema_fixture_path()).expect("read schema fixture");
    let schema = build_schema_workload(&fixture, REDACTION_BATCHES);

    let mut group = c.benchmark_group("schema");
    group.bench_function("collect_redactions", |b| {
        b.iter(|| collect_redactions(&schema));
    });
    group.bench_function("collect_default_annotations", |b| {
        b.iter(|| collect_default_annotations(&schema).expect("default annotations"));
    });
    group.finish();
}

fn bench_loader_and_prepare(c: &mut Criterion) {
    let component = support::fixtures::good_component();
    let manifest_path = component.manifest_path.to_string_lossy().into_owned();

    let mut group = c.benchmark_group("component");
    group.bench_function(BenchmarkId::new("discover", "manifest-path"), |b| {
        b.iter(|| discover(&manifest_path).expect("discover fixture component"));
    });
    group.bench_function(
        BenchmarkId::new("prepare_component_with_manifest", "warm-cache"),
        |b| {
            b.iter(|| {
                prepare_component_with_manifest(&manifest_path, None)
                    .expect("prepare fixture component")
            });
        },
    );
    group.finish();
}

fn bench_config_inference(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let wit_dir = dir.path().join("wit");
    fs::create_dir_all(&wit_dir).expect("wit dir");
    fs::write(
        wit_dir.join("component.wit"),
        r#"
package greentic:component@0.6.0;
interface cfg {
  record config {
    /// Human description
    /// @default("hello")
    title: string,
    /// Secret field
    /// @flow:hidden
    secret: option<string>,
  }
}
world component {
  import cfg;
}
"#,
    )
    .expect("write wit");

    let manifest_path = dir.path().join("component.manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "world": "greentic:component/component@0.6.0"
        }))
        .expect("manifest json"),
    )
    .expect("write manifest");

    c.bench_function("config/load_manifest_with_schema/infer_from_wit", |b| {
        b.iter(|| {
            load_manifest_with_schema(&manifest_path, &ConfigInferenceOptions::default())
                .expect("config inference")
        });
    });
}

fn schema_fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schemas/redaction.schema.json")
}

fn build_schema_workload(seed: &str, batches: usize) -> String {
    let mut properties = serde_json::Map::new();
    let seed_json: serde_json::Value = serde_json::from_str(seed).expect("seed schema json");
    for idx in 0..batches {
        properties.insert(format!("payload_{idx}"), seed_json.clone());
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": [],
        "additionalProperties": false
    })
    .to_string()
}

criterion_group!(
    benches,
    bench_schema_introspection,
    bench_loader_and_prepare,
    bench_config_inference
);
criterion_main!(benches);
