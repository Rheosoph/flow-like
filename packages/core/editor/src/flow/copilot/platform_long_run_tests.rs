use super::*;
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
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

fn is_checkpoint(request: &CompletionRequest) -> bool {
    request.chat_history.iter().any(|message| matches!(
        message,
        rig::message::Message::System { content } if content.starts_with("Write a factual continuation checkpoint")
    )) || request.preamble.as_deref().is_some_and(|preamble| preamble.starts_with("Write a factual continuation checkpoint"))
}

#[derive(Clone)]
struct ScriptedModel {
    rounds: Arc<Mutex<VecDeque<Vec<RawStreamingChoice<()>>>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    cancelled: Arc<AtomicBool>,
    cancel_stream: bool,
    cancel_checkpoint: bool,
    fail_checkpoint: bool,
    stall_checkpoint: bool,
    fail_stream_at: Option<usize>,
    fail_chunk_at: Option<usize>,
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
        unreachable!("bot turns stream")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<StreamingCompletionResponse<()>, CompletionError> {
        let checkpoint = is_checkpoint(&request);
        let round = {
            let mut requests = self.requests.lock().unwrap();
            let round = requests.len();
            requests.push(request);
            round
        };
        if self.fail_stream_at == Some(round) {
            return Err(CompletionError::ResponseError(
                "private-provider-detail".into(),
            ));
        }
        if self.fail_chunk_at == Some(round) {
            let mut chunks: Vec<_> = call("partial-call", "list_apps", json!({}))
                .into_iter()
                .map(Ok)
                .collect();
            chunks.push(Err(CompletionError::ResponseError(
                "private-provider-detail".into(),
            )));
            return Ok(StreamingCompletionResponse::stream(Box::pin(
                futures::stream::iter(chunks),
            )));
        }
        if self.cancel_stream || checkpoint && self.cancel_checkpoint {
            self.cancelled.store(true, Ordering::Relaxed);
            return Ok(StreamingCompletionResponse::stream(Box::pin(
                futures::stream::pending(),
            )));
        }
        if checkpoint && self.stall_checkpoint {
            return Ok(StreamingCompletionResponse::stream(Box::pin(
                futures::stream::pending(),
            )));
        }
        let mut chunks = if checkpoint {
            if self.fail_checkpoint {
                Vec::new()
            } else {
                text(
                    "Inspected earlier pages. Artifact workspace/report.txt exists. Remaining work: verify the last pages. No additional permissions granted.",
                )
            }
        } else {
            self.rounds
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected model round")
        };
        chunks.push(RawStreamingChoice::FinalResponse(()));
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(chunks.into_iter().map(Ok)),
        )))
    }
}

struct Client(ScriptedModel);

