#![cfg(feature = "cli")]

use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use greentic_component::{collect_default_annotations, collect_redactions};

const OPS_PER_THREAD: usize = 48;
const SCALING_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn schema_introspection_parallel_throughput_does_not_collapse() {
    let schema = Arc::new(build_schema_workload());
    let single = run_schema_workload(schema.clone(), 1, OPS_PER_THREAD);

    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let parallelism = available.min(4);
    if parallelism <= 1 {
        return;
    }

    let parallel = run_schema_workload(schema, parallelism, OPS_PER_THREAD);
    let single_per_op = single.as_secs_f64() / OPS_PER_THREAD as f64;
    let parallel_per_op = parallel.as_secs_f64() / (OPS_PER_THREAD * parallelism) as f64;

    assert!(
        parallel_per_op <= single_per_op * 1.75,
        "parallel schema traversal degraded too much: threads={parallelism}, single/op={single_per_op:.6}s, parallel/op={parallel_per_op:.6}s",
    );
}

fn run_schema_workload(schema: Arc<String>, threads: usize, ops_per_thread: usize) -> Duration {
    let start = Instant::now();
    let mut handles = Vec::with_capacity(threads);

    for _ in 0..threads {
        let schema = schema.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..ops_per_thread {
                let redactions = collect_redactions(&schema);
                let defaults =
                    collect_default_annotations(&schema).expect("collect default annotations");
                assert!(!redactions.is_empty());
                assert!(!defaults.is_empty());
            }
        }));
    }

    for handle in handles {
        handle.join().expect("schema workload thread panicked");
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed < SCALING_TIMEOUT,
        "parallel schema traversal timed out after {elapsed:?} with {threads} threads",
    );
    elapsed
}

fn build_schema_workload() -> String {
    let fixture = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/schemas/redaction.schema.json"),
    )
    .expect("read schema fixture");
    let seed_json: serde_json::Value = serde_json::from_str(&fixture).expect("schema fixture json");
    let mut properties = serde_json::Map::new();
    for idx in 0..32 {
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
