#![cfg(feature = "execute")]

use std::sync::{Arc, Mutex as StdMutex, Weak};

use flow_like_catalog_core::FlowPath;
use flow_like_catalog_embedding::{CachedEmbeddingModel, CachedEmbeddingModelObject};
use flow_like_catalog_media_video::video::utils::embed_media::{EmbedAudioNode, EmbedVideoNode};
use flow_like_catalog_media_video::video::utils::prepare_embedding_media::PrepareEmbeddingMediaNode;
use flow_like_model_provider::embedding::interface::*;
use flow_like_runtime::{
    bit::BitTypes,
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin,
        },
        node::NodeLogic,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path,
    files::store::FlowLikeStore,
    object_store::{ObjectStoreExt, memory::InMemory},
};
use flow_like_types::{
    Bytes, Value, async_trait,
    json::{from_value, json},
    sync::{Mutex, RwLock},
};

const WAV: &[u8] = include_bytes!("fixtures/embedding-tone.wav");
const VIDEO: &[u8] = include_bytes!("fixtures/embedding-lowdelay.mp4");
const VIDEO_AUDIO: &[u8] = include_bytes!("fixtures/embedding-offset-audio.mp4");
const MARKDOWN: &[u8] = b"# Bird songs\n\nA recording beside a stream.\n";