impl Client {
    fn new(rounds: Vec<Vec<RawStreamingChoice<()>>>) -> Self {
        Self(ScriptedModel {
            rounds: Arc::new(Mutex::new(rounds.into())),
            requests: Arc::default(),
            cancelled: Arc::default(),
            cancel_stream: false,
            cancel_checkpoint: false,
            fail_checkpoint: false,
            stall_checkpoint: false,
            fail_stream_at: None,
            fail_chunk_at: None,
        })
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

struct Bridge {
    calls: AtomicUsize,
    cancellation_checks: AtomicUsize,
    cancelled: Arc<AtomicBool>,
    large_results: bool,
    steering: Mutex<Vec<String>>,
}

#[async_trait]
impl PlatformToolBridge for Bridge {
    async fn call(&self, name: &str, _: Value) -> String {
        assert_eq!(name, "list_apps", "work-plan tools must execute in core");
        let index = self.calls.fetch_add(1, Ordering::Relaxed);
        if self.large_results {
            json!({
                "status": "ok", "apps": [], "observation": "x".repeat(20_000), "page": index,
                PLATFORM_TOOL_IMAGE_URLS_FIELD: [{"url": format!("https://example.com/screen/{index}.png"), "media_type": "image/png"}]
            }).to_string()
        } else {
            json!({"status": "ok", "apps": [], "observation": index}).to_string()
        }
    }

    async fn is_cancelled(&self) -> bool {
        self.cancellation_checks.fetch_add(1, Ordering::Relaxed);
        self.cancelled.load(Ordering::Relaxed)
    }
    async fn drain_steering(&self) -> Vec<String> {
        std::mem::take(&mut *self.steering.lock().unwrap())
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

fn plan(revision: u64, status: &str) -> Value {
    json!({
        "expected_revision": revision, "objective": "Inspect every page and save the report", "status": status,
        "steps": [{"id": "inspect", "description": "Inspect pages and verify the report", "status": if status == "active" { "in_progress" } else { status }, "verification": if status == "completed" { "Tool verified workspace/report.txt" } else { "" }}],
        "checkpoint": if status == "blocked" { "The login needs the owner's input" } else { "Report path: workspace/report.txt" }
    })
}

async fn run(client: &Client, rounds: usize, large_results: bool) -> (Result<String>, Arc<Bridge>) {
    run_surface(client, rounds, large_results, PlatformSurface::Orchestrator).await
}

async fn run_surface(
    client: &Client,
    rounds: usize,
    large_results: bool,
    surface: PlatformSurface,
) -> (Result<String>, Arc<Bridge>) {
    run_with_images(client, rounds, large_results, surface, None).await
}

async fn run_with_images(
    client: &Client,
    rounds: usize,
    large_results: bool,
    surface: PlatformSurface,
    images: Option<Vec<ChatImage>>,
) -> (Result<String>, Arc<Bridge>) {
    run_conversation(
        client,
        rounds,
        large_results,
        surface,
        images,
        Vec::new(),
        None,
    )
    .await
}

async fn run_conversation(
    client: &Client,
    rounds: usize,
    large_results: bool,
    surface: PlatformSurface,
    images: Option<Vec<ChatImage>>,
    history: Vec<ChatMessage>,
    scope: Option<&str>,
) -> (Result<String>, Arc<Bridge>) {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let bridge = Arc::new(Bridge {
        calls: AtomicUsize::new(0),
        cancellation_checks: AtomicUsize::new(0),
        cancelled: client.0.cancelled.clone(),
        large_results,
        steering: Mutex::new(vec!["Keep the report in German.".into()]),
    });
    let answer = PlatformCopilot::new(state, None)
        .with_tool_specs(vec![find_global_tool_spec("list_apps").unwrap()])
        .with_tool_rounds(rounds)
        .with_long_running()
        .with_conversation_scope(scope.map(str::to_owned))
        .chat_with_client(
            surface,
            "Help the owner.".into(),
            "Inspect every page and save the report.".into(),
            images,
            history,
            "scripted".into(),
            client,
            None,
            bridge.clone(),
            None,
            None::<fn(String)>,
        )
        .await;
    (answer, bridge)
}

#[tokio::test]
async fn extended_bot_turns_execute_more_than_twelve_rounds_and_reserve_synthesis() {
    let mut rounds: Vec<_> = (0..16)
        .map(|index| call(&format!("call-{index}"), "list_apps", json!({})))
        .collect();
    rounds.push(text("Verified all pages."));
    let client = Client::new(rounds);
    let (answer, bridge) = run(&client, 16, false).await;
    let answer = answer.unwrap();
    assert!(answer.starts_with("Verified all pages."));
    assert!(answer.contains("step limit"));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 16);
    let requests = client.0.requests.lock().unwrap();
    assert_eq!(requests.len(), 17);
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == UPDATE_WORK_PLAN_TOOL)
    );
    assert!(requests.last().unwrap().tools.is_empty());
}

#[tokio::test]
async fn active_plan_continues_after_progress_text_and_finishes_after_verification() {
    let client = Client::new(vec![
        call("plan", UPDATE_WORK_PLAN_TOOL, plan(0, "active")),
        text("I will inspect the remaining page."),
        call("inspect", "list_apps", json!({})),
        call("finish", UPDATE_WORK_PLAN_TOOL, plan(1, "completed")),
        text("The report is complete."),
    ]);
    let (answer, bridge) = run(&client, 20, false).await;
    assert!(answer.unwrap().ends_with("The report is complete."));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 1);
    let requests = client.0.requests.lock().unwrap();
    let continued = serde_json::to_string(&requests[2]).unwrap();
    assert!(continued.contains("Your work plan still has unfinished work"));
    assert!(continued.contains("workspace/report.txt"));
    assert!(continued.contains("Keep the report in German"));
}

