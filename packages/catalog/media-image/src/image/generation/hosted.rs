use super::*;
use flow_like_model_provider::image_generation::{HostedImageClient, HostedImageRequest};

fn request_for(
    bit: &Bit,
    provider: &ModelProvider,
    request: &ImageGenerationRequest,
    options: &ImageGenerationProviderOptions,
) -> flow_like_types::Result<HostedImageRequest> {
    if bit.bit_type != BitTypes::ImageGeneration {
        bail!("Hosted image generation requires an ImageGeneration catalog Bit");
    }
    if !matches!(
        provider.provider_name.trim().to_ascii_lowercase().as_str(),
        "hosted:cloudflare" | "hosted:openrouter"
    ) {
        bail!("This provider does not support hosted image generation");
    }
    if matches!(options, ImageGenerationProviderOptions::StableDiffusion(_)) {
        bail!("stable-diffusion.cpp image options require a stable-diffusion.cpp provider");
    }
    if request.count != 1 || !request.provider_options.is_empty() {
        bail!("Hosted image generation accepts one image and typed image options only");
    }
    let request = HostedImageRequest {
        model: bit.id.clone(),
        prompt: request.prompt.clone(),
        negative_prompt: request.negative_prompt.clone(),
        aspect_ratio: request.aspect_ratio.clone(),
        size: request.size.clone(),
        quality: request.quality.clone(),
        // Keep the provider default when the node has no explicit format option.
        output_format: options.normalized().output_format,
        seed: request.seed,
        background: request.background.clone(),
    };
    request.validate()?;
    Ok(request)
}

fn bearer_token<'a>(scoped: Option<&'a str>, run: Option<&'a str>) -> Option<&'a str> {
    [scoped, run]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|token| !token.is_empty())
}

/// What a caller outside a flow run supplies for a hosted image call: the API proxy's address,
/// the credential that reaches it and the usage labels.
#[derive(Clone, Default)]
pub struct HostedAccess {
    pub token: Option<String>,
    pub authorizer: Option<std::sync::Arc<dyn flow_like_types::authorization::RequestAuthorizer>>,
    pub api_base_url: Option<String>,
    pub app_id: Option<String>,
    pub run_id: Option<String>,
}

pub(super) fn access_from(context: &ExecutionContext) -> HostedAccess {
    let state = &context.app_state;
    let usage = context.model_usage_context();
    HostedAccess {
        token: bearer_token(
            state.hosted_model_token.as_deref(),
            context.token.as_deref(),
        )
        .map(ToOwned::to_owned),
        authorizer: state.request_authorizer.clone(),
        api_base_url: usage.as_ref().and_then(|usage| usage.api_base_url.clone()),
        app_id: usage.as_ref().and_then(|usage| usage.app_id.clone()),
        run_id: usage.as_ref().and_then(|usage| usage.run_id.clone()),
    }
}

