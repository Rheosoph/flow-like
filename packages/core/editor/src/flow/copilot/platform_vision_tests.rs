use super::*;
use crate::flow::copilot::vision_reader::{ImageReader, MAX_IMAGES_PER_TURN, Reading};
use crate::{state::FlowLikeConfig, utils::http::HTTPClient};
use flow_like_model_provider::llm::{CompletionModelDyn, CompletionModelHandle};
use rig::{
    agent::AgentBuilder,
    completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
    streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse},
};
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct TextModel {
    rounds: Arc<Mutex<VecDeque<Vec<RawStreamingChoice<()>>>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

impl CompletionModel for TextModel {
    type Response = ();
    type StreamingResponse = ();
    type Client = ();

    fn make(_: &(), _: impl Into<String>) -> Self {
        unreachable!()
    }

    async fn completion(
        &self,
        _: CompletionRequest,
    ) -> std::result::Result<CompletionResponse<()>, CompletionError> {
        unreachable!("the platform loop streams")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<StreamingCompletionResponse<()>, CompletionError> {
        self.requests.lock().unwrap().push(request);
        let mut chunks = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model round");
        chunks.push(RawStreamingChoice::FinalResponse(()));
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(chunks.into_iter().map(Ok)),
        )))
    }
}

struct Client(TextModel);

impl CompletionClientDyn for Client {
    fn completion_model<'a>(&self, _: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(self.0.clone())
    }

    fn agent<'a>(&self, _: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        unreachable!()
    }
}

struct Reader {
    available: bool,
    reads: AtomicUsize,
}

impl Reader {
    fn new(available: bool) -> Arc<Self> {
        Arc::new(Self {
            available,
            reads: AtomicUsize::new(0),
        })
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ImageReader for Reader {
    async fn available(&self) -> bool {
        self.available
    }

    async fn read(&self, image: Image) -> Result<Reading> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let DocumentSourceKind::Url(url) = image.data else {
            panic!("the test images are links");
        };
        let mut usage = LLMUsageStats::default();
        usage.record_call(
            &Usage {
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
                ..Default::default()
            },
            "vision-model",
            "vision",
            5,
        );
        Ok(Reading {
            text: format!(
                "A tabby cat on a sofa, seen at {}.",
                url.rsplit('/').next().unwrap()
            ),
            usage,
        })
    }
}

struct ShotBridge;

#[async_trait]
impl PlatformToolBridge for ShotBridge {
    async fn call(&self, _: &str, _: Value) -> String {
        json!({
            "status": "ok",
            "apps": [],
            PLATFORM_TOOL_IMAGE_URLS_FIELD: [{"url": "https://example.com/shot.png", "media_type": "image/png"}]
        })
        .to_string()
    }
}

fn text(value: &str) -> Vec<RawStreamingChoice<()>> {
    vec![RawStreamingChoice::Message(value.into())]
}

fn list_apps_call() -> Vec<RawStreamingChoice<()>> {
    vec![RawStreamingChoice::ToolCall(RawStreamingToolCall {
        id: "call-1".into(),
        internal_call_id: "call-1".into(),
        name: "list_apps".into(),
        arguments: json!({}),
        ..RawStreamingToolCall::empty()
    })]
}

fn link(name: &str) -> ChatImage {
    ChatImage::link(format!("https://example.com/{name}"), "image/png".into())
}

struct Turn {
    answer: String,
    streamed: String,
    requests: Vec<String>,
}

async fn turn(
    vision: Option<Arc<Reader>>,
    sees_images: bool,
    images: Vec<ChatImage>,
    history: Vec<ChatMessage>,
    rounds: Vec<Vec<RawStreamingChoice<()>>>,
) -> Turn {
    let model = TextModel {
        rounds: Arc::new(Mutex::new(rounds.into())),
        requests: Arc::default(),
    };
    let tool_rounds = model.rounds.lock().unwrap().len() - 1;
    let output = Arc::new(Mutex::new(String::new()));
    let streamed = output.clone();
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let answer = PlatformCopilot::new(state, None)
        .with_tool_rounds(tool_rounds)
        .with_tool_specs(vec![find_global_tool_spec("list_apps").unwrap()])
        .with_vision(vision.map(|reader| Arc::new(ImageDetour::new(reader))))
        .chat_with_model(
            PlatformSurface::Orchestrator,
            "You are a bot.".into(),
            "What is in the picture?".into(),
            Some(images),
            history,
            "scripted".into(),
            &Client(model.clone()),
            None,
            ModelContext {
                sees_images,
                ..Default::default()
            },
            Arc::new(ShotBridge),
            None,
            Some(move |chunk: String| streamed.lock().unwrap().push_str(&chunk)),
        )
        .await
        .unwrap();
    let streamed = output.lock().unwrap().clone();
    let requests = model
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| serde_json::to_string(request).unwrap())
        .collect();
    Turn {
        answer,
        streamed,
        requests,
    }
}

