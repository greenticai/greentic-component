use anyhow::{Context, Result, anyhow, bail};
use greentic_types::cbor::canonical;
use greentic_types::component::ComponentOperation;
use greentic_types::flow::FlowKind;
use greentic_types::{SecretRequirement, cbor::canonical::CanonicalError};
use serde::{Deserialize, Serialize};
use wasm_encoder::{CustomSection, Encode, Section};
use wasmparser::{Parser, Payload};

use crate::capabilities::{Capabilities, ComponentConfigurators, ComponentProfiles};
use crate::limits::Limits;
use crate::manifest::ComponentManifest;
use crate::provenance::Provenance;
use crate::telemetry::TelemetrySpec;

pub const EMBEDDED_COMPONENT_MANIFEST_SECTION_V1: &str = "greentic.component.manifest.v1";
pub const EMBEDDED_COMPONENT_MANIFEST_KIND_V1: &str = "greentic.component.manifest";
pub const EMBEDDED_COMPONENT_MANIFEST_PAYLOAD_SCHEMA_V1: &str = "greentic.component.manifest.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddedComponentDescriptorEnvelopeV1 {
    pub kind: String,
    pub version: u32,
    pub encoding: String,
    pub payload_schema: Option<String>,
    pub payload_hash_blake3: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddedComponentManifestV1 {
    pub id: String,
    pub name: String,
    pub version: String,
    pub supports: Vec<FlowKind>,
    pub world: String,
    pub capabilities: Capabilities,
    pub secret_requirements: Vec<SecretRequirement>,
    pub profiles: ComponentProfiles,
    pub configurators: Option<ComponentConfigurators>,
    pub limits: Option<Limits>,
    pub telemetry: Option<TelemetrySpec>,
    pub describe_export: String,
    pub operations: Vec<ComponentOperation>,
    pub default_operation: Option<String>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedEmbeddedDescriptorV1 {
    pub envelope: EmbeddedComponentDescriptorEnvelopeV1,
    pub manifest: EmbeddedComponentManifestV1,
    pub payload_bytes: Vec<u8>,
}

impl EmbeddedComponentManifestV1 {
    pub fn from_canonical_manifest(manifest: &ComponentManifest) -> Self {
        Self {
            id: manifest.id.as_str().to_string(),
            name: manifest.name.clone(),
            version: manifest.version.to_string(),
            supports: manifest.supports.clone(),
            world: manifest.world.as_str().to_string(),
            capabilities: manifest.capabilities.clone(),
            secret_requirements: manifest.secret_requirements.clone(),
            profiles: manifest.profiles.clone(),
            configurators: manifest.configurators.clone(),
            limits: manifest.limits.clone(),
            telemetry: manifest.telemetry.clone(),
            describe_export: manifest.describe_export.as_str().to_string(),
            operations: manifest.operations.clone(),
            default_operation: manifest.default_operation.clone(),
            provenance: manifest.provenance.clone(),
        }
    }
}

pub fn build_embedded_manifest_projection(
    manifest: &ComponentManifest,
) -> EmbeddedComponentManifestV1 {
    EmbeddedComponentManifestV1::from_canonical_manifest(manifest)
}

pub fn encode_embedded_component_descriptor_v1(
    manifest: &EmbeddedComponentManifestV1,
) -> Result<(Vec<u8>, EmbeddedComponentDescriptorEnvelopeV1, Vec<u8>)> {
    let payload = canonical::to_canonical_cbor_allow_floats(manifest)
        .map_err(|err| anyhow!("failed to encode embedded manifest payload: {err}"))?;
    let payload_hash_blake3 = blake3::hash(&payload).to_hex().to_string();
    let envelope = EmbeddedComponentDescriptorEnvelopeV1 {
        kind: EMBEDDED_COMPONENT_MANIFEST_KIND_V1.to_string(),
        version: 1,
        encoding: "application/cbor".to_string(),
        payload_schema: Some(EMBEDDED_COMPONENT_MANIFEST_PAYLOAD_SCHEMA_V1.to_string()),
        payload_hash_blake3,
        payload: payload.clone(),
    };
    let envelope_bytes = canonical::to_canonical_cbor_allow_floats(&envelope)
        .map_err(|err| anyhow!("failed to encode embedded manifest envelope: {err}"))?;
    Ok((envelope_bytes, envelope, payload))
}

pub fn decode_embedded_component_descriptor_v1(
    envelope_bytes: &[u8],
) -> Result<VerifiedEmbeddedDescriptorV1> {
    let envelope: EmbeddedComponentDescriptorEnvelopeV1 = canonical::from_cbor(envelope_bytes)
        .map_err(|err| anyhow!("failed to decode embedded manifest envelope: {err}"))?;
    verify_embedded_component_descriptor_v1(&envelope)
}

pub fn verify_embedded_component_descriptor_v1(
    envelope: &EmbeddedComponentDescriptorEnvelopeV1,
) -> Result<VerifiedEmbeddedDescriptorV1> {
    if envelope.kind != EMBEDDED_COMPONENT_MANIFEST_KIND_V1 {
        bail!("unexpected embedded manifest kind `{}`", envelope.kind);
    }
    if envelope.version != 1 {
        bail!(
            "unsupported embedded manifest version `{}`",
            envelope.version
        );
    }
    if envelope.encoding != "application/cbor" {
        bail!(
            "unsupported embedded manifest encoding `{}`",
            envelope.encoding
        );
    }
    let payload_hash = blake3::hash(&envelope.payload).to_hex().to_string();
    if payload_hash != envelope.payload_hash_blake3 {
        bail!(
            "embedded manifest payload hash mismatch: expected {}, found {}",
            envelope.payload_hash_blake3,
            payload_hash
        );
    }
    let canonical_payload =
        canonical::canonicalize_allow_floats(&envelope.payload).map_err(map_canonical_error)?;
    if canonical_payload != envelope.payload {
        bail!("embedded manifest payload is not canonical");
    }
    let manifest: EmbeddedComponentManifestV1 =
        canonical::from_cbor(&canonical_payload).map_err(map_canonical_error)?;
    Ok(VerifiedEmbeddedDescriptorV1 {
        envelope: envelope.clone(),
        manifest,
        payload_bytes: canonical_payload,
    })
}

pub fn append_embedded_component_manifest_section_v1(
    wasm_bytes: &[u8],
    envelope_bytes: &[u8],
) -> Vec<u8> {
    let mut output = wasm_bytes.to_vec();
    let section = CustomSection {
        name: EMBEDDED_COMPONENT_MANIFEST_SECTION_V1.into(),
        data: envelope_bytes.into(),
    };
    output.push(section.id());
    section.encode(&mut output);
    output
}

pub fn read_embedded_component_manifest_section_v1(wasm_bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    for payload in Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload.map_err(|err| anyhow!("failed to parse wasm: {err}"))?;
        if let Payload::CustomSection(section) = payload
            && section.name() == EMBEDDED_COMPONENT_MANIFEST_SECTION_V1
        {
            return Ok(Some(section.data().to_vec()));
        }
    }
    Ok(None)
}

