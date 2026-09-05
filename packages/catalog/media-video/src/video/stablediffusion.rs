use super::*;
use flow_like_model_provider::stablediffusion::{
    GenerationMode, GenerationRequest, StableDiffusionConfig,
};

pub(super) use flow_like_model_provider::stablediffusion::PROVIDER_NAME;

// Names accepted by stable-diffusion.cpp at the pinned server revision.
const SAMPLERS: &[&str] = &[
    "auto",
    "euler",
    "euler_a",
    "heun",
    "dpm2",
    "dpm++2s_a",
    "dpm++2m",
    "dpm++2mv2",
    "ipndm",
    "ipndm_v",
    "lcm",
    "ddim_trailing",
    "tcd",
    "res_multistep",
    "res_2s",
    "er_sde",
    "euler_cfg_pp",
    "euler_a_cfg_pp",
    "euler_ge",
    "dpm++2m_sde",
    "dpm++2m_sde_bt",
    "lms",
];
const SCHEDULERS: &[&str] = &[
    "auto",
    "discrete",
    "normal",
    "karras",
    "exponential",
    "ays",
    "gits",
    "sgm_uniform",
    "simple",
    "smoothstep",
    "kl_optimal",
    "lcm",
    "bong_tangent",
    "ltx2",
    "logit_normal",
    "flux2",
    "flux",
    "beta",
];

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StableDiffusionVideoOutputFormat {
    #[default]
    Avi,
    Webp,
    Webm,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StableDiffusionVideoOptions {
    pub width: u32,
    pub height: u32,
    pub video_frames: u32,
    pub fps: u32,
    pub steps: u32,
    pub cfg_scale: f64,
    pub negative_prompt: Option<String>,
    /// Use -1 for a random seed. Zero is a deterministic seed.
    pub seed: i64,
    /// Omit to use the loaded model's default sampler.
    pub sampler: Option<String>,
    /// Omit to use the loaded model's default scheduler.
    pub scheduler: Option<String>,
    pub output_format: StableDiffusionVideoOutputFormat,
}

impl Default for StableDiffusionVideoOptions {
    fn default() -> Self {
        Self {
            width: 832,
            height: 480,
            video_frames: 33,
            fps: 16,
            steps: 28,
            cfg_scale: 7.0,
            negative_prompt: None,
            seed: -1,
            sampler: None,
            scheduler: None,
            output_format: StableDiffusionVideoOutputFormat::Avi,
        }
    }
}

impl StableDiffusionVideoOptions {
    pub(super) fn validate(&self) -> flow_like_types::Result<()> {
        for (name, value) in [("width", self.width), ("height", self.height)] {
            if value == 0 || value > i32::MAX as u32 || value % 8 != 0 {
                bail!(
                    "stable-diffusion.cpp video {name} must be a positive multiple of 8 within the 32-bit integer range"
                );
            }
        }
        if self.video_frames == 0
            || self.video_frames > i32::MAX as u32
            || (self.video_frames - 1) % 4 != 0
        {
            bail!(
                "stable-diffusion.cpp video_frames must be a positive 4n + 1 frame count, such as 33 or 81, within the 32-bit integer range"
            );
        }
        if self.fps == 0 || self.fps > i32::MAX as u32 {
            bail!("stable-diffusion.cpp video fps must be a positive 32-bit integer");
        }
        if !(1..=100).contains(&self.steps) {
            bail!("stable-diffusion.cpp video steps must be between 1 and 100");
        }
        if !self.cfg_scale.is_finite()
            || self.cfg_scale < 0.0
            || self.cfg_scale > f64::from(f32::MAX)
        {
            bail!(
                "stable-diffusion.cpp video CFG scale must be a finite non-negative 32-bit float"
            );
        }
        if self.seed < -1 {
            bail!(
                "stable-diffusion.cpp video seed must be -1 for random generation or a non-negative integer"
            );
        }
        for (name, value, accepted) in [
            ("sampler", &self.sampler, SAMPLERS),
            ("scheduler", &self.scheduler, SCHEDULERS),
        ] {
            if let Some(value) = value.clone().and_then(optional_clean)
                && !accepted.contains(&value.as_str())
            {
                bail!("Unknown stable-diffusion.cpp video {name}: {value}");
            }
        }
        Ok(())
    }
}

pub(super) fn normalize_options(
    options: &StableDiffusionVideoOptions,
) -> NormalizedVideoProviderOptions {
    NormalizedVideoProviderOptions {
        negative_prompt: options.negative_prompt.clone().and_then(optional_clean),
        seed: u64::try_from(options.seed).ok(),
        generate_audio: Some(false),
        provider_options: HashMap::from([("stablediffusion".to_string(), json!(options))]),
        ..Default::default()
    }
}

fn generation_request(req: &VideoGenerationRequest) -> flow_like_types::Result<GenerationRequest> {
    if req.last_frame.is_some() {
        bail!(
            "This stable-diffusion.cpp video integration does not accept Last Frame. Disconnect Last Frame and use First Frame for image-to-video generation."
        );
    }
    if req.input_video.is_some() {
        bail!(
            "This stable-diffusion.cpp video integration does not accept Input Video. Disconnect Input Video and use a prompt or First Frame."
        );
    }
    if req.generate_audio == Some(true) {
        bail!(
            "stable-diffusion.cpp video generation does not generate audio. Disable Generate Audio and add audio after generation."
        );
    }
    if req.count != 1 {
        bail!(
            "stable-diffusion.cpp generates one video per request. Run the node again to generate another clip."
        );
    }
    if req.aspect_ratio.is_some() || req.size.is_some() || req.duration_seconds.is_some() {
        bail!("Use stable-diffusion.cpp Video Options to set width, height, frame count, and FPS.");
    }
    let options: StableDiffusionVideoOptions = req
        .provider_options
        .get("stablediffusion")
        .map(|value| from_value(value.clone()))
        .transpose()?
        .unwrap_or_default();
    options.validate()?;
    let mut params = json!({
        "prompt": req.prompt,
        "negative_prompt": req.negative_prompt.clone().unwrap_or_default(),
        "width": options.width,
        "height": options.height,
        "video_frames": options.video_frames,
        "fps": options.fps,
        "seed": options.seed,
        "output_format": options.output_format,
        "sample_params": {
            "sample_steps": options.steps,
            "guidance": {"txt_cfg": options.cfg_scale}
        }
    });
    if let Some(sampler) = options.sampler.and_then(optional_clean) {
        params["sample_params"]["sample_method"] = json!(sampler);
    }
    if let Some(scheduler) = options.scheduler.and_then(optional_clean) {
        params["sample_params"]["scheduler"] = json!(scheduler);
    }
    if let Some(first_frame) = &req.first_frame {
        if !matches!(
            first_frame.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        ) {
            bail!("stable-diffusion.cpp First Frame must be a PNG, JPEG, or WebP image");
        }
        params["init_image"] = json!(media_data_uri(first_frame));
    }
    Ok(GenerationRequest {
        mode: GenerationMode::Video,
        params,
    })
}

pub(super) async fn generate_video(
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let request = generation_request(req)?;
    let config = provider
        .params
        .as_ref()
        .and_then(|params| params.get("stablediffusion"))
        .ok_or_else(|| anyhow!("Missing stable-diffusion.cpp configuration. Connect a stable-diffusion.cpp Video Model provider."))?;
    let config: StableDiffusionConfig = from_value(config.clone())?;
    let assets = flow_like_model_provider::stablediffusion::generate(&config, &request).await?;
    Ok(assets
        .into_iter()
        .map(|asset| GeneratedVideo {
            bytes: asset.bytes,
            mime_type: Some(asset.mime_type),
            provider_metadata: asset.metadata,
        })
        .collect())
}

fn build_stablediffusion_provider_bit(
    config: &StableDiffusionConfig,
) -> flow_like_types::Result<Bit> {
    config.validate()?;
    Ok(build_provider_bit(
        PROVIDER_NAME,
        config
            .model_path
            .clone()
            .or_else(|| config.diffusion_model_path.clone())
            .or_else(|| config.endpoint.clone()),
        None,
        HashMap::from([("stablediffusion".to_string(), to_value(config)?)]),
    ))
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildStableDiffusionVideoProviderNode {}

impl BuildStableDiffusionVideoProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildStableDiffusionVideoProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_build_stablediffusion",
            "stable-diffusion.cpp Video Model",
            "Configures a local video model or an existing stable-diffusion.cpp server.",
            "AI/Generative/Video/Provider",
        );
        node.set_flowscript_name("ai.video.provider", "stableDiffusion");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());
        add_exec_input(&mut node);
        node.add_input_pin(
            "config",
            "Configuration",
            "Set a local model path and optional companion models, or an existing server endpoint. Paths refer to the machine executing the flow.",
            VariableType::Struct,
        )
        .set_schema::<StableDiffusionConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(StableDiffusionConfig::default())));
        add_provider_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let config: StableDiffusionConfig = context.evaluate_pin("config").await?;
        let bit = build_stablediffusion_provider_bit(&config)?;
        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeStableDiffusionVideoOptionsNode {}

