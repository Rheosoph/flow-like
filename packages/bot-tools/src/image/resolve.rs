use std::sync::Arc;

use flow_like_runtime::bit::{Bit, BitTypes};
use flow_like_runtime::profile::Profile;
use flow_like_runtime::state::FlowLikeState;

use super::ImageUnavailable;

const DEFAULT_SCAN_LIMIT: usize = 12;

/// Whether the Bit is a hosted image generation model: the API's proxy runs it, so a host
/// holding only a run token can use it.
pub fn is_hosted_image_bit(bit: &Bit) -> bool {
    is_image_capable(bit)
        && bit.parameters["provider"]["provider_name"]
            .as_str()
            .is_some_and(flow_like_model_provider::provider::is_hosted_provider_name)
}

fn is_image_capable(bit: &Bit) -> bool {
    matches!(bit.bit_type, BitTypes::ImageGeneration)
}

/// Whether the Bit runs on the machine from stable-diffusion.cpp weights, not at a provider.
pub fn is_local_image_bit(bit: &Bit) -> bool {
    bit.parameters["provider"]["provider_name"].as_str()
        == Some(flow_like_model_provider::stablediffusion::PROVIDER_NAME)
}

/// Whether the Bit is an image model at a provider the owner signs in to with their own key;
/// the Bit's provider `params` carry that key.
pub fn is_own_key_image_bit(bit: &Bit) -> bool {
    is_image_capable(bit) && !is_hosted_image_bit(bit) && !is_local_image_bit(bit)
}

/// Takes the provider `params` (keys, endpoints) out of the Bit.
pub fn take_provider_params(bit: &mut Bit) -> Option<serde_json::Value> {
    bit.parameters
        .get_mut("provider")?
        .as_object_mut()?
        .remove("params")
}

/// The Bit with provider `params` put back, for one call.
pub fn with_provider_params(mut bit: Bit, params: serde_json::Value) -> Bit {
    if let Some(provider) = bit
        .parameters
        .get_mut("provider")
        .and_then(serde_json::Value::as_object_mut)
    {
        provider.insert("params".into(), params);
    }
    bit
}

/// The bot's `image_model`, else the first image generation Bit the profile lists (its own
/// library first, then its hub references).
pub async fn resolve_bit(
    profile: &Profile,
    state: &Arc<FlowLikeState>,
    image_model: Option<&str>,
) -> Result<Bit, ImageUnavailable> {
    if let Some(id) = image_model.map(str::trim).filter(|id| !id.is_empty()) {
        return profile
            .find_bit(id, state.http_client.clone())
            .await
            .map_err(|error| {
                ImageUnavailable::Unusable(format!(
                    "the image model '{id}' was not found: {error:#}"
                ))
            });
    }
    let listed = |reference: &String| {
        reference
            .rsplit_once(':')
            .map_or(reference.as_str(), |(_, id)| id)
            .to_string()
    };
    for reference in &profile.bits {
        if let Some(bit) = profile
            .custom_bit(&listed(reference))
            .filter(is_image_capable)
        {
            return Ok(bit);
        }
    }
    for reference in profile.bits.iter().take(DEFAULT_SCAN_LIMIT) {
        if profile.custom_bit(&listed(reference)).is_some() {
            continue;
        }
        if let Ok(bit) = profile
            .resolve_model_reference(reference, state.http_client.clone())
            .await
            && is_image_capable(&bit)
        {
            return Ok(bit);
        }
    }
    Err(ImageUnavailable::NoModel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn image_bit(provider_name: &str) -> Bit {
        Bit {
            bit_type: BitTypes::ImageGeneration,
            parameters: json!({ "provider": {
                "provider_name": provider_name, "model_id": "m",
                "params": { "api_key": "owner-key" },
            }}),
            ..Default::default()
        }
    }

    #[test]
    fn image_bits_are_hosted_local_or_signed_in_with_the_owners_key() {
        let hosted = image_bit("hosted:openrouter");
        let local = image_bit(flow_like_model_provider::stablediffusion::PROVIDER_NAME);
        let own_key = image_bit("openai");
        assert!(is_hosted_image_bit(&hosted) && !is_own_key_image_bit(&hosted));
        assert!(is_local_image_bit(&local) && !is_own_key_image_bit(&local));
        assert!(is_own_key_image_bit(&own_key) && !is_local_image_bit(&own_key));
    }

    #[test]
    fn provider_params_leave_the_bit_and_come_back_for_one_call() {
        let mut bit = image_bit("openai");
        let params = take_provider_params(&mut bit).expect("the key was there");
        assert_eq!(params["api_key"], "owner-key");
        assert!(bit.parameters["provider"].get("params").is_none());
        assert_eq!(bit.parameters["provider"]["provider_name"], "openai");
        let signed = with_provider_params(bit, params);
        assert_eq!(
            signed.parameters["provider"]["params"]["api_key"],
            "owner-key"
        );
    }
}
