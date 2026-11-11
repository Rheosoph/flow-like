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

fn research(question: &str) -> Vec<RawStreamingChoice<()>> {
    call(
        "research",
        RESEARCH_AGENT_TOOL,
        json!({ "question": question }),
    )
}

fn silence() -> Vec<RawStreamingChoice<()>> {
    Vec::new()
}

#[tokio::test]
async fn an_empty_reply_after_research_is_asked_again_before_the_turn_ends() {
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        text("BMW cut its outlook."),
        silence(),
        text("BMW cut its outlook this week."),
    ]);

    assert_eq!(run_root(&client).await, "BMW cut its outlook this week.");
    assert!(client.request(3).contains("Your last reply was empty"));
    assert!(client.request(3).contains("BMW cut its outlook."));
}

#[tokio::test]
async fn findings_are_shown_when_the_model_never_writes_its_own_answer() {
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        text("BMW cut its outlook for the second time this year."),
        silence(),
        silence(),
        silence(),
    ]);

    let answer = run_root(&client).await;
    assert!(answer.contains("here are the researcher's findings"));
    assert!(answer.contains("BMW cut its outlook for the second time this year."));
    assert!(!answer.contains("without a usable final synthesis"));
    assert_eq!(client.0.requests.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn a_researcher_that_answers_with_nothing_is_nudged_to_write_up() {
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        silence(),
        text("Findings after the nudge."),
        text("Answer from the findings."),
    ]);

    assert_eq!(run_root(&client).await, "Answer from the findings.");
    assert!(client.request(2).contains("Your last reply was empty"));
    assert!(client.request(3).contains("Findings after the nudge."));
}

#[tokio::test]
async fn a_researcher_without_findings_or_pages_reports_a_failure_the_bot_can_act_on() {
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        silence(),
        silence(),
        silence(),
        text("I could not research this."),
    ]);

    assert_eq!(run_root(&client).await, "I could not research this.");
    let after_research = client.request(4);
    assert!(after_research.contains("sealed_research_failed"));
    assert!(after_research.contains("its model returned no text"));
}

#[tokio::test]
async fn rounds_that_find_nothing_close_the_tools_and_ask_for_the_write_up() {
    let failing_search =
        |id: &str, query: &str| call(id, INTERNET_SEARCH_TOOL, json!({ "query": query }));
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        failing_search("first", ""),
        failing_search("second", " "),
        failing_search("third", "  "),
        text("Nothing could be established."),
        text("I found nothing."),
    ]);

    assert_eq!(run_root(&client).await, "I found nothing.");
    let requests = client.0.requests.lock().unwrap();
    assert!(
        !requests[3].tools.is_empty(),
        "two fruitless rounds keep the tools"
    );
    assert!(requests[4].tools.is_empty(), "the third closes them");
    drop(requests);
    assert!(
        client
            .request(4)
            .contains("Several rounds in a row returned nothing usable")
    );
}

#[tokio::test]
async fn a_repeated_search_is_answered_without_running_again() {
    let same = || call("again", INTERNET_SEARCH_TOOL, json!({ "query": "" }));
    let client = Client::new(vec![
        research("What did BMW announce this week?"),
        same(),
        same(),
        text("Nothing new."),
        text("Nothing new to report."),
    ]);

    run_root(&client).await;
    assert!(!client.request(2).contains("duplicate_search"));
    assert!(client.request(3).contains("duplicate_search"));
}

#[test]
fn pages_opened_by_a_run_that_never_wrote_up_come_back_as_excerpts() {
    let session = WebResearchSession::new("");
    let evidence = vec![EvidencePage {
        title: "BMW warns on profits".into(),
        url: "https://example.com/bmw".into(),
        excerpt: "BMW issued its second profit warning.".into(),
    }];
    let outcome = unfinished_research(
        evidence,
        LLMUsageStats::default(),
        &session,
        "its time ran out before the write-up",
    )
    .unwrap();
    assert_eq!(
        outcome.incomplete,
        Some("its time ran out before the write-up")
    );
    assert!(
        outcome
            .findings
            .contains("### [BMW warns on profits](https://example.com/bmw)\nBMW issued")
    );
    assert!(
        unfinished_research(
            Vec::new(),
            LLMUsageStats::default(),
            &session,
            "its model returned no text"
        )
        .is_err()
    );

    let opened = json!({
        "status": "ok",
        "citation_eligible": true,
        "url": "https://example.com/bmw",
        "source": { "title": "BMW [warns] on profits" },
        "content": "x".repeat(EVIDENCE_EXCERPT_CHARS + 10),
    });
    let page = evidence_page(&opened).unwrap();
    assert_eq!(page.title, "BMW warns on profits");
    assert_eq!(page.excerpt.chars().count(), EVIDENCE_EXCERPT_CHARS);
    assert!(evidence_page(&json!({ "status": "error", "url": "https://example.com" })).is_none());
}