fn png_fixture() -> Vec<u8> {
    let image = flow_like_types::image::DynamicImage::new_rgb8(7, 5);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, flow_like_types::image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[derive(Clone, Copy, Default)]
enum Reply {
    #[default]
    Valid,
    Failure,
    ExtraVector,
    NonFinite,
    WrongDimensions,
}

struct RecordingModel {
    descriptor: EmbeddingDescriptor,
    calls: StdMutex<Vec<EmbeddingRequest>>,
    reply: Reply,
    encoded: bool,
}

impl RecordingModel {
    fn new(modalities: Vec<EmbeddingModality>) -> Self {
        Self {
            descriptor: EmbeddingDescriptor {
                model_id: "fixture-model".into(),
                adapter: "fixture".into(),
                modalities,
                joint_combinations: vec![vec![EmbeddingModality::Audio, EmbeddingModality::Video]],
                purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
                space: EmbeddingSpace {
                    id: "fixture-space".into(),
                    dimensions: 3,
                    normalized: true,
                    metric: EmbeddingMetric::Cosine,
                },
                supported_dimensions: vec![2, 3],
                limits: EmbeddingLimits::default(),
                pipeline_fingerprint: "fixture-pipeline".into(),
            },
            calls: StdMutex::new(Vec::new()),
            reply: Reply::Valid,
            encoded: false,
        }
    }

    fn calls(&self) -> Vec<EmbeddingRequest> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl EmbeddingModel for RecordingModel {
    fn supports_encoded_media(&self) -> bool {
        self.encoded
    }

    fn descriptor(&self) -> &EmbeddingDescriptor {
        &self.descriptor
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingBatch, EmbeddingError> {
        self.descriptor.validate(&request)?;
        self.calls.lock().unwrap().push(request.clone());
        if matches!(self.reply, Reply::Failure) {
            return Err(EmbeddingError::Backend(flow_like_types::anyhow!(
                "fixture backend failure"
            )));
        }
        let mut result = self
            .descriptor
            .finish(&request, vec![vec![0.6, 0.8, 0.0]; request.items.len()])?;
        match self.reply {
            Reply::ExtraVector => result.embeddings.push(result.embeddings[0].clone()),
            Reply::NonFinite => result.embeddings[0][0] = f32::NAN,
            Reply::WrongDimensions => {
                result.embeddings[0].pop();
            }
            _ => {}
        }
        Ok(result)
    }
}

async fn context(
    logic: Arc<dyn NodeLogic>,
    model: Option<Arc<dyn EmbeddingModel>>,
    name: &str,
    bytes: Option<&[u8]>,
) -> ExecutionContext {
    let node = logic.get_node();
    let pins = node
        .pins
        .values()
        .map(|pin| {
            (
                pin.id.clone(),
                pin.name.clone(),
                Arc::new(InternalPin::new(pin, false)),
            )
        })
        .collect::<Vec<_>>();
    let current = Arc::new(InternalNode::new(
        node,
        pins.iter()
            .map(|(id, _, pin)| (id.clone(), pin.clone()))
            .collect(),
        logic,
        pins.iter()
            .map(|(_, name, pin)| (name.clone(), vec![pin.clone()]))
            .collect(),
    ));
    for pin in current.pins.iter() {
        pin.init_node(Arc::downgrade(&current));
        pin.init_connected_to(Vec::new());
        pin.init_depends_on(Vec::new());
    }
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let variables = Arc::new(Mutex::new(Default::default()));
    let cache = Arc::new(RwLock::new(Default::default()));
    let run: Weak<Mutex<Run>> = Weak::new();
    let mut context = ExecutionContext::new(
        Arc::new(
            [(current.node_id().to_string(), current.clone())]
                .into_iter()
                .collect(),
        ),
        &run,
        &state,
        &current,
        &variables,
        &cache,
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(Default::default()),
        None,
    )
    .await;
    let store = Arc::new(InMemory::new());
    if let Some(bytes) = bytes {
        store
            .put(&Path::from(name), Bytes::copy_from_slice(bytes).into())
            .await
            .unwrap();
    }
    context
        .set_cache("media-store", Arc::new(FlowLikeStore::Memory(store)))
        .await;
    if let Some(model) = model {
        context
            .set_cache(
                "fixture-model",
                Arc::new(CachedEmbeddingModelObject {
                    text_model: None,
                    image_model: None,
                    model: Some(model),
                }),
            )
            .await;
    }
    if context.node.get_pin_by_name("model").await.is_ok() {
        context
            .set_pin_value(
                "model",
                json!(CachedEmbeddingModel {
                    cache_key: "fixture-model".into(),
                    model_type: BitTypes::Embedding,
                }),
            )
            .await
            .unwrap();
    }
    context
        .set_pin_value(
            "source",
            json!(FlowPath::new(name.into(), "media-store".into(), None)),
        )
        .await
        .unwrap();
    context
}

async fn output(context: &ExecutionContext, name: &str) -> Option<Value> {
    context
        .node
        .get_pin_by_name(name)
        .await
        .unwrap()
        .get_raw_value()
        .await
}

async fn assert_success(context: &ExecutionContext, dimensions: usize) -> EmbeddingBatch {
    assert_eq!(output(context, "exec_out").await, Some(json!(true)));
    let vector: Vec<f32> = from_value(output(context, "vector").await.unwrap()).unwrap();
    let result: EmbeddingBatch = from_value(output(context, "result").await.unwrap()).unwrap();
    assert_eq!(result.embeddings, vec![vector.clone()]);
    assert_eq!(result.space.dimensions, dimensions);
    assert_eq!(vector.len(), dimensions);
    assert!(vector.iter().all(|value| value.is_finite()));
    let norm = vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
    result
}

async fn assert_failed(context: &ExecutionContext) {
    assert_eq!(output(context, "exec_out").await, Some(json!(false)));
    assert!(output(context, "vector").await.is_none());
    assert!(output(context, "result").await.is_none());
}

#[tokio::test]
async fn hosted_media_nodes_preserve_original_files() {
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
    for (name, bytes, modality) in [
        ("tone.wav", WAV, EmbeddingModality::Audio),
        ("video.mp4", VIDEO, EmbeddingModality::Video),
    ] {
        let logic = Arc::new(EmbedVideoNode::new());
        let mut recording = RecordingModel::new(vec![modality]);
        recording.encoded = true;
        let model = Arc::new(recording);
        let mut context = context(logic.clone(), Some(model.clone()), name, Some(bytes)).await;
        logic.run(&mut context).await.unwrap();
        assert_success(&context, 3).await;
        let calls = model.calls();
        let [
            EmbeddingPart::EncodedMedia {
                modality: actual,
                source,
            },
        ] = calls[0].items[0].parts.as_slice()
        else {
            panic!("hosted models need the original encoded file");
        };
        assert_eq!(*actual, modality);
        assert_eq!(STANDARD.decode(source).unwrap(), bytes);
    }
}

#[tokio::test]
async fn hosted_video_keeps_original_file_and_aligned_soundtrack() {
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};

    let logic = Arc::new(EmbedVideoNode::new());
    let mut recording =
        RecordingModel::new(vec![EmbeddingModality::Audio, EmbeddingModality::Video]);
    recording.encoded = true;
    let model = Arc::new(recording);
    let mut context = context(
        logic.clone(),
        Some(model.clone()),
        "offset.mp4",
        Some(VIDEO_AUDIO),
    )
    .await;
    context
        .set_pin_value("include_audio", json!(true))
        .await
        .unwrap();
    logic.run(&mut context).await.unwrap();
    let result = assert_success(&context, 3).await;
    let calls = model.calls();
    let [
        EmbeddingPart::EncodedMedia { modality, source },
        EmbeddingPart::Audio(audio),
    ] = calls[0].items[0].parts.as_slice()
    else {
        panic!("expected original video and a separate soundtrack");
    };
    assert_eq!(*modality, EmbeddingModality::Video);
    assert_eq!(STANDARD.decode(source).unwrap(), VIDEO_AUDIO);
    assert_eq!(
        (audio.sample_rate, audio.channels, audio.samples.len()),
        (16_000, 1, 14_400)
    );
    assert!(audio.samples[..2176].iter().all(|sample| *sample == 0.0));
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.05));
    assert_eq!(result.usage.audio_duration_ms, 900);
    assert_eq!(result.usage.video_frames, 0);
}