impl MakeStableDiffusionVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeStableDiffusionVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_stablediffusion",
            "stable-diffusion.cpp Video Options",
            "Sets local video dimensions, sampling settings, frame count, and container format.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "stableDiffusion");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());
        for (name, label, description, value) in [
            (
                "width",
                "Width",
                "Frame width in pixels, a positive multiple of 8",
                832,
            ),
            (
                "height",
                "Height",
                "Frame height in pixels, a positive multiple of 8",
                480,
            ),
            (
                "video_frames",
                "Frames",
                "Frame count must be 4n + 1, for example 33 or 81",
                33,
            ),
            ("fps", "FPS", "Playback frames per second", 16),
            (
                "steps",
                "Steps",
                "Number of sampling steps, between 1 and 100",
                28,
            ),
        ] {
            node.add_input_pin(name, label, description, VariableType::Integer)
                .set_default_value(Some(json!(value)));
        }
        node.add_input_pin(
            "cfg_scale",
            "CFG Scale",
            "Text guidance scale, a finite non-negative 32-bit float",
            VariableType::Float,
        )
        .set_default_value(Some(json!(7.0)));
        add_negative_prompt_pin(&mut node);
        node.add_input_pin(
            "seed",
            "Seed",
            "Use -1 for random generation. Zero is a deterministic seed.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        for (name, label, values) in [
            ("sampler", "Sampler", SAMPLERS),
            ("scheduler", "Scheduler", SCHEDULERS),
        ] {
            add_select_pin(
                &mut node,
                name,
                label,
                "Use auto to keep the loaded model's default",
                values,
                "auto",
            );
        }
        add_select_pin(
            &mut node,
            "output_format",
            "Output Format",
            "AVI is built in. Animated WebP and WebM require support in the server build.",
            &["avi", "webp", "webm"],
            "avi",
        );
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format: String = context.evaluate_pin("output_format").await?;
        let output_format = match output_format.trim() {
            "avi" => StableDiffusionVideoOutputFormat::Avi,
            "webp" => StableDiffusionVideoOutputFormat::Webp,
            "webm" => StableDiffusionVideoOutputFormat::Webm,
            _ => bail!("stable-diffusion.cpp video output format must be avi, webp, or webm"),
        };
        let options = StableDiffusionVideoOptions {
            width: context.evaluate_pin("width").await?,
            height: context.evaluate_pin("height").await?,
            video_frames: context.evaluate_pin("video_frames").await?,
            fps: context.evaluate_pin("fps").await?,
            steps: context.evaluate_pin("steps").await?,
            cfg_scale: context.evaluate_pin("cfg_scale").await?,
            negative_prompt: optional_clean(context.evaluate_pin("negative_prompt").await?),
            seed: context.evaluate_pin("seed").await?,
            sampler: optional_clean(context.evaluate_pin("sampler").await?),
            scheduler: optional_clean(context.evaluate_pin("scheduler").await?),
            output_format,
        };
        options.validate()?;
        context
            .set_pin_value(
                "options",
                json!(VideoGenerationProviderOptions::StableDiffusion(options)),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(options: StableDiffusionVideoOptions) -> VideoGenerationRequest {
        let normalized = normalize_options(&options);
        VideoGenerationRequest {
            prompt: "Waves breaking on a beach".to_string(),
            negative_prompt: normalized.negative_prompt,
            first_frame: None,
            last_frame: None,
            input_video: None,
            aspect_ratio: normalized.aspect_ratio,
            size: normalized.size,
            duration_seconds: normalized.duration_seconds,
            seed: normalized.seed,
            generate_audio: normalized.generate_audio,
            count: normalized.count,
            provider_options: normalized.provider_options,
            poll_interval_seconds: normalized.poll_interval_seconds,
            max_wait_seconds: normalized.max_wait_seconds,
        }
    }

    fn frame() -> MediaInput {
        MediaInput {
            bytes: vec![1, 2, 3],
            file_name: "frame.png".into(),
            mime_type: "image/png".into(),
        }
    }

    #[test]
    fn sampling_boundaries_match_native_server_limits() {
        for steps in [1, 100] {
            StableDiffusionVideoOptions {
                steps,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
        for steps in [0, 101, u32::MAX] {
            assert!(
                StableDiffusionVideoOptions {
                    steps,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for cfg_scale in [0.0, f64::from(f32::MAX)] {
            StableDiffusionVideoOptions {
                cfg_scale,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
        for cfg_scale in [
            f64::from(f32::MAX) * 2.0,
            f64::MAX,
            f64::INFINITY,
            f64::NAN,
            -1.0,
        ] {
            assert!(
                StableDiffusionVideoOptions {
                    cfg_scale,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn maps_native_video_parameters_and_first_frame() {
        let mut req = request(StableDiffusionVideoOptions {
            negative_prompt: Some("blur".into()),
            seed: 0,
            sampler: Some("euler".into()),
            scheduler: Some("discrete".into()),
            ..Default::default()
        });
        req.first_frame = Some(frame());
        let generated = generation_request(&req).unwrap();
        assert!(matches!(generated.mode, GenerationMode::Video));
        assert_eq!(generated.params["prompt"], "Waves breaking on a beach");
        assert_eq!(generated.params["negative_prompt"], "blur");
        assert_eq!(generated.params["video_frames"], 33);
        assert_eq!(generated.params["fps"], 16);
        assert_eq!(generated.params["seed"], 0);
        assert_eq!(generated.params["sample_params"]["sample_steps"], 28);
        assert_eq!(
            generated.params["sample_params"]["guidance"]["txt_cfg"],
            7.0
        );
        assert_eq!(generated.params["sample_params"]["sample_method"], "euler");
        assert_eq!(generated.params["sample_params"]["scheduler"], "discrete");
        assert_eq!(generated.params["init_image"], "data:image/png;base64,AQID");
        assert_eq!(generated.params["output_format"], "avi");
        assert!(generated.params.get("batch_count").is_none());
    }

    #[test]
    fn defaults_preserve_model_sampling_defaults_and_disable_audio() {
        let normalized = VideoGenerationProviderOptions::Default
            .normalized_for_provider(PROVIDER_NAME)
            .unwrap();
        assert_eq!(normalized.generate_audio, Some(false));
        let generated =
            generation_request(&request(StableDiffusionVideoOptions::default())).unwrap();
        assert_eq!(generated.params["seed"], -1);
        assert!(
            generated.params["sample_params"]
                .get("sample_method")
                .is_none()
        );
        assert!(generated.params["sample_params"].get("scheduler").is_none());
    }

    #[test]
    fn rejects_incompatible_provider_options() {
        assert!(
            VideoGenerationProviderOptions::OpenAiSora(Default::default())
                .normalized_for_provider(PROVIDER_NAME)
                .is_err()
        );
        assert!(
            VideoGenerationProviderOptions::StableDiffusion(Default::default())
                .normalized_for_provider(PROVIDER_OPENAI)
                .is_err()
        );
        assert!(
            VideoGenerationProviderOptions::OpenAiSora(Default::default())
                .normalized_for_provider(PROVIDER_OPENAI)
                .is_ok()
        );
    }

    #[test]
    fn rejects_unsupported_inputs_before_generation() {
        let mut req = request(StableDiffusionVideoOptions::default());
        req.last_frame = Some(frame());
        assert!(
            generation_request(&req)
                .unwrap_err()
                .to_string()
                .contains("Last Frame")
        );
        req.last_frame = None;
        req.input_video = Some(frame());
        assert!(
            generation_request(&req)
                .unwrap_err()
                .to_string()
                .contains("Input Video")
        );
        req.input_video = None;
        req.generate_audio = Some(true);
        assert!(
            generation_request(&req)
                .unwrap_err()
                .to_string()
                .contains("audio")
        );
    }

    #[test]
    fn rejects_frame_count_truncation_and_invalid_sampling() {
        for video_frames in [0, 32, 34, u32::MAX] {
            let options = StableDiffusionVideoOptions {
                video_frames,
                ..Default::default()
            };
            assert!(options.validate().is_err());
        }
        for cfg_scale in [f64::NAN, f64::INFINITY, -1.0] {
            let options = StableDiffusionVideoOptions {
                cfg_scale,
                ..Default::default()
            };
            assert!(options.validate().is_err());
        }
        assert!(
            StableDiffusionVideoOptions {
                sampler: Some("eulre".into()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            StableDiffusionVideoOptions {
                scheduler: Some("karrass".into()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            from_value::<StableDiffusionVideoOptions>(json!({"generate_audio": true})).is_err()
        );
    }

    #[test]
    fn provider_bit_preserves_typed_configuration() {
        let config: StableDiffusionConfig =
            from_value(json!({"endpoint": "http://127.0.0.1:1234"})).unwrap();
        let bit = build_stablediffusion_provider_bit(&config).unwrap();
        assert_eq!(bit.bit_type, BitTypes::VideoGeneration);
        let provider = provider_from_bit(&bit).unwrap();
        assert_eq!(provider.provider_name, PROVIDER_NAME);
        assert_eq!(
            provider.params.unwrap()["stablediffusion"],
            to_value(config).unwrap()
        );
    }

    #[test]
    fn output_paths_use_container_extensions() {
        assert_eq!(extension_from_mime(Some("video/x-msvideo")), "avi");
        assert_eq!(extension_from_mime(Some("image/webp")), "webp");
        assert_eq!(extension_from_mime(Some("video/webm")), "webm");
        assert_eq!(extension_from_mime(Some("video/mp4")), "mp4");
    }
}
