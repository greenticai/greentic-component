#![forbid(unsafe_code)]

#[cfg(feature = "abi")]
pub mod abi;
pub mod capabilities;
#[cfg(feature = "cli")]
pub mod config;
#[cfg(feature = "describe")]
pub mod describe;
#[cfg(feature = "cli")]
pub mod embedded_compare;
#[cfg(any(feature = "cli", feature = "abi", feature = "prepare"))]
pub mod embedded_descriptor;
pub mod error;
pub mod lifecycle;
pub mod limits;
#[cfg(feature = "loader")]
pub mod loader;
pub mod manifest;
pub mod path_safety;
#[cfg(feature = "prepare")]
pub mod prepare;
pub mod provenance;
pub mod schema;
pub mod schema_quality;
pub mod security;
pub mod signing;
pub mod telemetry;

pub mod store;

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
pub mod cmd;
#[cfg(feature = "cli")]
pub mod scaffold;
#[cfg(feature = "cli")]
pub mod test_harness;
#[cfg(any(
    feature = "abi",
    feature = "describe",
    feature = "prepare",
    feature = "cli"
))]
pub mod wasm;
#[cfg(feature = "cli")]
pub mod wizard;

#[cfg(feature = "abi")]
pub use abi::{AbiError, check_world, has_lifecycle};
pub use capabilities::{Capabilities, CapabilityError};
#[cfg(feature = "describe")]
pub use describe::{
    DescribeError, DescribePayload, DescribeVersion, from_embedded, from_exported_func,
    from_wit_world, load as load_describe,
};
#[cfg(feature = "cli")]
pub use embedded_compare::{
    ComparisonStatus, DescribeProjection, EmbeddedManifestComparisonReport, FieldComparison,
    build_describe_projection, compare_embedded_with_describe, compare_embedded_with_manifest,
};
#[cfg(any(feature = "cli", feature = "abi", feature = "prepare"))]
pub use embedded_descriptor::{
    EMBEDDED_COMPONENT_MANIFEST_SECTION_V1, EmbeddedComponentDescriptorEnvelopeV1,
    EmbeddedComponentManifestV1, VerifiedEmbeddedDescriptorV1,
    append_embedded_component_manifest_section_v1, build_embedded_manifest_projection,
    decode_embedded_component_descriptor_v1, embed_and_verify_wasm,
    encode_embedded_component_descriptor_v1,
    read_and_verify_embedded_component_manifest_section_v1,
    read_embedded_component_manifest_section_v1, verify_embedded_component_descriptor_v1,
    verify_embedded_projection_matches_canonical_manifest,
};
pub use error::ComponentError;
pub use lifecycle::Lifecycle;
pub use limits::{LimitError, LimitOverrides, Limits, defaults_dev, merge};
#[cfg(feature = "loader")]
pub use loader::{ComponentHandle, LoadError, discover};
pub use manifest::{
    Artifacts, ComponentManifest, DescribeExport, DescribeKind, Hashes, ManifestError, ManifestId,
    WasmHash, World, parse_manifest, schema as manifest_schema, validate_manifest,
};
#[cfg(feature = "prepare")]
pub use prepare::{
    PackEntry, PreparedComponent, RunnerConfig, clear_cache_for, prepare_component,
    prepare_component_with_manifest,
};
pub use provenance::{Provenance, ProvenanceError};
pub use schema::{
    JsonPath, collect_capability_hints, collect_default_annotations, collect_redactions,
};
pub use schema_quality::{SchemaQualityMode, SchemaQualityWarning, validate_operation_schemas};
pub type RedactionPath = JsonPath;
pub use security::{Profile, enforce_capabilities};
pub use signing::{
    DevPolicy, SignatureRef, SigningError, StrictPolicy, compute_wasm_hash, verify_manifest_hash,
    verify_wasm_hash,
};
pub use store::{
    CompatError, CompatPolicy, ComponentBytes, ComponentId, ComponentLocator, ComponentStore,
    MetaInfo, SourceId,
};
pub use telemetry::{TelemetrySpec, span_name};
