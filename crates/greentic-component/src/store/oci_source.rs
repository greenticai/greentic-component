use anyhow::{Result, anyhow};
use bytes::Bytes;
use oci_distribution::Reference;
use oci_distribution::client::{Client, ClientConfig};
use oci_distribution::secrets::RegistryAuth;

pub async fn fetch(reference: &str) -> Result<Bytes> {
    let reference: Reference = reference
        .parse()
        .map_err(|err| anyhow!("invalid OCI reference '{reference}': {err}"))?;

    let client = Client::new(ClientConfig::default());
    let auth = RegistryAuth::Anonymous;
    let accepted_media_types = accepted_media_types();
    let image = client.pull(&reference, &auth, accepted_media_types).await?;

    let bytes = select_component_layer(
        image
            .layers
            .into_iter()
            .map(|layer| (layer.media_type, layer.data)),
    )
    .ok_or_else(|| anyhow!("no suitable component layer found for {reference}"))?;
    Ok(Bytes::from(bytes))
}

fn accepted_media_types() -> Vec<&'static str> {
    vec![
        "application/wasm",
        "application/octet-stream",
        "application/vnd.module.wasm.content.layer.v1+wasm",
        "application/vnd.module.wasm.content.layer.v1+tar",
    ]
}

fn select_component_layer(layers: impl IntoIterator<Item = (String, Vec<u8>)>) -> Option<Vec<u8>> {
    for (media_type, data) in layers {
        let media_type = media_type.to_ascii_lowercase();
        if media_type.contains("application/wasm") || media_type.contains("octet-stream") {
            return Some(data);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_rejects_invalid_oci_reference_before_network() {
        let err = fetch("not a valid reference")
            .await
            .expect_err("should fail");
        assert!(err.to_string().contains("invalid OCI reference"));
    }

    #[test]
    fn accepted_media_types_include_wasm_and_octet_stream_variants() {
        let media_types = accepted_media_types();
        assert!(media_types.contains(&"application/wasm"));
        assert!(media_types.contains(&"application/octet-stream"));
        assert!(media_types.contains(&"application/vnd.module.wasm.content.layer.v1+wasm"));
    }

    #[test]
    fn select_component_layer_prefers_first_matching_wasm_payload() {
        let selected = select_component_layer(vec![
            ("application/json".to_string(), b"meta".to_vec()),
            ("application/octet-stream".to_string(), b"wasm".to_vec()),
            ("application/wasm".to_string(), b"second".to_vec()),
        ]);

        assert_eq!(selected, Some(b"wasm".to_vec()));
    }

    #[test]
    fn select_component_layer_returns_none_when_no_supported_layers_exist() {
        let selected = select_component_layer(vec![
            ("application/json".to_string(), b"meta".to_vec()),
            ("text/plain".to_string(), b"readme".to_vec()),
        ]);

        assert!(selected.is_none());
    }
}
