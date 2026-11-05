use super::*;
use crate::{state::FlowLikeConfig, utils::http::HTTPClient};
use flow_like_model_provider::llm::{CompletionModelDyn, CompletionModelHandle};
use rig::{
    agent::AgentBuilder,
    completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
    streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse},
};
use std::sync::Mutex;

#[derive(Default, Debug)]
struct Updates(Mutex<Vec<String>>);

#[async_trait]
impl RoomUpdates for Updates {
    async fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[derive(Clone)]
struct ScriptedModel {
    rounds: Arc<Mutex<Vec<Vec<RawStreamingChoice<()>>>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    updates: Arc<Updates>,
    update_round: usize,
}

impl CompletionModel for ScriptedModel {
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
        let round = {
            let mut requests = self.requests.lock().unwrap();
            let round = requests.len();
            requests.push(request);
            round
        };
        let chunks = self.rounds.lock().unwrap().remove(0);
        let updates = self.updates.clone();
        let update_round = self.update_round;
        let end = futures::stream::once(async move {
            if round == update_round {
                updates.0.lock().unwrap().push(
                    "Flux (member, not your owner) wrote: I already introduced myself.".into(),
                );
            }
            Ok(RawStreamingChoice::FinalResponse(()))
        });
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(chunks.into_iter().map(Ok)).chain(end),
        )))
    }
}

struct Client(ScriptedModel);

impl CompletionClientDyn for Client {
    fn completion_model<'a>(&self, _: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(self.0.clone())
    }

    fn agent<'a>(&self, _: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        unreachable!()
    }
}

#[derive(Default)]
struct Bridge(Mutex<usize>);

#[async_trait]
impl PlatformToolBridge for Bridge {
    async fn call(&self, _: &str, _: Value) -> String {
        *self.0.lock().unwrap() += 1;
        "completed tool work".into()
    }
}

fn text(value: &str) -> Vec<RawStreamingChoice<()>> {
    vec![RawStreamingChoice::Message(value.into())]
}

fn tool() -> Vec<RawStreamingChoice<()>> {
    vec![
        RawStreamingChoice::Message("I will check the apps first.".into()),
        RawStreamingChoice::ToolCall(RawStreamingToolCall {
            id: "call-1".into(),
            internal_call_id: "call-1".into(),
            name: "list_apps".into(),
            arguments: json!({}),
            ..RawStreamingToolCall::empty()
        }),
    ]
}

async fn run(
    rounds: Vec<Vec<RawStreamingChoice<()>>>,
    update_round: usize,
    tool_rounds: usize,
) -> (String, String, Vec<CompletionRequest>, usize) {
    let updates = Arc::new(Updates::default());
    let model = ScriptedModel {
        rounds: Arc::new(Mutex::new(rounds)),
        requests: Arc::default(),
        updates: updates.clone(),
        update_round,
    };
    let bridge = Arc::new(Bridge::default());
    let output = Arc::new(Mutex::new(String::new()));
    let streamed = output.clone();
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let copilot = PlatformCopilot::new(state, None)
        .with_tool_rounds(tool_rounds)
        .with_tool_specs(vec![find_global_tool_spec("list_apps").unwrap()]);
    let answer = copilot
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Answer as Joerg.".into(),
            "Introduce yourselves.".into(),
            None,
            Vec::new(),
            "scripted".into(),
            &Client(model.clone()),
            None,
            Arc::new(RoomUpdatesBridge::new(bridge.clone(), updates)),
            None,
            Some(move |chunk: String| streamed.lock().unwrap().push_str(&chunk)),
        )
        .await
        .unwrap();
    let streamed = output.lock().unwrap().clone();
    let requests = model.requests.lock().unwrap().clone();
    let calls = *bridge.0.lock().unwrap();
    (answer, streamed, requests, calls)
}

#[tokio::test]
async fn a_quiet_room_posts_one_answer_without_regeneration() {
    let (answer, streamed, requests, calls) = run(vec![text("I am Joerg.")], usize::MAX, 0).await;
    assert_eq!(answer, "I am Joerg.");
    assert!(streamed.contains("I am Joerg."));
    assert_eq!(requests.len(), 1);
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn room_updates_during_final_generation_replace_the_unpublished_draft() {
    let (answer, streamed, requests, calls) = run(
        vec![
            text("Flux, please introduce yourself."),
            text("I am Joerg. Hello Flux!"),
        ],
        0,
        0,
    )
    .await;
    assert_eq!(answer, "I am Joerg. Hello Flux!");
    assert!(!streamed.contains("please introduce yourself"));
    assert_eq!(
        requests.len(),
        2,
        "refresh gets a synthesis even with no tool rounds left"
    );
    assert!(
        serde_json::to_string(&requests[1])
            .unwrap()
            .contains("I already introduced myself")
    );
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn room_updates_discard_unexecuted_tools() {
    let (answer, _, requests, calls) = run(vec![tool(), text("I am Joerg.")], 0, 1).await;
    assert_eq!(answer, "I am Joerg.");
    assert_eq!(requests.len(), 2);
    assert_eq!(calls, 0, "the stale tool plan was never executed");
}

#[tokio::test]
async fn room_updates_preserve_completed_tools_when_the_final_draft_changes() {
    let (answer, streamed, requests, calls) = run(
        vec![
            tool(),
            text("Flux, please introduce yourself."),
            text("I am Joerg."),
        ],
        1,
        1,
    )
    .await;
    assert_eq!(answer, "I am Joerg.");
    assert_eq!(calls, 1, "completed work must not be replayed");
    assert!(!streamed.contains("please introduce yourself"));
    assert!(!streamed.contains("I will check the apps first"));
    let last = serde_json::to_string(&requests[2]).unwrap();
    assert!(last.contains("completed tool work"));
    assert!(last.contains("I already introduced myself"));
}