#[tokio::test]
async fn prepare_media_checks_frame_limits_before_reading_encoded_or_decoded_video() {
    let logic = Arc::new(PrepareEmbeddingMediaNode::new());
    for encoded in [false, true] {
        for max_frames in [-1, 0, 1025] {
            let mut context = context(logic.clone(), None, "missing.mp4", None).await;
            context.set_pin_value("kind", json!("video")).await.unwrap();
            context
                .set_pin_value("encoded", json!(encoded))
                .await
                .unwrap();
            context
                .set_pin_value("max_frames", json!(max_frames))
                .await
                .unwrap();
            let error = logic.run(&mut context).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Max Frames must be between 1 and 1024")
            );
            assert_eq!(output(&context, "exec_out").await, Some(json!(false)));
            assert!(output(&context, "content").await.is_none());
        }
    }
}

#[tokio::test]
async fn audio_node_resolves_cached_model_and_embeds_decoded_wav() {
    let logic = Arc::new(EmbedAudioNode::new());
    let model = Arc::new(RecordingModel::new(vec![EmbeddingModality::Audio]));
    let mut context = context(logic.clone(), Some(model.clone()), "tone.wav", Some(WAV)).await;
    context
        .set_pin_value(
            "options",
            json!(EmbeddingOptions {
                dimensions: Some(2)
            }),
        )
        .await
        .unwrap();
    logic.run(&mut context).await.unwrap();
    let result = assert_success(&context, 2).await;
    assert_eq!(result.space.id, "fixture-space");
    assert_eq!(result.provenance.pipeline_fingerprint, "fixture-pipeline");
    assert_eq!(result.usage.audio_duration_ms, 1000);
    let calls = model.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].purpose, EmbeddingPurpose::Document);
    assert_eq!(calls[0].options.dimensions, Some(2));
    let [EmbeddingPart::Audio(audio)] = calls[0].items[0].parts.as_slice() else {
        panic!("expected decoded audio")
    };
    assert_eq!(
        (audio.sample_rate, audio.channels, audio.samples.len()),
        (16_000, 1, 16_000)
    );
    let rms = (audio
        .samples
        .iter()
        .map(|sample| sample * sample)
        .sum::<f32>()
        / audio.samples.len() as f32)
        .sqrt();
    assert!((0.08..0.10).contains(&rms));
    assert_eq!(
        context.cache.read().await.len(),
        2,
        "direct media must not create image cache entries"
    );
}

