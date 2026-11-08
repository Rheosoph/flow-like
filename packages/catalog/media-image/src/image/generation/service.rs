use super::*;

pub use super::hosted::HostedAccess;

pub const MAX_IMAGES_PER_REQUEST: u32 = 4;

/// One image request outside a flow run. Provider options stay at the Bit's defaults.
#[derive(Debug, Clone, Default)]
pub struct ImageRequest {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub count: u32,
    /// `<width>x<height>`.
    pub size: Option<String>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AiImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    /// Provider, model and `ai_generated: true`, plus whatever the provider reported.
    pub metadata: Value,
}

/// The provider a Bit names (image, VLM or LLM Bit), unresolved.
pub fn bit_provider(bit: &Bit) -> flow_like_types::Result<ModelProvider> {
    provider_from_bit(bit)
}

fn aspect_ratio_of(size: &str) -> Option<String> {
    let (width, height) = parse_size(Some(size))?;
    if width == 0 || height == 0 {
        return None;
    }
    let (mut divisor, mut remainder) = (width, height);
    while remainder != 0 {
        (divisor, remainder) = (remainder, divisor % remainder);
    }
    Some(format!("{}:{}", width / divisor, height / divisor))
}

fn imagen_ratio_of(size: &str) -> Option<String> {
    let (width, height) = parse_size(Some(size))?;
    if width == 0 || height == 0 {
        return None;
    }
    let target = f64::from(width) / f64::from(height);
    // Imagen's predict API accepts these ratios instead of pixel dimensions.
    [(1_u32, 1_u32), (4, 3), (3, 4), (16, 9), (9, 16)]
        .into_iter()
        .filter(|(w, h)| w.cmp(h) == width.cmp(&height))
        .min_by(|(aw, ah), (bw, bh)| {
            let distance = |w, h| ((f64::from(w) / f64::from(h)) / target).ln().abs();
            distance(*aw, *ah).total_cmp(&distance(*bw, *bh))
        })
        .map(|(width, height)| format!("{width}:{height}"))
}

fn stable_diffusion_options(
    provider: &ModelProvider,
    options: ImageGenerationProviderOptions,
    request: &ImageRequest,
) -> ImageGenerationProviderOptions {
    let is_stable_diffusion =
        provider.provider_name == flow_like_model_provider::stablediffusion::PROVIDER_NAME;
    let mut sd = match options {
        ImageGenerationProviderOptions::StableDiffusion(sd) => sd,
        ImageGenerationProviderOptions::Default if is_stable_diffusion => Default::default(),
        other => return other,
    };
    if let Some((width, height)) = parse_size(request.size.as_deref()) {
        sd.width = width;
        sd.height = height;
    }
    if let Some(negative) = &request.negative_prompt {
        sd.negative_prompt = Some(negative.clone());
    }
    if let Some(seed) = request.seed.and_then(|seed| i64::try_from(seed).ok()) {
        sd.seed = seed;
    }
    ImageGenerationProviderOptions::StableDiffusion(sd)
}

fn service_request(provider: &ModelProvider, request: &ImageRequest) -> ImageGenerationRequest {
    let mut size = request.size.clone();
    let mut aspect_ratio = size.as_deref().and_then(aspect_ratio_of);
    match provider.provider_name.as_str() {
        // OpenRouter resolves the requested dimensions against the model's advertised sizes.
        PROVIDER_OPENROUTER | "hosted:openrouter" | PROVIDER_OPENAI => aspect_ratio = None,
        PROVIDER_GEMINI | PROVIDER_VERTEX => {
            aspect_ratio = size.as_deref().and_then(imagen_ratio_of);
            size = None;
        }
        PROVIDER_XAI => size = None,
        _ => {}
    }
    ImageGenerationRequest {
        prompt: request.prompt.clone(),
        system_prompt: None,
        negative_prompt: request.negative_prompt.clone().and_then(optional_clean),
        count: request.count.clamp(1, MAX_IMAGES_PER_REQUEST),
        aspect_ratio,
        size,
        quality: None,
        output_format: "png".to_string(),
        seed: request.seed,
        background: None,
        provider_options: HashMap::new(),
    }
}