#[tokio::test]
async fn blocked_plans_and_quick_answers_end_without_extra_model_calls() {
    let client = Client::new(vec![
        call("plan", UPDATE_WORK_PLAN_TOOL, plan(0, "blocked")),
        text("The login needs your input."),
    ]);
    let (answer, _) = run(&client, 20, false).await;
    let answer = answer.unwrap();
    assert!(answer.starts_with("The login needs your input."));
    assert!(answer.contains("Last recorded progress for:"));
    assert_eq!(client.0.requests.lock().unwrap().len(), 2);
    let client = Client::new(vec![text("A quick answer.")]);
    assert_eq!(run(&client, 20, false).await.0.unwrap(), "A quick answer.");
    assert_eq!(client.0.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn repeated_progress_text_stops_with_an_unfinished_notice() {
    let client = Client::new(vec![
        call("plan", UPDATE_WORK_PLAN_TOOL, plan(0, "active")),
        text("Still working."),
        text("Still working."),
        text("Still working."),
        text("Still working."),
    ]);
    let answer = run(&client, 128, false).await.0.unwrap();
    assert!(answer.contains("The work plan is still unfinished."));
    assert_eq!(client.0.requests.lock().unwrap().len(), 5);
}

fn assert_complete_tool_pairs(request: &CompletionRequest) {
    let mut pending = HashSet::new();
    for message in request.chat_history.iter() {
        match message {
            rig::message::Message::Assistant { content, .. } => {
                assert!(pending.is_empty(), "previous tool calls have no results");
                for content in content.iter() {
                    if let AssistantContent::ToolCall(call) = content {
                        pending.insert(call.id.clone());
                    }
                }
            }
            rig::message::Message::User { content } => {
                for content in content.iter() {
                    if let UserContent::ToolResult(result) = content {
                        assert!(
                            pending.remove(&result.id),
                            "orphan tool result: {}",
                            result.id
                        );
                    }
                }
            }
            _ => {}
        }
    }
    assert!(pending.is_empty(), "request ended with dangling calls");
}

#[tokio::test]
async fn compaction_preserves_tool_pairs_original_request_steering_and_recent_images() {
    let mut rounds: Vec<_> = (0..10)
        .map(|index| call(&format!("call-{index}"), "list_apps", json!({})))
        .collect();
    rounds.push(text("Verified every page."));
    let client = Client::new(rounds);
    assert!(run(&client, 20, true).await.0.is_ok());
    let requests = client.0.requests.lock().unwrap();
    assert!(requests.iter().any(is_checkpoint));
    let mut saw_recent_images = false;
    for request in requests.iter().filter(|request| !is_checkpoint(request)) {
        assert_complete_tool_pairs(request);
        let images = request
            .chat_history
            .iter()
            .filter_map(|message| match message {
                rig::message::Message::User { content } => Some(
                    content
                        .iter()
                        .filter(|item| matches!(item, UserContent::Image(_)))
                        .count(),
                ),
                _ => None,
            })
            .sum::<usize>();
        assert!(images <= 2, "old screenshots accumulated: {images}");
        saw_recent_images |= images == 2;
    }
    assert!(
        saw_recent_images,
        "the model never received recent screenshots"
    );
    let last = serde_json::to_string(requests.last().unwrap()).unwrap();
    assert!(last.contains("Original user request, retained verbatim"));
    assert!(last.contains("Inspect every page and save the report."));
    assert!(last.contains("Keep the report in German."));
    assert!(last.contains("untrusted_historical_checkpoint"));
    assert!(last.contains("workspace/report.txt"));
}

fn large_rounds(count: usize) -> Vec<Vec<RawStreamingChoice<()>>> {
    let mut rounds: Vec<_> = (0..count)
        .map(|index| call(&format!("call-{index}"), "list_apps", json!({})))
        .collect();
    rounds.push(text("Verified every page."));
    rounds
}

/// `count` messages of about 6,000 characters each, the person and the bot in turn.
fn long_talk(count: usize) -> Vec<ChatMessage> {
    (0..count)
        .map(|index| ChatMessage {
            role: if index % 2 == 0 {
                ChatRole::User
            } else {
                ChatRole::Assistant
            },
            content: format!("message-{index} {}", "detail ".repeat(850)),
            images: None,
        })
        .collect()
}

fn exchange(reply: &str) -> [ChatMessage; 2] {
    [
        ChatMessage {
            role: ChatRole::User,
            content: "Inspect every page and save the report.".into(),
            images: None,
        },
        ChatMessage {
            role: ChatRole::Assistant,
            content: reply.into(),
            images: None,
        },
    ]
}

async fn converse(client: &Client, talk: &[ChatMessage], scope: &str) -> String {
    let surface = PlatformSurface::Orchestrator;
    run_conversation(client, 4, false, surface, None, talk.to_vec(), Some(scope))
        .await
        .0
        .unwrap()
}

fn reply_request(client: &Client) -> String {
    let requests = client.0.requests.lock().unwrap();
    let reply = requests
        .iter()
        .find(|request| !is_checkpoint(request))
        .expect("the turn asked the model for a reply");
    serde_json::to_string(reply).unwrap()
}

fn checkpoints(client: &Client) -> usize {
    let requests = client.0.requests.lock().unwrap();
    requests
        .iter()
        .filter(|request| is_checkpoint(request))
        .count()
}

#[tokio::test]
async fn a_checkpoint_that_never_arrives_does_not_hold_the_turn_back() {
    let mut client = Client::new(large_rounds(12));
    client.0.stall_checkpoint = true;
    let (answer, bridge) = tokio::time::timeout(Duration::from_secs(5), run(&client, 20, true))
        .await
        .expect("the turn waited for its checkpoint");
    assert!(answer.unwrap().contains("Verified every page."));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 12);
    let requests = client.0.requests.lock().unwrap();
    assert!(requests.iter().any(is_checkpoint));
    for request in requests.iter().filter(|request| !is_checkpoint(request)) {
        assert_complete_tool_pairs(request);
    }
    let last = serde_json::to_string(requests.last().unwrap()).unwrap();
    assert!(last.contains("Older tool result shortened"));
    assert!(!last.contains("untrusted_historical_checkpoint"));
}

#[tokio::test]
async fn a_failed_checkpoint_stops_the_turn_only_when_nothing_else_frees_context() {
    let mut client = Client::new(large_rounds(12));
    client.0.fail_checkpoint = true;
    let (answer, bridge) = run(&client, 20, true).await;
    assert!(answer.unwrap().contains("Verified every page."));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 12);

    let mut client = Client::new(vec![call("call-0", "list_apps", json!({}))]);
    client.0.fail_checkpoint = true;
    let surface = PlatformSurface::Orchestrator;
    let (answer, bridge) =
        run_conversation(&client, 20, false, surface, None, long_talk(34), None).await;
    assert!(answer.unwrap().contains("task is unfinished"));
    assert_eq!(bridge.calls.load(Ordering::Relaxed), 0);
    assert!(is_checkpoint(
        client.0.requests.lock().unwrap().last().unwrap()
    ));
}