#[tokio::test]
async fn video_node_keeps_frame_times_and_forwards_task_options() {
    for max_frames in [None, Some(3)] {
        let logic = Arc::new(EmbedVideoNode::new());
        let model = Arc::new(RecordingModel::new(vec![EmbeddingModality::Video]));
        let mut context =
            context(logic.clone(), Some(model.clone()), "video.mp4", Some(VIDEO)).await;
        context
            .set_pin_value("purpose", json!(EmbeddingPurpose::Query))
            .await
            .unwrap();
        if let Some(max_frames) = max_frames {
            context
                .set_pin_value("max_frames", json!(max_frames))
                .await
                .unwrap();
        }
        logic.run(&mut context).await.unwrap();
        let result = assert_success(&context, 3).await;
        let calls = model.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].purpose, EmbeddingPurpose::Query);
        assert_eq!(calls[0].options.dimensions, None);
        let [EmbeddingPart::Video(video)] = calls[0].items[0].parts.as_slice() else {
            panic!("expected decoded video")
        };
        assert_eq!(video.duration_ms, 1000);
        assert!(video.audio.is_none());
        let expected = if max_frames.is_some() {
            vec![0, 400, 800]
        } else {
            vec![0, 200, 400, 600, 800]
        };
        assert_eq!(
            video
                .frames
                .iter()
                .map(|frame| frame.timestamp_ms)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            video
                .frames
                .iter()
                .all(|frame| frame.image.width() == 64 && frame.image.height() == 48)
        );
        assert_ne!(
            video.frames[0].image.as_bytes(),
            video.frames.last().unwrap().image.as_bytes()
        );
        assert_eq!(result.usage.video_frames, video.frames.len());
        assert_eq!(context.cache.read().await.len(), 2);
    }
}

#[tokio::test]
async fn video_node_includes_audio_on_its_original_timeline() {
    let logic = Arc::new(EmbedVideoNode::new());
    let model = Arc::new(RecordingModel::new(vec![
        EmbeddingModality::Audio,
        EmbeddingModality::Video,
    ]));
    let mut context = context(
        logic.clone(),
        Some(model.clone()),
        "offset.mp4",
        Some(VIDEO_AUDIO),
    )
    .await;
    context
        .set_pin_value("include_audio", json!(true))
        .await
        .unwrap();
    context.set_pin_value("max_frames", json!(3)).await.unwrap();
    logic.run(&mut context).await.unwrap();
    let result = assert_success(&context, 3).await;
    let calls = model.calls();
    let [EmbeddingPart::Video(video)] = calls[0].items[0].parts.as_slice() else {
        panic!("expected decoded video")
    };
    let audio = video.audio.as_ref().unwrap();
    assert_eq!(
        (audio.sample_rate, audio.channels, audio.samples.len()),
        (16_000, 1, 14_400)
    );
    assert!(audio.samples[..2176].iter().all(|sample| *sample == 0.0));
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.05));
    assert_eq!(result.usage.audio_duration_ms, 900);
    assert_eq!(result.usage.video_frames, 3);
}