/// Generates `request.count` images with `provider`, the Bit's provider after
/// `resolve_generation_provider`. Hosted providers need `hosted`.
pub async fn generate(
    environment: ExecutionEnvironment,
    bit: &Bit,
    provider: &ModelProvider,
    request: &ImageRequest,
    hosted: Option<&HostedAccess>,
) -> flow_like_types::Result<Vec<AiImage>> {
    if request.prompt.trim().is_empty() {
        bail!("Image generation needs a prompt");
    }
    flow_like_catalog_media_support::ensure_vertex_credentials_explicit_in(environment, provider)?;
    let options = stable_diffusion_options(
        provider,
        stablediffusion::with_model_defaults(provider, ImageGenerationProviderOptions::default())?,
        request,
    );
    let service = service_request(provider, request);
    let generated = dispatch(environment, bit, provider, &service, &options, hosted).await?;
    if generated.is_empty() {
        bail!("The image provider returned no images");
    }
    Ok(generated
        .into_iter()
        .map(|image| ai_image(provider, image))
        .collect())
}

/// Hosted providers go through the API proxy, every other one straight to its provider.
pub(super) async fn dispatch(
    environment: ExecutionEnvironment,
    bit: &Bit,
    provider: &ModelProvider,
    request: &ImageGenerationRequest,
    options: &ImageGenerationProviderOptions,
    hosted: Option<&HostedAccess>,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    if flow_like_model_provider::provider::is_hosted_provider_name(&provider.provider_name) {
        let access = hosted.ok_or_else(|| {
            anyhow!("Hosted image generation needs an access token for the API proxy")
        })?;
        let images = hosted::generate(access, bit, provider, request, options).await?;
        capped::ensure_within_caps(images.iter().map(|image| image.bytes.len()))?;
        return Ok(images);
    }
    let images = generate_with_provider(environment, provider, request, options).await?;
    capped::ensure_within_caps(images.iter().map(|image| image.bytes.len()))?;
    Ok(images)
}