/// A tool round holding one result, as the researcher's transcript stores it.
fn result_round(output: Value) -> rig::message::Message {
    rig::message::Message::User {
        content: OneOrMany::one(UserContent::ToolResult(RigToolResult {
            id: "call".into(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::text(output.to_string())),
        })),
    }
}

fn result_of(message: &rig::message::Message) -> Value {
    let rig::message::Message::User { content } = message else {
        panic!("a tool round is a user message")
    };
    let UserContent::ToolResult(result) = content.first() else {
        panic!("a tool round holds a tool result")
    };
    let ToolResultContent::Text(text) = result.content.first() else {
        panic!("a tool result holds text")
    };
    serde_json::from_str(&text.text).unwrap()
}

fn page_round(index: usize) -> rig::message::Message {
    result_round(json!({
        "status": "ok",
        "tool": OPEN_URL_TOOL,
        "url": format!("https://example.com/{index}"),
        "content": "page text ".repeat(9_000),
        "find": { "match_count": 1 },
    }))
}

#[test]
fn older_results_lose_their_page_text_once_the_transcript_is_large() {
    let search = result_round(json!({
        "status": "ok",
        "tool": INTERNET_SEARCH_TOOL,
        "results": [{
            "title": "A result",
            "url": "https://example.com/r",
            "content": "snippet",
            "publishedDate": "2026-11-04",
            "source_id": "src_1",
        }],
    }));
    let mut history = vec![search, page_round(0), page_round(1), page_round(2)];
    clear_old_research_results(&mut history);

    assert_eq!(
        result_of(&history[0])["results"][0],
        json!({ "title": "A result", "url": "https://example.com/r", "publishedDate": "2026-11-04" })
    );
    let cleared = result_of(&history[1]);
    assert_eq!(cleared["context_cleared"], true);
    assert_eq!(cleared["url"], "https://example.com/0");
    assert_eq!(cleared["find"]["match_count"], 1);
    let kept = cleared["content"].as_str().unwrap();
    assert!(kept.starts_with("page text ") && kept.contains("cleared from your context"));
    assert!(kept.chars().count() < CLEARED_PAGE_HEAD_CHARS + 300);
    for recent in &history[2..] {
        assert!(
            result_of(recent).get("context_cleared").is_none(),
            "the latest rounds stay whole"
        );
    }

    clear_old_research_results(&mut history);
    assert_eq!(
        result_of(&history[1]),
        cleared,
        "clearing twice changes nothing"
    );
}

#[test]
fn a_small_transcript_keeps_every_result_whole() {
    let mut history = vec![page_round(0)];
    clear_old_research_results(&mut history);
    assert!(result_of(&history[0]).get("context_cleared").is_none());
}

#[test]
fn research_keeps_time_of_a_durable_turn_for_the_answer() {
    assert_eq!(research_time_budget(900, None), Duration::from_secs(900));
    let now = chrono::Utc::now().timestamp_millis();
    let short = research_time_budget(900, Some(now + 300_000));
    assert!(short <= Duration::from_secs(240) && short > Duration::from_secs(230));
    assert_eq!(research_time_budget(900, Some(now - 1)), Duration::ZERO);
    assert_eq!(
        research_time_budget(900, Some(now + 3_600_000)),
        Duration::from_secs(900)
    );
}

#[tokio::test]
async fn search_arguments_are_checked_before_any_request_is_sent() {
    let invalid = run_internet_search(&json!({ "query": "bmw", "category": "images" })).await;
    assert_eq!(invalid["code"], "invalid_category");
    let invalid = run_internet_search(&json!({ "query": "bmw", "language": "de_DE!" })).await;
    assert_eq!(invalid["code"], "invalid_language");
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