#[tokio::test]
async fn both_nodes_dispatch_image_and_markdown_extensions_case_insensitively() {
    let png = png_fixture();
    for video_node in [false, true] {
        for (name, bytes, modality) in [
            ("picture.PnG", png.as_slice(), EmbeddingModality::Image),
            ("notes.MD", MARKDOWN, EmbeddingModality::Text),
        ] {
            let logic: Arc<dyn NodeLogic> = if video_node {
                Arc::new(EmbedVideoNode::new())
            } else {
                Arc::new(EmbedAudioNode::new())
            };
            let model = Arc::new(RecordingModel::new(vec![modality]));
            let mut context = context(logic.clone(), Some(model.clone()), name, Some(bytes)).await;
            if video_node {
                context.set_pin_value("max_frames", json!(0)).await.unwrap();
                context
                    .set_pin_value("include_audio", json!(true))
                    .await
                    .unwrap();
            }
            logic.run(&mut context).await.unwrap();
            assert_success(&context, 3).await;
            let calls = model.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].items[0].parts.len(), 1);
            match &calls[0].items[0].parts[0] {
                EmbeddingPart::Text(text) => assert_eq!(text.as_bytes(), MARKDOWN),
                EmbeddingPart::Image(image) => assert_eq!((image.width(), image.height()), (7, 5)),
                _ => panic!("file extension did not select the requested modality"),
            }
            assert_eq!(context.cache.read().await.len(), 2);
        }
    }
}

#[tokio::test]
async fn audio_node_dispatches_a_video_file_with_video_defaults() {
    let logic = Arc::new(EmbedAudioNode::new());
    let model = Arc::new(RecordingModel::new(vec![EmbeddingModality::Video]));
    let mut context = context(logic.clone(), Some(model.clone()), "video.MP4", Some(VIDEO)).await;
    logic.run(&mut context).await.unwrap();
    assert_success(&context, 3).await;
    let calls = model.calls();
    let [EmbeddingPart::Video(video)] = calls[0].items[0].parts.as_slice() else {
        panic!("expected video selected by its extension")
    };
    assert_eq!(video.frames.len(), 5);
    assert!(video.audio.is_none());
}

#[tokio::test]
async fn invalid_extensions_and_invalid_utf8_never_reach_the_model() {
    for name in ["unsupported.avi", "unsupported.unknown", "no-extension"] {
        let logic = Arc::new(EmbedAudioNode::new());
        let model = Arc::new(RecordingModel::new(vec![
            EmbeddingModality::Text,
            EmbeddingModality::Video,
        ]));
        let mut context = context(logic.clone(), Some(model.clone()), name, None).await;
        context
            .set_pin_value(
                "source",
                json!(FlowPath::new(
                    name.into(),
                    "unregistered-store".into(),
                    None
                )),
            )
            .await
            .unwrap();
        let error = logic.run(&mut context).await.unwrap_err().to_string();
        assert!(!error.contains("unregistered-store"), "{error}");
        assert!(model.calls().is_empty());
        assert_failed(&context).await;
    }
    let logic = Arc::new(EmbedVideoNode::new());
    let model = Arc::new(RecordingModel::new(vec![EmbeddingModality::Text]));
    let mut context = context(
        logic.clone(),
        Some(model.clone()),
        "broken.MD",
        Some(&[0xff, 0xfe]),
    )
    .await;
    assert!(logic.run(&mut context).await.is_err());
    assert!(model.calls().is_empty());
    assert_failed(&context).await;
}