#[tokio::test]
async fn a_stop_at_the_hard_limit_leaves_recorded_artifacts_and_remaining_work_in_saved_reply() {
    let mut rounds = vec![call("plan", UPDATE_WORK_PLAN_TOOL, plan(0, "active"))];
    rounds.extend((0..10).map(|index| {
        let mut round = text(&"Inspecting pages. ".repeat(1_500));
        round.extend(call(&format!("call-{index}"), "list_apps", json!({})));
        round
    }));
    let mut client = Client::new(rounds);
    client.0.fail_checkpoint = true;
    let answer = run(&client, 20, false).await.0.unwrap();
    assert!(answer.contains("task is unfinished"));
    assert!(answer.contains("Report path: workspace/report.txt"));
    assert!(answer.contains("Unfinished: Inspect pages and verify the report"));
}

#[tokio::test]
async fn the_next_turn_starts_from_the_summary_the_previous_turn_began() {
    let scope = "summary-reuse";
    let mut talk = long_talk(14);
    let first = Client::new(vec![text("First answer.")]);
    assert_eq!(converse(&first, &talk, scope).await, "First answer.");
    assert!(
        reply_request(&first).contains("message-0 "),
        "the reply waited for the summary"
    );

    talk.extend(exchange("First answer."));
    let second = Client::new(vec![text("Second answer.")]);
    converse(&second, &talk, scope).await;
    let request = reply_request(&second);
    assert!(request.contains("Earlier part of this conversation, summarized"));
    assert!(request.contains("workspace/report.txt"));
    assert!(!request.contains("message-0 "));
    assert!(
        request.contains("message-13 "),
        "recent messages stay word for word"
    );
    assert!(request.contains("First answer."));

    talk.extend(exchange("Second answer."));
    let third = Client::new(vec![text("Third answer.")]);
    converse(&third, &talk, scope).await;
    let request = reply_request(&third);
    assert!(request.contains("Earlier part of this conversation, summarized"));
    assert!(request.contains("Second answer."));
    assert_eq!(
        checkpoints(&first) + checkpoints(&second) + checkpoints(&third),
        1,
        "a later turn summarized the conversation again"
    );

    talk[10].content = "message-10 was edited".into();
    let fourth = Client::new(vec![text("Fourth answer.")]);
    converse(&fourth, &talk, scope).await;
    let request = reply_request(&fourth);
    assert!(!request.contains("Earlier part of this conversation, summarized"));
    assert!(request.contains("message-10 was edited"));
}