pub fn read_and_verify_embedded_component_manifest_section_v1(
    wasm_bytes: &[u8],
) -> Result<Option<VerifiedEmbeddedDescriptorV1>> {
    let Some(section) = read_embedded_component_manifest_section_v1(wasm_bytes)? else {
        return Ok(None);
    };
    decode_embedded_component_descriptor_v1(&section).map(Some)
}

fn map_canonical_error(err: CanonicalError) -> anyhow::Error {
    anyhow!(err.to_string())
}

pub fn verify_embedded_projection_matches_canonical_manifest(
    projection: &EmbeddedComponentManifestV1,
    canonical_manifest: &ComponentManifest,
) -> Result<()> {
    let expected = build_embedded_manifest_projection(canonical_manifest);
    if projection != &expected {
        bail!("embedded manifest projection does not match canonical build-time manifest");
    }
    Ok(())
}

pub fn embed_and_verify_wasm(
    wasm_path: &std::path::Path,
    canonical_manifest: &ComponentManifest,
) -> Result<()> {
    let wasm_bytes = std::fs::read(wasm_path)
        .with_context(|| format!("failed to read wasm at {}", wasm_path.display()))?;
    let projection = build_embedded_manifest_projection(canonical_manifest);
    let (envelope_bytes, _envelope, _payload_bytes) =
        encode_embedded_component_descriptor_v1(&projection)?;
    let patched = append_embedded_component_manifest_section_v1(&wasm_bytes, &envelope_bytes);
    std::fs::write(wasm_path, &patched).with_context(|| {
        format!(
            "failed to write embedded manifest to {}",
            wasm_path.display()
        )
    })?;

    let verified = read_and_verify_embedded_component_manifest_section_v1(&patched)?
        .ok_or_else(|| anyhow!("embedded manifest section missing after write"))?;
    verify_embedded_projection_matches_canonical_manifest(&verified.manifest, canonical_manifest)?;

    let section_bytes = read_embedded_component_manifest_section_v1(&patched)?
        .ok_or_else(|| anyhow!("embedded manifest section missing after verification"))?;
    if section_bytes != envelope_bytes {
        bail!("embedded manifest envelope bytes changed during write/read verification");
    }
    Ok(())
}