#[tokio::test]
async fn media_preflight_rejects_unsupported_requests_before_resolving_the_source() {
    for (modalities, name, pin, value, expected) in [
        (
            vec![EmbeddingModality::Text],
            "missing.wav",
            "purpose",
            json!("document"),
            "Audio",
        ),
        (
            vec![EmbeddingModality::Audio],
            "missing.wav",
            "purpose",
            json!("classification"),
            "Classification",
        ),
        (
            vec![EmbeddingModality::Audio],
            "missing.wav",
            "options",
            json!({"dimensions": 1}),
            "dimensions",
        ),
        (
            vec![EmbeddingModality::Audio],
            "missing.PNG",
            "purpose",
            json!("document"),
            "Image",
        ),
        (
            vec![EmbeddingModality::Audio],
            "missing.MD",
            "purpose",
            json!("document"),
            "Text",
        ),
    ] {
        let logic = Arc::new(EmbedAudioNode::new());
        let model = Arc::new(RecordingModel::new(modalities));
        let mut context = context(logic.clone(), Some(model.clone()), name, None).await;
        context
            .set_pin_value(
                "source",
                json!(FlowPath::new(
                    name.into(),
                    "unregistered-store".into(),
                    None
                )),
            )
            .await
            .unwrap();
        context.set_pin_value(pin, value).await.unwrap();
        context.activate_exec_pin("exec_out").await.unwrap();
        let error = logic.run(&mut context).await.unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(
            !error.contains("unregistered-store"),
            "preflight reached source resolution"
        );
        assert!(model.calls().is_empty());
        assert_failed(&context).await;
    }
    for joint in [false, true] {
        let logic = Arc::new(EmbedVideoNode::new());
        let mut model =
            RecordingModel::new(vec![EmbeddingModality::Audio, EmbeddingModality::Video]);
        model.descriptor.joint_combinations.clear();
        let model = Arc::new(model);
        let mut context = context(logic.clone(), Some(model.clone()), "missing.mp4", None).await;
        context
            .set_pin_value(
                "source",
                json!(FlowPath::new(
                    "missing.mp4".into(),
                    "unregistered-store".into(),
                    None
                )),
            )
            .await
            .unwrap();
        if joint {
            context
                .set_pin_value("include_audio", json!(true))
                .await
                .unwrap();
        } else {
            context.set_pin_value("max_frames", json!(0)).await.unwrap();
        }
        let error = logic.run(&mut context).await.unwrap_err().to_string();
        assert!(!error.contains("unregistered-store"), "{error}");
        assert!(model.calls().is_empty());
        assert_failed(&context).await;
    }
}

#[tokio::test]
async fn missing_cache_or_file_never_activates_success() {
    let logic = Arc::new(EmbedAudioNode::new());
    let mut missing_cache = context(logic.clone(), None, "tone.wav", Some(WAV)).await;
    missing_cache.activate_exec_pin("exec_out").await.unwrap();
    assert!(logic.run(&mut missing_cache).await.is_err());
    assert_failed(&missing_cache).await;
    missing_cache
        .set_cache(
            "fixture-model",
            Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
        )
        .await;
    assert!(logic.run(&mut missing_cache).await.is_err());
    assert_failed(&missing_cache).await;
    let model = Arc::new(RecordingModel::new(vec![EmbeddingModality::Audio]));
    let mut missing_file = context(logic.clone(), Some(model.clone()), "missing.wav", None).await;
    missing_file.activate_exec_pin("exec_out").await.unwrap();
    assert!(logic.run(&mut missing_file).await.is_err());
    assert!(model.calls().is_empty());
    assert_failed(&missing_file).await;
}

#[tokio::test]
async fn backend_errors_and_invalid_vectors_never_activate_success() {
    for reply in [
        Reply::Failure,
        Reply::ExtraVector,
        Reply::NonFinite,
        Reply::WrongDimensions,
    ] {
        let logic = Arc::new(EmbedAudioNode::new());
        let mut model = RecordingModel::new(vec![EmbeddingModality::Audio]);
        model.reply = reply;
        let model = Arc::new(model);
        let mut context = context(logic.clone(), Some(model.clone()), "tone.wav", Some(WAV)).await;
        context.activate_exec_pin("exec_out").await.unwrap();
        assert!(logic.run(&mut context).await.is_err());
        assert_eq!(model.calls().len(), 1);
        assert_failed(&context).await;
    }
}

