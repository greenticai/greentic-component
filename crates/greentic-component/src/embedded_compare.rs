use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::embedded_descriptor::{EmbeddedComponentManifestV1, build_embedded_manifest_projection};
use crate::manifest::ComponentManifest;
use greentic_types::schemas::component::v0_6_0::ComponentDescribe;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonStatus {
    Match,
    Mismatch,
    MissingLeft,
    MissingRight,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldComparison {
    pub field: String,
    pub status: ComparisonStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmbeddedManifestComparisonReport {
    pub overall: ComparisonStatus,
    pub fields: Vec<FieldComparison>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeProjection {
    pub id: String,
    pub version: String,
    pub operation_names: BTreeSet<String>,
}

pub fn compare_embedded_with_manifest(
    embedded: &EmbeddedComponentManifestV1,
    manifest: &ComponentManifest,
) -> EmbeddedManifestComparisonReport {
    let canonical_projection = build_embedded_manifest_projection(manifest);
    compare_embedded_projection(embedded, &canonical_projection)
}

pub fn compare_embedded_with_describe(
    embedded: &EmbeddedComponentManifestV1,
    describe: &ComponentDescribe,
) -> EmbeddedManifestComparisonReport {
    let describe_projection = build_describe_projection(describe);
    let embedded_ops: BTreeSet<String> = embedded
        .operations
        .iter()
        .map(|op| op.name.clone())
        .collect();
    finalize_report(vec![
        compare_scalar("id", &embedded.id, &describe_projection.id),
        compare_scalar("version", &embedded.version, &describe_projection.version),
        compare_set(
            "operation_names",
            &embedded_ops,
            &describe_projection.operation_names,
        ),
    ])
}

pub fn build_describe_projection(describe: &ComponentDescribe) -> DescribeProjection {
    let operation_names = describe
        .operations
        .iter()
        .map(|op| op.id.clone())
        .collect::<BTreeSet<_>>();
    DescribeProjection {
        id: describe.info.id.clone(),
        version: describe.info.version.clone(),
        operation_names,
    }
}

fn compare_embedded_projection(
    left: &EmbeddedComponentManifestV1,
    right: &EmbeddedComponentManifestV1,
) -> EmbeddedManifestComparisonReport {
    let left_ops: BTreeMap<String, String> = left
        .operations
        .iter()
        .map(|op| {
            (
                op.name.clone(),
                format!("{:?}|{:?}", op.input_schema, op.output_schema),
            )
        })
        .collect();
    let right_ops: BTreeMap<String, String> = right
        .operations
        .iter()
        .map(|op| {
            (
                op.name.clone(),
                format!("{:?}|{:?}", op.input_schema, op.output_schema),
            )
        })
        .collect();

    finalize_report(vec![
        compare_scalar("id", &left.id, &right.id),
        compare_scalar("name", &left.name, &right.name),
        compare_scalar("version", &left.version, &right.version),
        compare_debug("supports", &left.supports, &right.supports),
        compare_scalar("world", &left.world, &right.world),
        compare_debug("capabilities", &left.capabilities, &right.capabilities),
        compare_debug(
            "secret_requirements",
            &left.secret_requirements,
            &right.secret_requirements,
        ),
        compare_debug("profiles", &left.profiles, &right.profiles),
        compare_debug("configurators", &left.configurators, &right.configurators),
        compare_debug("limits", &left.limits, &right.limits),
        compare_debug("telemetry", &left.telemetry, &right.telemetry),
        compare_scalar(
            "describe_export",
            &left.describe_export,
            &right.describe_export,
        ),
        compare_map("operations", &left_ops, &right_ops),
        compare_debug(
            "default_operation",
            &left.default_operation,
            &right.default_operation,
        ),
        compare_debug("provenance", &left.provenance, &right.provenance),
    ])
}

fn compare_scalar(field: &str, left: &str, right: &str) -> FieldComparison {
    if left == right {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Match,
            detail: None,
        }
    } else {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Mismatch,
            detail: Some(format!("left={left:?}, right={right:?}")),
        }
    }
}

fn compare_debug<T: std::fmt::Debug + PartialEq>(
    field: &str,
    left: &T,
    right: &T,
) -> FieldComparison {
    if left == right {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Match,
            detail: None,
        }
    } else {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Mismatch,
            detail: Some(format!("left={left:?}, right={right:?}")),
        }
    }
}

fn compare_set(field: &str, left: &BTreeSet<String>, right: &BTreeSet<String>) -> FieldComparison {
    if left == right {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Match,
            detail: None,
        }
    } else {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Mismatch,
            detail: Some(format!("left={left:?}, right={right:?}")),
        }
    }
}

fn compare_map(
    field: &str,
    left: &BTreeMap<String, String>,
    right: &BTreeMap<String, String>,
) -> FieldComparison {
    if left == right {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Match,
            detail: None,
        }
    } else {
        FieldComparison {
            field: field.to_string(),
            status: ComparisonStatus::Mismatch,
            detail: Some(format!("left={left:?}, right={right:?}")),
        }
    }
}

fn finalize_report(fields: Vec<FieldComparison>) -> EmbeddedManifestComparisonReport {
    let overall = if fields
        .iter()
        .all(|field| field.status == ComparisonStatus::Match)
    {
        ComparisonStatus::Match
    } else {
        ComparisonStatus::Mismatch
    };
    EmbeddedManifestComparisonReport { overall, fields }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_manifest;
    use serde_json::json;

    fn manifest() -> ComponentManifest {
        parse_manifest(
            &json!({
                "id": "ai.greentic.example",
                "name": "example",
                "version": "0.1.0",
                "world": "greentic:component/component@0.6.0",
                "describe_export": "describe",
                "operations": [{
                    "name": "handle_message",
                    "input_schema": {"type":"object","properties":{},"required":[]},
                    "output_schema": {"type":"object","properties":{},"required":[]}
                }],
                "default_operation": "handle_message",
                "config_schema": {"type":"object","properties":{},"required":[],"additionalProperties":false},
                "supports": ["messaging"],
                "profiles": {"default":"stateless","supported":["stateless"]},
                "secret_requirements": [],
                "capabilities": {
                    "wasi": {"filesystem":{"mode":"none","mounts":[]},"random":true,"clocks":true},
                    "host": {"messaging":{"inbound":true,"outbound":true}, "secrets":{"required":[]}}
                },
                "artifacts": {"component_wasm":"component.wasm"},
                "hashes": {"component_wasm":"blake3:0000000000000000000000000000000000000000000000000000000000000000"}
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn manifest_projection_comparison_matches() {
        let manifest = manifest();
        let embedded = build_embedded_manifest_projection(&manifest);
        let report = compare_embedded_with_manifest(&embedded, &manifest);
        assert_eq!(report.overall, ComparisonStatus::Match);
    }
}