pub(super) async fn generate(
    access: &HostedAccess,
    bit: &Bit,
    provider: &ModelProvider,
    request: &ImageGenerationRequest,
    options: &ImageGenerationProviderOptions,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let request = request_for(bit, provider, request, options)?;
    let authorizer = access.authorizer.clone();
    let token = bearer_token(None, access.token.as_deref());
    if token.is_none() && authorizer.is_none() {
        bail!("Hosted image generation requires an access token for the API proxy");
    }
    let base = access
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(flow_like_model_provider::embedding::proxy_config::api_base_url);
    let headers: Vec<(String, String)> = [
        ("x-flow-like-app-id", access.app_id.as_deref()),
        ("x-flow-like-run-id", access.run_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| (name.to_string(), value.to_owned()))
    })
    .collect();
    // Only the runtime and credential broker select the destination of this token.
    let mut client =
        HostedImageClient::new(&base, token.unwrap_or_default())?.with_usage_headers(headers);
    if let Some(authorizer) = authorizer {
        client = client.with_authorizer(authorizer)?;
    }
    let response = client.generate(&request).await?;
    response
        .data
        .into_iter()
        .map(|image| {
            Ok(GeneratedImage {
                bytes: image.decode()?,
                mime_type: Some(image.mime_type),
                provider_metadata: json!({"model": response.model}),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bit() -> Bit {
        Bit {
            id: "official-image-bit".into(),
            bit_type: BitTypes::ImageGeneration,
            parameters: json!({"provider": {
                "provider_name": "hosted:openrouter", "model_id": "upstream-model",
                "params": {"endpoint": "https://untrusted.example", "api_key": "not-used"}
            }}),
            ..Default::default()
        }
    }

    fn request() -> ImageGenerationRequest {
        ImageGenerationRequest {
            prompt: "A lighthouse".into(),
            system_prompt: None,
            negative_prompt: None,
            count: 1,
            aspect_ratio: None,
            size: None,
            quality: None,
            output_format: "png".into(),
            seed: None,
            background: None,
            provider_options: HashMap::new(),
        }
    }

    #[test]
    fn forwards_catalog_id_and_typed_options_without_provider_secrets() {
        let bit = bit();
        let provider = provider_from_bit(&bit).unwrap();
        let mut input = request();
        input.aspect_ratio = Some("16:9".into());
        input.seed = Some(0);
        let request = request_for(
            &bit,
            &provider,
            &input,
            &ImageGenerationProviderOptions::Default,
        )
        .unwrap();
        let body = json!(request);
        assert_eq!(body["model"], "official-image-bit");
        assert_eq!(body["aspect_ratio"], "16:9");
        assert_eq!(body["seed"], 0);
        assert!(body.get("output_format").is_none());
        assert!(body.get("endpoint").is_none());
        assert!(body.get("api_key").is_none());
        assert!(body.get("provider").is_none());
    }

    #[test]
    fn rejects_chat_bits_other_hosted_providers_and_unmetered_options() {
        let mut bit = bit();
        let mut provider = provider_from_bit(&bit).unwrap();
        bit.bit_type = BitTypes::Llm;
        assert!(
            request_for(
                &bit,
                &provider,
                &request(),
                &ImageGenerationProviderOptions::Default
            )
            .is_err()
        );
        bit.bit_type = BitTypes::ImageGeneration;
        provider.provider_name = "hosted:openai".into();
        assert!(
            request_for(
                &bit,
                &provider,
                &request(),
                &ImageGenerationProviderOptions::Default
            )
            .is_err()
        );
        provider.provider_name = "hosted:cloudflare".into();
        let mut input = request();
        input.count = 2;
        assert!(
            request_for(
                &bit,
                &provider,
                &input,
                &ImageGenerationProviderOptions::Default
            )
            .is_err()
        );
        input.count = 1;
        input.provider_options.insert("n".into(), json!(10));
        assert!(
            request_for(
                &bit,
                &provider,
                &input,
                &ImageGenerationProviderOptions::Default
            )
            .is_err()
        );
        assert!(
            request_for(
                &bit,
                &provider,
                &request(),
                &ImageGenerationProviderOptions::StableDiffusion(Default::default())
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_format_and_scoped_token_take_precedence() {
        let bit = bit();
        let provider = provider_from_bit(&bit).unwrap();
        let options = ImageGenerationProviderOptions::OpenAi(OpenAiImageOptions {
            output_format: ImageOutputFormat::Webp,
            ..Default::default()
        });
        assert_eq!(
            request_for(&bit, &provider, &request(), &options)
                .unwrap()
                .output_format
                .as_deref(),
            Some("webp")
        );
        assert_eq!(bearer_token(Some(" scoped "), Some("run")), Some("scoped"));
        assert_eq!(bearer_token(Some(" "), Some("run")), Some("run"));
        assert_eq!(bearer_token(None, None), None);
    }
}