#[test]
fn media_embedding_nodes_are_registered_with_typed_pins() {
    let catalog = flow_like_catalog_media_video::get_catalog();
    for expected in [
        EmbedAudioNode::new().get_node(),
        EmbedVideoNode::new().get_node(),
    ] {
        assert_eq!(
            catalog
                .iter()
                .filter(|node| node.get_node().name == expected.name)
                .count(),
            1
        );
        for name in ["model", "source", "options", "result"] {
            let pin = expected.pins.values().find(|pin| pin.name == name).unwrap();
            assert!(pin.schema.is_some(), "{}:{name}", expected.name);
        }
        assert!(expected.pins.values().any(|pin| pin.name == "vector"));
    }
}

#[cfg(feature = "local-ml")]
#[tokio::test]
#[ignore = "Requires pinned q4 assets in FLOW_LIKE_GEMMA2_MODEL"]
async fn real_gemma2_runs_through_audio_and_video_nodes() -> flow_like_types::Result<()> {
    use flow_like_runtime::{bit::Bit, models::embedding_factory::EmbeddingFactory};
    use flow_like_storage::files::store::local_store::LocalObjectStore;
    let assets = std::path::PathBuf::from(std::env::var("FLOW_LIKE_GEMMA2_MODEL")?);
    let bit: Bit = flow_like_types::json::from_str(include_str!(
        "../../../model-provider/tests/fixtures/gemma2/bit.json"
    ))?;
    let spec: EmbeddingSpec = from_value(bit.parameters["embedding"].clone())?;
    let temporary = tempfile::tempdir()?;
    let store = Arc::new(LocalObjectStore::new(temporary.path().to_path_buf())?);
    for dependency in bit.inline_embedding_asset_bits()? {
        let artifact = spec
            .artifacts
            .values()
            .find(|artifact| artifact.bit == dependency.id)
            .unwrap();
        let target = dependency.to_path(&store).unwrap();
        std::fs::create_dir_all(target.parent().unwrap())?;
        let source = assets.join(&artifact.path);
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(source, target)?;
        }
    }
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::with_default_store(FlowLikeStore::Local(store)),
        HTTPClient::new_without_refetch(),
    ));
    let model = EmbeddingFactory::new()
        .build(&bit, state, None, None)
        .await?;
    let png = png_fixture();
    for (logic, name, bytes, dimensions, video, audio_ms) in [
        (
            Arc::new(EmbedAudioNode::new()) as Arc<dyn NodeLogic>,
            "tone.wav",
            WAV,
            128,
            false,
            1000,
        ),
        (
            Arc::new(EmbedVideoNode::new()) as Arc<dyn NodeLogic>,
            "offset.mp4",
            VIDEO_AUDIO,
            768,
            true,
            900,
        ),
        (
            Arc::new(EmbedVideoNode::new()) as Arc<dyn NodeLogic>,
            "picture.PNG",
            png.as_slice(),
            128,
            false,
            0,
        ),
        (
            Arc::new(EmbedAudioNode::new()) as Arc<dyn NodeLogic>,
            "notes.MD",
            MARKDOWN,
            256,
            false,
            0,
        ),
    ] {
        let mut context = context(logic.clone(), Some(model.clone()), name, Some(bytes)).await;
        context
            .set_pin_value(
                "options",
                json!(EmbeddingOptions {
                    dimensions: Some(dimensions)
                }),
            )
            .await?;
        if video {
            context.set_pin_value("include_audio", json!(true)).await?;
            context.set_pin_value("max_frames", json!(3)).await?;
        }
        logic.run(&mut context).await?;
        let result = assert_success(&context, dimensions).await;
        assert_eq!(result.space.id, spec.space_id);
        assert!(result.space.normalized);
        assert_eq!(result.provenance.model_id, bit.id);
        assert_eq!(result.provenance.pipeline_fingerprint.len(), 64);
        assert_eq!(result.usage.input_items, 1);
        assert_eq!(result.usage.audio_duration_ms, audio_ms);
        assert_eq!(result.usage.video_frames, if video { 3 } else { 0 });
    }
    Ok(())
}