#[tokio::test]
async fn cancellation_interrupts_stalled_model_streams_and_compaction() {
    let mut client = Client::new(vec![text("must not finish")]);
    client.0.cancel_stream = true;
    let result = tokio::time::timeout(Duration::from_secs(2), run(&client, 20, false))
        .await
        .unwrap()
        .0;
    assert!(result.unwrap_err().to_string().contains("cancelled"));

    let rounds = (0..10)
        .map(|index| call(&format!("call-{index}"), "list_apps", json!({})))
        .collect();
    let mut client = Client::new(rounds);
    client.0.cancel_checkpoint = true;
    let result = tokio::time::timeout(Duration::from_secs(2), run(&client, 20, true))
        .await
        .unwrap()
        .0;
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[tokio::test]
async fn zero_round_schema_repairs_have_no_plan_tool() {
    let client = Client::new(vec![text("{\"result\":true}")]);
    assert!(run(&client, 0, false).await.0.is_ok());
    assert!(client.0.requests.lock().unwrap()[0].tools.is_empty());
}

#[tokio::test]
async fn long_progress_does_not_displace_the_final_saved_answer() {
    for surface in [PlatformSurface::Orchestrator, PlatformSurface::Subtask] {
        let mut progress = text(&"Inspecting pages. ".repeat(1_500));
        progress.extend(call("inspect", "list_apps", json!({})));
        let client = Client::new(vec![
            progress,
            text("Saved the verified report to workspace/report.txt."),
        ]);
        let answer = run_surface(&client, 20, false, surface).await.0.unwrap();
        assert_eq!(answer, "Saved the verified report to workspace/report.txt.");
    }
}

#[tokio::test]
async fn unfinished_children_return_typed_outcomes_instead_of_success() {
    let client = Client::new(vec![
        call("inspect", "list_apps", json!({})),
        text("The remaining pages need inspection."),
    ]);
    let error = run_surface(&client, 1, false, PlatformSurface::Subtask)
        .await
        .0
        .unwrap_err();
    let incomplete = error.downcast_ref::<IncompleteWork>().unwrap();
    assert_eq!(incomplete.reason, WorkStopReason::Budget);
    assert!(incomplete.output.contains("remaining pages"));

    let client = Client::new(vec![
        call("blocked", UPDATE_WORK_PLAN_TOOL, plan(0, "blocked")),
        text("Login needs your input."),
    ]);
    let error = run_surface(&client, 20, false, PlatformSurface::Subtask)
        .await
        .0
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<IncompleteWork>().unwrap().reason,
        WorkStopReason::Blocked
    );
}