fn ai_image(provider: &ModelProvider, image: GeneratedImage) -> AiImage {
    AiImage {
        mime: image.mime_type.unwrap_or_else(|| "image/png".to_string()),
        metadata: json!({
            "ai_generated": true,
            "provider": provider.provider_name,
            "model": provider.model_id,
            "provider_metadata": image.provider_metadata,
        }),
        bytes: image.bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ModelProvider {
        ModelProvider {
            provider_name: name.into(),
            model_id: Some("gpt-image-1".into()),
            version: None,
            api_surface: None,
            params: None,
        }
    }

    #[test]
    fn a_request_maps_size_to_the_provider_fields() {
        let provider = provider(PROVIDER_TOGETHER);
        let request = service_request(
            &provider,
            &ImageRequest {
                prompt: "a fox".into(),
                negative_prompt: Some("  ".into()),
                count: 9,
                size: Some("1536x1024".into()),
                seed: Some(7),
            },
        );
        assert_eq!(request.count, MAX_IMAGES_PER_REQUEST);
        assert_eq!(request.size.as_deref(), Some("1536x1024"));
        assert_eq!(request.aspect_ratio.as_deref(), Some("3:2"));
        assert_eq!(request.negative_prompt, None);
        assert_eq!(request.seed, Some(7));
        let portrait = service_request(
            &provider,
            &ImageRequest {
                prompt: "a fox".into(),
                size: Some("1024x1536".into()),
                ..Default::default()
            },
        );
        assert_eq!(portrait.aspect_ratio.as_deref(), Some("2:3"));
        assert_eq!(portrait.count, 1);
    }

    #[test]
    fn generic_dimensions_follow_the_provider_request_contract() {
        let input = ImageRequest {
            prompt: "a landscape".into(),
            size: Some("1536x1024".into()),
            count: 1,
            ..Default::default()
        };
        for name in [PROVIDER_OPENAI, PROVIDER_OPENROUTER, "hosted:openrouter"] {
            let request = service_request(&provider(name), &input);
            assert_eq!(request.size.as_deref(), Some("1536x1024"), "{name}");
            assert_eq!(request.aspect_ratio, None, "{name}");
        }
        for name in [PROVIDER_GEMINI, PROVIDER_VERTEX] {
            let request = service_request(&provider(name), &input);
            assert_eq!(request.size, None, "{name}");
            assert_eq!(request.aspect_ratio.as_deref(), Some("4:3"), "{name}");
        }
        let xai = service_request(&provider(PROVIDER_XAI), &input);
        assert_eq!(xai.size, None);
        assert_eq!(xai.aspect_ratio.as_deref(), Some("3:2"));
        let options = ImageGenerationProviderOptions::OpenRouter(OpenRouterImageOptions {
            aspect_ratio: ImageAspectRatio::Landscape16x9,
            size: ImageSize::Landscape1536x1024,
        });
        let normalized = options.normalized();
        assert_eq!(normalized.aspect_ratio.as_deref(), Some("16:9"));
        assert_eq!(normalized.size.as_deref(), Some("1536x1024"));
    }

    #[test]
    fn generic_imagen_shapes_choose_an_accepted_ratio_with_the_same_orientation() {
        for (size, ratio) in [
            ("1024x1024", "1:1"),
            ("1536x1024", "4:3"),
            ("1024x1536", "3:4"),
            ("1920x1080", "16:9"),
            ("1080x1920", "9:16"),
            ("2048x1024", "16:9"),
            ("1024x2048", "9:16"),
        ] {
            assert_eq!(imagen_ratio_of(size).as_deref(), Some(ratio), "{size}");
        }
    }

    #[test]
    fn inferred_ratios_match_the_actual_dimensions() {
        for (size, ratio) in [
            ("1024x1024", "1:1"),
            ("1536x1024", "3:2"),
            ("1024x1536", "2:3"),
            ("1024x768", "4:3"),
            ("768x1024", "3:4"),
            ("1920x1080", "16:9"),
            ("1080x1920", "9:16"),
            ("1173x640", "1173:640"),
        ] {
            assert_eq!(aspect_ratio_of(size).as_deref(), Some(ratio), "{size}");
        }
        for size in ["0x0", "0x1024", "1024x0", "1024", "auto"] {
            assert_eq!(aspect_ratio_of(size), None, "{size}");
        }
    }

    #[test]
    fn generated_images_are_marked_as_ai_generated() {
        let provider = provider("custom:openai");
        let image = ai_image(
            &provider,
            GeneratedImage {
                bytes: vec![1, 2],
                mime_type: None,
                provider_metadata: Value::Null,
            },
        );
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.metadata["ai_generated"], true);
        assert_eq!(image.metadata["model"], "gpt-image-1");
    }

    #[test]
    fn a_provider_response_maps_to_marked_images() {
        let response = json!({"predictions": [
            {"bytesBase64Encoded": STANDARD.encode([1u8, 2, 3]), "mimeType": "image/jpeg"},
            {"bytesBase64Encoded": STANDARD.encode([4u8]), "mimeType": "image/png"},
        ]});
        let provider = provider("custom:gemini");
        let images: Vec<AiImage> = parse_google_predictions(&response)
            .unwrap()
            .into_iter()
            .map(|image| ai_image(&provider, image))
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].bytes, vec![1, 2, 3]);
        assert_eq!(images[0].mime, "image/jpeg");
        assert!(
            images
                .iter()
                .all(|image| image.metadata["ai_generated"] == true)
        );
        assert_eq!(images[1].metadata["provider"], "custom:gemini");
    }

    #[test]
    fn stable_diffusion_options_take_the_requested_size_and_seed() {
        let defaults = ImageGenerationProviderOptions::StableDiffusion(Default::default());
        let sd_provider = provider(flow_like_model_provider::stablediffusion::PROVIDER_NAME);
        let ImageGenerationProviderOptions::StableDiffusion(sd) = stable_diffusion_options(
            &sd_provider,
            defaults,
            &ImageRequest {
                prompt: "x".into(),
                size: Some("1024x1536".into()),
                seed: Some(5),
                negative_prompt: Some("blur".into()),
                count: 1,
            },
        ) else {
            panic!("stable diffusion options stay stable diffusion options");
        };
        assert_eq!((sd.width, sd.height, sd.seed), (1024, 1536, 5));
        assert_eq!(sd.negative_prompt.as_deref(), Some("blur"));
    }

    #[tokio::test]
    async fn an_empty_prompt_is_refused_before_any_provider_call() {
        let bit = Bit::default();
        let provider = provider("custom:openai");
        let error = generate(
            ExecutionEnvironment::Server,
            &bit,
            &provider,
            &ImageRequest::default(),
            None,
        )
        .await
        .expect_err("empty prompt");
        assert!(error.to_string().contains("prompt"));
    }
}
