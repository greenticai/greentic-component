#![cfg(feature = "cli")]

#[path = "support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use greentic_component::prepare_component_with_manifest;

const PREPARE_TIMEOUT: Duration = Duration::from_secs(5);
const PREPARE_ITERATIONS: usize = 24;

#[test]
fn prepare_hot_path_finishes_quickly() {
    let component = support::fixtures::good_component();
    let manifest_path = component.manifest_path.to_string_lossy().into_owned();

    let start = Instant::now();
    for _ in 0..PREPARE_ITERATIONS {
        let prepared = prepare_component_with_manifest(&manifest_path, None)
            .expect("prepare fixture component");
        assert!(prepared.hash_verified);
        assert!(prepared.world_ok);
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed < PREPARE_TIMEOUT,
        "prepare workload exceeded timeout: {elapsed:?} for {PREPARE_ITERATIONS} iterations",
    );
}