#[test]
fn oversized_tool_output_is_bounded_and_explicitly_marked() {
    let output = format!("start {} end", "🦀".repeat(100_000));
    let bounded = context::bounded_tool_output(&output, 4);
    assert!(bounded.chars().count() <= 8_000);
    assert!(bounded.starts_with("start "));
    assert!(bounded.ends_with(" end"));
    assert!(bounded.contains("narrower read or query"));
}

#[tokio::test]
async fn cancellation_is_polled_per_stream_not_per_token() {
    let client = Client::new(vec![
        (0..2_000)
            .map(|_| RawStreamingChoice::Message("x".into()))
            .collect(),
    ]);
    let (answer, bridge) = run(&client, 20, false).await;
    assert_eq!(answer.unwrap().len(), 2_000);
    assert!(bridge.cancellation_checks.load(Ordering::Relaxed) < 20);
}

#[tokio::test]
async fn original_reference_images_survive_compaction_while_old_screenshots_expire() {
    let mut rounds: Vec<_> = (0..10)
        .map(|index| call(&format!("call-{index}"), "list_apps", json!({})))
        .collect();
    rounds.push(text("The report matches the reference."));
    let client = Client::new(rounds);
    let reference = "b3JpZ2luYWwtcmVmZXJlbmNl";
    let answer = run_with_images(
        &client,
        20,
        true,
        PlatformSurface::Orchestrator,
        Some(vec![ChatImage::inline(
            reference.into(),
            "image/png".into(),
        )]),
    )
    .await
    .0
    .unwrap();
    assert_eq!(answer, "The report matches the reference.");
    let requests = client.0.requests.lock().unwrap();
    let mut summaries = 0;
    for request in requests.iter() {
        let request_json = serde_json::to_string(request).unwrap();
        if is_checkpoint(request) {
            summaries += 1;
            assert!(
                !request_json.contains(reference),
                "summarizer received image bytes"
            );
        } else {
            assert!(request_json.contains(reference), "original image was lost");
        }
    }
    assert!(summaries > 0);
    let last = serde_json::to_string(requests.last().unwrap()).unwrap();
    assert!(!last.contains("https://example.com/screen/0.png"));
    assert!(last.contains("https://example.com/screen/9.png"));
}

#[tokio::test]
async fn model_failures_after_work_preserve_handoff_and_do_not_replay_actions() {
    for chunk_failure in [false, true] {
        for surface in [PlatformSurface::Orchestrator, PlatformSurface::Subtask] {
            let mut client = Client::new(vec![
                call("plan", UPDATE_WORK_PLAN_TOOL, plan(0, "active")),
                call("inspection", "list_apps", json!({})),
            ]);
            if chunk_failure {
                client.0.fail_chunk_at = Some(2);
            } else {
                client.0.fail_stream_at = Some(2);
            }
            let (answer, bridge) = run_surface(&client, 20, false, surface).await;
            assert_eq!(
                bridge.calls.load(Ordering::Relaxed),
                1,
                "partial or completed actions were replayed"
            );
            assert_eq!(client.0.requests.lock().unwrap().len(), 3);
            let saved_reply = match surface {
                PlatformSurface::Subtask => {
                    let error = answer.unwrap_err();
                    let incomplete = error.downcast_ref::<IncompleteWork>().unwrap();
                    assert_eq!(incomplete.reason, WorkStopReason::Blocked);
                    incomplete.output.clone()
                }
                _ => answer.unwrap(),
            };
            assert!(saved_reply.contains("model could not complete the next step"));
            assert!(saved_reply.contains("workspace/report.txt"));
            assert!(saved_reply.contains("Inspect pages and verify the report"));
            assert!(!saved_reply.contains("private-provider-detail"));
        }
    }
}

