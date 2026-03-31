use anyhow::{Result, anyhow};
use wasmparser::Parser;
use wit_component::{DecodedWasm, metadata};
use wit_parser::{Resolve, WorldId};

/// Indicates how a world was decoded from a wasm binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldSource {
    /// Metadata was decoded from a core wasm module.
    Metadata,
    /// WIT was reconstructed from a fully-fledged component.
    Component,
}

/// Resolved world information extracted from a wasm binary.
pub struct DecodedWorld {
    pub resolve: Resolve,
    pub world: WorldId,
    pub source: WorldSource,
}

/// Decode a wasm module or component into its WIT world description.
pub fn decode_world(bytes: &[u8]) -> Result<DecodedWorld> {
    if Parser::is_component(bytes) {
        match wit_component::decode(bytes) {
            Ok(DecodedWasm::Component(resolve, world)) => Ok(DecodedWorld {
                resolve,
                world,
                source: WorldSource::Component,
            }),
            Ok(DecodedWasm::WitPackage(_, _)) | Err(_) => match metadata::decode(bytes) {
                Ok((_maybe_module, bindgen)) => Ok(DecodedWorld {
                    resolve: bindgen.resolve,
                    world: bindgen.world,
                    source: WorldSource::Metadata,
                }),
                Err(module_err) => Err(anyhow!(
                    "failed to decode component and module metadata ({module_err})"
                )),
            },
        }
    } else {
        match metadata::decode(bytes) {
            Ok((_maybe_module, bindgen)) => Ok(DecodedWorld {
                resolve: bindgen.resolve,
                world: bindgen.world,
                source: WorldSource::Metadata,
            }),
            Err(module_err) => match wit_component::decode(bytes) {
                Ok(DecodedWasm::Component(resolve, world)) => Ok(DecodedWorld {
                    resolve,
                    world,
                    source: WorldSource::Component,
                }),
                Ok(DecodedWasm::WitPackage(_, _)) => Err(module_err),
                Err(component_err) => Err(anyhow!(
                    "failed to decode module metadata ({module_err}) and component ({component_err})"
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn decode_world_rejects_invalid_binary_with_combined_error() {
        match decode_world(b"not a wasm") {
            Ok(_) => panic!("invalid input should fail"),
            Err(err) => {
                let message = err.to_string();
                assert!(
                    message.contains("failed to decode")
                        || message.contains("unexpected end-of-file")
                        || message.contains("invalid")
                );
            }
        }
    }

    #[test]
    fn decode_world_accepts_contract_fixture_component() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("contract")
            .join("fixtures")
            .join("component_v0_6_0")
            .join("component.wasm");
        let bytes = std::fs::read(path).expect("read contract fixture");

        let decoded = decode_world(&bytes).expect("decode fixture component");

        assert_eq!(decoded.source, WorldSource::Component);
    }
}