#[tokio::test]
async fn a_text_only_model_gets_the_vision_models_description_and_the_usage_counts_the_read() {
    let reader = Reader::new(true);
    let turn = turn(
        Some(reader.clone()),
        false,
        vec![link("cat.png")],
        Vec::new(),
        vec![text("A cat.")],
    )
    .await;
    assert_eq!(turn.answer, "A cat.");
    assert_eq!(reader.reads(), 1);
    let request = &turn.requests[0];
    assert!(request.contains("A tabby cat on a sofa, seen at cat.png."));
    assert!(request.contains("as the bot's vision model describes it"));
    assert!(!request.contains("https://example.com/cat.png"));
    let usage = turn
        .streamed
        .split("<usage_stat>")
        .nth(1)
        .and_then(|rest| rest.split("</usage_stat>").next())
        .expect("the turn publishes its usage");
    let usage: Value = serde_json::from_str(usage).unwrap();
    assert_eq!(usage["step_name"], "Assistant");
    assert_eq!(usage["stats"]["usage"]["total_tokens"], 120);
    assert_eq!(usage["stats"]["calls"][0]["purpose"], "vision");
    assert_eq!(usage["stats"]["calls"][0]["model"], "vision-model");
}

#[tokio::test]
async fn a_model_that_sees_images_gets_them_and_the_vision_model_is_never_asked() {
    let reader = Reader::new(true);
    let turn = turn(
        Some(reader.clone()),
        true,
        vec![link("cat.png")],
        Vec::new(),
        vec![list_apps_call(), text("A cat.")],
    )
    .await;
    assert_eq!(reader.reads(), 0);
    assert!(turn.requests[0].contains("https://example.com/cat.png"));
    assert!(turn.requests[1].contains("https://example.com/shot.png"));
    assert!(!turn.streamed.contains("\"purpose\":\"vision\""));
}

#[tokio::test]
async fn without_a_vision_model_the_images_reach_the_model_as_before() {
    let turn = turn(
        None,
        false,
        vec![link("cat.png")],
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Earlier photo".into(),
            images: Some(vec![link("dog.png")]),
        }],
        vec![list_apps_call(), text("A cat.")],
    )
    .await;
    assert!(turn.requests[0].contains("https://example.com/cat.png"));
    assert!(turn.requests[0].contains("https://example.com/dog.png"));
    assert!(turn.requests[1].contains("https://example.com/shot.png"));
    assert!(!turn.requests[0].contains("vision model"));
}

#[tokio::test]
async fn a_vision_model_this_host_cannot_run_leaves_the_images_as_they_are() {
    let reader = Reader::new(false);
    let turn = turn(
        Some(reader.clone()),
        false,
        vec![link("cat.png")],
        Vec::new(),
        vec![text("A cat.")],
    )
    .await;
    assert_eq!(reader.reads(), 0);
    assert!(turn.requests[0].contains("https://example.com/cat.png"));
}

#[tokio::test]
async fn history_and_tool_result_images_are_described_in_place_for_a_text_only_model() {
    let reader = Reader::new(true);
    let turn = turn(
        Some(reader.clone()),
        false,
        vec![link("cat.png")],
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Earlier photo".into(),
            images: Some(vec![link("dog.png")]),
        }],
        vec![list_apps_call(), text("A cat and a dog.")],
    )
    .await;
    assert_eq!(reader.reads(), 3);
    assert!(turn.requests[0].contains("seen at cat.png"));
    assert!(turn.requests[0].contains("seen at dog.png"));
    assert!(!turn.requests[0].contains("https://example.com/"));
    let second = &turn.requests[1];
    assert!(second.contains("seen at shot.png"));
    assert!(!second.contains("https://example.com/shot.png"));
    let usage = turn.streamed.rsplit("<usage_stat>").next().unwrap();
    assert!(usage.contains("\"total_tokens\":360"));
}

#[tokio::test]
async fn the_vision_model_reads_at_most_its_bound_of_images_a_turn() {
    let reader = Reader::new(true);
    let images: Vec<ChatImage> = (0..MAX_IMAGES_PER_TURN + 2)
        .map(|index| link(&format!("photo-{index}.png")))
        .collect();
    let turn = turn(
        Some(reader.clone()),
        false,
        images,
        Vec::new(),
        vec![list_apps_call(), text("Many photos.")],
    )
    .await;
    assert_eq!(reader.reads(), MAX_IMAGES_PER_TURN);
    let first = &turn.requests[0];
    assert!(first.contains(&format!("seen at photo-{}.png", MAX_IMAGES_PER_TURN - 1)));
    assert!(!first.contains(&format!("seen at photo-{MAX_IMAGES_PER_TURN}.png")));
    assert!(!first.contains("https://example.com/"));
    assert_eq!(
        first
            .matches(&format!(
                "not read. The bot's vision model reads at most {MAX_IMAGES_PER_TURN} images a turn"
            ))
            .count(),
        2
    );
    assert!(turn.requests[1].contains("not read."));
    assert!(!turn.requests[1].contains("https://example.com/shot.png"));
}