#[tokio::test]
async fn initial_model_failures_and_cancellation_remain_errors() {
    let mut client = Client::new(Vec::new());
    client.0.fail_stream_at = Some(0);
    assert!(run(&client, 20, false).await.0.is_err());

    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(Bridge {
        calls: AtomicUsize::new(0),
        cancellation_checks: AtomicUsize::new(0),
        cancelled: Arc::new(AtomicBool::new(true)),
        large_results: false,
        steering: Mutex::new(Vec::new()),
    });
    let result =
        context::model_result::<(), _>(Err("provider disconnected"), true, &bridge, "Stream error")
            .await;
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[path = "platform_recovery_tests.rs"]
mod recovery;

struct LinkBridge {
    link: String,
}

#[async_trait]
impl PlatformToolBridge for LinkBridge {
    async fn call(&self, name: &str, _: Value) -> String {
        assert_eq!(name, "list_apps", "image_view must execute in core");
        json!({ "status": "ok", "apps": [], "preview": { "page": 1, "url": self.link } })
            .to_string()
    }
}

async fn serve_png() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await;
            let body = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body).await;
        }
    });
    format!("http://{address}/tmp/page-1.png?X-Amz-Signature=abc")
}

async fn run_image_view_turn(client: &Client, link: String) -> Result<String> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    PlatformCopilot::new(state, None)
        .with_tool_specs(vec![
            find_global_tool_spec("list_apps").unwrap(),
            super::super::image_view::image_view_tool_spec(),
        ])
        .with_tool_rounds(6)
        .chat_with_client(
            PlatformSurface::Orchestrator,
            "Help the owner.".into(),
            "Look at the first page.".into(),
            None,
            Vec::new(),
            "scripted".into(),
            client,
            None,
            Arc::new(LinkBridge { link }),
            None,
            None::<fn(String)>,
        )
        .await
}

fn image_links(request: &CompletionRequest) -> Vec<String> {
    request
        .chat_history
        .iter()
        .filter_map(|message| match message {
            rig::message::Message::User { content } => Some(content.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            UserContent::Image(Image {
                data: DocumentSourceKind::Url(url),
                ..
            }) => Some(url.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_link_a_tool_returned_reaches_the_next_model_call_as_an_image() {
    let link = serve_png().await;
    let client = Client::new(vec![
        call("list", "list_apps", json!({})),
        call("view", IMAGE_VIEW_TOOL, json!({ "urls": [link] })),
        text("Page one shows a chart."),
    ]);
    let answer = run_image_view_turn(&client, link.clone()).await.unwrap();
    assert!(answer.contains("Page one shows a chart."));
    let requests = client.0.requests.lock().unwrap();
    assert!(image_links(&requests[1]).is_empty());
    assert_eq!(image_links(&requests[2]), [link]);
    let viewed = serde_json::to_string(&requests[2]).unwrap();
    assert!(viewed.contains("preceding tool result(s) of image_view"));
    assert!(!viewed.contains(PLATFORM_TOOL_IMAGE_URLS_FIELD));
}

#[tokio::test]
async fn a_link_the_model_made_up_is_refused_and_never_attached() {
    let link = serve_png().await;
    let invented = link.replace("page-1", "page-2");
    let client = Client::new(vec![
        call("list", "list_apps", json!({})),
        call("view", IMAGE_VIEW_TOOL, json!({ "urls": [invented] })),
        text("I could not view it."),
    ]);
    run_image_view_turn(&client, link).await.unwrap();
    let requests = client.0.requests.lock().unwrap();
    assert!(image_links(&requests[2]).is_empty());
    let refused = serde_json::to_string(&requests[2]).unwrap();
    assert!(refused.contains("no_viewable_image"));
}
