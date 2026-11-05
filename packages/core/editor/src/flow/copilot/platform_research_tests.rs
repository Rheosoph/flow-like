use super::*;
use crate::{state::FlowLikeConfig, utils::http::HTTPClient};
use flow_like_model_provider::llm::{CompletionModelDyn, CompletionModelHandle};
use rig::{
    agent::AgentBuilder,
    completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse},
    streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse},
};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone)]
struct ScriptedResearchModel {
    rounds: Arc<Mutex<VecDeque<Vec<RawStreamingChoice<()>>>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

impl CompletionModel for ScriptedResearchModel {
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
        unreachable!("research streams")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<StreamingCompletionResponse<()>, CompletionError> {
        self.requests.lock().unwrap().push(request);
        let mut chunks = self.rounds.lock().unwrap().pop_front().unwrap();
        chunks.push(RawStreamingChoice::FinalResponse(()));
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(chunks.into_iter().map(Ok)),
        )))
    }
}

struct Client(ScriptedResearchModel);

impl Client {
    fn new(rounds: Vec<Vec<RawStreamingChoice<()>>>) -> Self {
        Self(ScriptedResearchModel {
            rounds: Arc::new(Mutex::new(rounds.into())),
            requests: Arc::default(),
        })
    }

    fn request(&self, index: usize) -> String {
        serde_json::to_string(&self.0.requests.lock().unwrap()[index]).unwrap()
    }
}

impl CompletionClientDyn for Client {
    fn completion_model<'a>(&self, _: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(self.0.clone())
    }

    fn agent<'a>(&self, _: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        unreachable!()
    }
}

struct UnusedBridge;

#[async_trait]
impl PlatformToolBridge for UnusedBridge {
    async fn call(&self, name: &str, _: Value) -> String {
        panic!("research must run in core, not the bridge: {name}")
    }
}

fn text(value: &str) -> Vec<RawStreamingChoice<()>> {
    vec![RawStreamingChoice::Message(value.into())]
}

fn call(id: &str, name: &str, arguments: Value) -> Vec<RawStreamingChoice<()>> {
    vec![RawStreamingChoice::ToolCall(RawStreamingToolCall {
        id: id.into(),
        internal_call_id: id.into(),
        name: name.into(),
        arguments,
        ..RawStreamingToolCall::empty()
    })]
}

async fn run_root(client: &Client) -> String {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    PlatformCopilot::new(state, None)
        .with_tool_rounds(4)
        .with_tool_specs(vec![find_global_tool_spec(RESEARCH_AGENT_TOOL).unwrap()])
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Answer the owner's question. Private owner context: secret-canary.".into(),
            "warum besser als quadrate zb".into(),
            None,
            Vec::new(),
            "scripted".into(),
            client,
            None,
            Arc::new(UnusedBridge),
            None,
            None::<fn(String)>,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn bot_directs_research_with_context_and_refines_its_question() {
    let client = Client::new(vec![
        call(
            "initial",
            RESEARCH_AGENT_TOOL,
            json!({
                "question": "Compare H3 hexagonal cells with square grids.",
                "context": "We are discussing spatial indexing, especially neighbor distances."
            }),
        ),
        text("The first brief needs additional evidence about distortions."),
        call(
            "followup",
            RESEARCH_AGENT_TOOL,
            json!({
                "question": "Verify H3 distortions and pentagon limitations.",
                "context": "The previous comparison with square grids left distortions unverified."
            }),
        ),
        text("The follow-up could not verify the requested limitations."),
        text("I could not verify the H3 comparison."),
    ]);

    assert_eq!(
        run_root(&client).await,
        "I could not verify the H3 comparison."
    );
    assert_eq!(client.0.requests.lock().unwrap().len(), 5);
    let first_research = client.request(1);
    assert!(first_research.contains("Compare H3 hexagonal cells with square grids."));
    assert!(first_research.contains("especially neighbor distances"));
    assert!(!first_research.contains("warum besser als quadrate zb"));
    assert!(!first_research.contains("secret-canary"));
    let followup = client.request(3);
    assert!(followup.contains("Verify H3 distortions and pentagon limitations."));
    assert!(followup.contains("left distortions unverified"));
    assert!(!followup.contains("secret-canary"));
    assert!(client.request(4).contains("directed_by_assistant"));
}

#[tokio::test]
async fn missing_brief_returns_a_correctable_error_without_researching_the_last_message() {
    let client = Client::new(vec![
        call("invalid", RESEARCH_AGENT_TOOL, json!({})),
        call(
            "corrected",
            RESEARCH_AGENT_TOOL,
            json!({"question": "Compare H3 hexagonal cells with square grids."}),
        ),
        text("No verified evidence available."),
        text("I could not verify the comparison."),
    ]);

    run_root(&client).await;
    assert_eq!(client.0.requests.lock().unwrap().len(), 4);
    assert!(client.request(1).contains("invalid_research_brief"));
    assert!(
        client
            .request(2)
            .contains("Compare H3 hexagonal cells with square grids.")
    );
}

#[tokio::test]
async fn research_followups_preserve_the_turn_budget() {
    // Empty queries fail before network access, but still consume the reserved search allowance.
    let search_calls = (0..super::super::public_web::MAX_SEARCH_CALLS_PER_SESSION)
        .flat_map(|index| {
            call(
                &format!("search-{index}"),
                INTERNET_SEARCH_TOOL,
                json!({"query": ""}),
            )
        })
        .collect();
    let client = Client::new(vec![
        call(
            "initial",
            RESEARCH_AGENT_TOOL,
            json!({"question": "Compare H3 cells with squares."}),
        ),
        search_calls,
        text("The research budget is exhausted."),
        call(
            "followup",
            RESEARCH_AGENT_TOOL,
            json!({"question": "Verify H3 distortions."}),
        ),
        call(
            "search-followup",
            INTERNET_SEARCH_TOOL,
            json!({"query": ""}),
        ),
        text("The shared research budget remains exhausted."),
        text("I could not verify the comparison."),
    ]);
    run_root(&client).await;
    assert_eq!(client.0.requests.lock().unwrap().len(), 7);
    assert!(
        !client
            .request(2)
            .contains("search_session_call_budget_exceeded")
    );
    assert!(
        client
            .request(5)
            .contains("search_session_call_budget_exceeded")
    );
}
