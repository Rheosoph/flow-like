use std::sync::Arc;

use async_trait::async_trait;
use flow_like_catalog_media_image::image::generation::service::{self, HostedAccess, ImageRequest};
use flow_like_model_provider::provider::ModelProvider;
use flow_like_runtime::bit::{Bit, BitTypes};
use flow_like_runtime::flow::execution::ExecutionEnvironment;
use flow_like_runtime::models::generation::resolve_generation_provider;
use flow_like_runtime::models::llm::ModelUsageContext;
use flow_like_runtime::profile::Profile;
use flow_like_runtime::state::FlowLikeState;
use flow_like_types::Bytes;

use super::{
    GeneratedImage, ImageBackend, ImageBudget, ImageJob, ImageModelInfo, ImageUnavailable,
};

/// The media-image catalog's providers behind `image_generate`: one resolved image Bit.
pub struct CatalogImages {
    bit: Bit,
    provider: ModelProvider,
    environment: ExecutionEnvironment,
    hosted: Option<HostedAccess>,
    budget: Arc<ImageBudget>,
}

fn is_stable_diffusion(provider: &ModelProvider) -> bool {
    provider.provider_name == flow_like_model_provider::stablediffusion::PROVIDER_NAME
}

impl CatalogImages {
    /// `bit` must be an image generation Bit or an image-capable VLM. Local stable-diffusion
    /// resolves its weight packs from the Bit store; phones refuse it.
    pub async fn resolve(
        bit: Bit,
        profile: &Profile,
        state: &Arc<FlowLikeState>,
        environment: ExecutionEnvironment,
        hosted: Option<HostedAccess>,
        budget: Arc<ImageBudget>,
    ) -> Result<Self, ImageUnavailable> {
        if !matches!(bit.bit_type, BitTypes::ImageGeneration | BitTypes::Vlm) {
            return Err(ImageUnavailable::Unusable(format!(
                "'{}' is a {:?} model, not an image model",
                bit.id, bit.bit_type
            )));
        }
        let provider = service::bit_provider(&bit)
            .map_err(|error| ImageUnavailable::Unusable(format!("{error:#}")))?;
        if is_stable_diffusion(&provider) && environment == ExecutionEnvironment::Mobile {
            return Err(ImageUnavailable::Unusable(
                "Local image generation runs on desktop only; pick a remote image model".into(),
            ));
        }
        if is_stable_diffusion(&provider) && environment == ExecutionEnvironment::Server {
            return Err(ImageUnavailable::Unusable(
                "Local image generation runs on the owner's desktop, not on a server; pick a remote image model".into(),
            ));
        }
        let provider = resolve_generation_provider(&bit, &provider, profile, state)
            .await
            .map_err(|error| ImageUnavailable::Unusable(format!("{error:#}")))?;
        Ok(Self {
            bit,
            provider,
            environment,
            hosted,
            budget,
        })
    }
}

#[async_trait]
impl ImageBackend for CatalogImages {
    fn model(&self) -> ImageModelInfo {
        ImageModelInfo {
            name: self
                .provider
                .model_id
                .clone()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| self.bit.id.clone()),
            external: !is_stable_diffusion(&self.provider),
        }
    }

    fn budget(&self) -> &ImageBudget {
        &self.budget
    }

    async fn generate(&self, job: ImageJob) -> flow_like_types::Result<Vec<GeneratedImage>> {
        let request = ImageRequest {
            prompt: job.prompt,
            negative_prompt: job.negative_prompt,
            count: job.count,
            size: Some(job.size),
            seed: job.seed,
        };
        let images = service::generate(
            self.environment,
            &self.bit,
            &self.provider,
            &request,
            self.hosted.as_ref(),
        )
        .await?;
        Ok(images
            .into_iter()
            .map(|image| GeneratedImage {
                bytes: Bytes::from(image.bytes),
                mime: image.mime,
                metadata: image.metadata,
            })
            .collect())
    }
}

const DEFAULT_SCAN_LIMIT: usize = 12;

/// The credentials a hosted image model needs, from the run's token and usage labels.
pub fn hosted_access(
    state: &FlowLikeState,
    token: Option<String>,
    usage: Option<&ModelUsageContext>,
) -> HostedAccess {
    HostedAccess {
        token: state.hosted_model_token.clone().or(token),
        authorizer: state.request_authorizer.clone(),
        api_base_url: usage.and_then(|usage| usage.api_base_url.clone()),
        app_id: usage.and_then(|usage| usage.app_id.clone()),
        run_id: usage.and_then(|usage| usage.run_id.clone()),
    }
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
