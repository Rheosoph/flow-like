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
