use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flow_like::flow::copilot::HostTools;
use flow_like::flow::copilot::platform::PlatformToolBridge;
use flow_like::flow::copilot::tool_grants::{GrantKey, GrantStore};
use flow_like::flow_like_model_provider::llm::ModelLogic;
use flow_like_bot_tools::{KnowledgeEmbedder, ModelRole};
use flow_like_bot_tools_host::{
    AppAccess, JobApp, JobScope, ModelSource, ToolJob, ToolReply, job_hash,
};
use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::object_store::memory::InMemory;
use flow_like_types::channel::{Channel, ChannelPush, ChannelPushKind, InProcessChannel};
use flow_like_types::data_classification::DataClassification;
use flow_like_types::tokio::sync::mpsc;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::host::{AppDirectory, BotToolsHost, TurnDeps, TurnModels};
use super::{ToolExecutor, routing};
use crate::routes::ai::global_chat::{GlobalChatFrame, ServerPlatformBridge};

const OWNER: &str = "user-1";
const TURN: &str = "turn-1";
const WORKSPACE: &str = "ws-bot";
const DOCS: &str = "app-docs";

struct Access {
    stores: HashMap<String, FlowLikeStore>,
    connections: HashMap<String, Connection>,
    _dirs: Vec<TempDir>,
}

impl Access {
    async fn new(apps: &[&str]) -> Self {
        let mut stores = HashMap::new();
        let mut connections = HashMap::new();
        let mut dirs = Vec::new();
        for app in apps {
            stores.insert(
                app.to_string(),
                FlowLikeStore::Memory(Arc::new(InMemory::new())),
            );
            let dir = tempfile::tempdir().expect("tempdir");
            connections.insert(
                app.to_string(),
                connect_lance(dir.path().to_str().expect("path"))
                    .execute()
                    .await
                    .expect("lance"),
            );
            dirs.push(dir);
        }
        Self {
            stores,
            connections,
            _dirs: dirs,
        }
    }
}

#[async_trait]
impl AppAccess for Access {
    async fn store(&self, app_id: &str) -> flow_like_types::Result<FlowLikeStore> {
        self.stores
            .get(app_id)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("no store for {app_id}"))
    }

    async fn connection(&self, app_id: &str) -> flow_like_types::Result<Connection> {
        self.connections
            .get(app_id)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("no tables for {app_id}"))
    }
}

struct Directory {
    scope: JobScope,
}

#[async_trait]
impl AppDirectory for Directory {
    async fn scope(&self) -> Result<JobScope, String> {
        Ok(self.scope.clone())
    }

    async fn credential(&self, app_id: &str) -> Result<Value, String> {
        Ok(json!({ "credential_for": app_id }))
    }
}

struct NoModels;

#[async_trait]
impl ModelSource for NoModels {
    async fn model(&self, _role: ModelRole) -> flow_like_types::Result<Arc<dyn ModelLogic>> {
        Err(flow_like_types::anyhow!("no model"))
    }

    async fn embedder(&self) -> flow_like_types::Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        Ok(None)
    }
}

#[derive(Default)]
struct FakeExecutor {
    jobs: Mutex<Vec<(String, String, String, String)>>,
    fail: AtomicBool,
}

#[async_trait]
impl ToolExecutor for FakeExecutor {
    async fn run(
        &self,
        owner: &str,
        turn_id: &str,
        workspace: &str,
        job: String,
        _timeout: Duration,
    ) -> Result<ToolReply, String> {
        self.jobs.lock().unwrap().push((
            owner.into(),
            turn_id.into(),
            workspace.into(),
            job.clone(),
        ));
        if self.fail.load(Ordering::SeqCst) {
            return Err("connection refused at 10.0.0.7".into());
        }
        let job: ToolJob = serde_json::from_str(&job).unwrap();
        Ok(ToolReply {
            output: json!({ "status": "ok", "tool": job.tool, "ran_on": "executor" }).to_string(),
            cpu_ms: 7,
        })
    }
}

fn job_scope() -> JobScope {
    JobScope {
        storage: true,
        data_connections: false,
        classification: DataClassification::Internal,
        apps: vec![JobApp {
            app_id: DOCS.into(),
            alias: None,
            files: true,
            tables: true,
            column_attributes: None,
        }],
    }
}

#[derive(Default)]
struct MemoryGrants(Mutex<Vec<GrantKey>>);

#[async_trait]
impl GrantStore for MemoryGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        self.0.lock().unwrap().push(GrantKey::new(tool, scope));
        Ok(())
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        let mut keys = self.0.lock().unwrap();
        let before = keys.len();
        keys.retain(|key| !(key.tool == tool && key.scope == scope));
        Ok(keys.len() != before)
    }
}

async fn host_with(
    executor: Option<Arc<FakeExecutor>>,
    host_tools: HostTools,
) -> Arc<BotToolsHost> {
    host_with_grants(executor, host_tools, Arc::new(MemoryGrants::default())).await
}

async fn host_with_grants(
    executor: Option<Arc<FakeExecutor>>,
    host_tools: HostTools,
    grants: Arc<MemoryGrants>,
) -> Arc<BotToolsHost> {
    let access = Arc::new(Access::new(&[WORKSPACE, DOCS]).await);
    Arc::new(BotToolsHost::new(TurnDeps {
        owner: OWNER.into(),
        turn_id: TURN.into(),
        workspace: WORKSPACE.into(),
        host_tools,
        directory: Arc::new(Directory { scope: job_scope() }),
        access,
        models: TurnModels {
            source: Arc::new(NoModels),
            chat: None,
            embedding: None,
        },
        executor: executor.map(|executor| executor as Arc<dyn ToolExecutor>),
        grants,
    }))
}

fn parse(output: &str) -> Value {
    serde_json::from_str(output).expect("tool output is JSON")
}

#[flow_like_types::tokio::test]
async fn light_tools_run_in_the_api_and_never_reach_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let listed = parse(&host.run("file_list", json!({})).await);
    assert_eq!(listed["status"], "ok");
    assert!(executor.jobs.lock().unwrap().is_empty());
}

#[flow_like_types::tokio::test]
async fn heavy_tools_become_a_job_that_carries_only_the_scoped_credentials() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let output = parse(
        &host
            .run("file_write", json!({ "path": "bot/a.md", "content": "x" }))
            .await,
    );
    assert_eq!(output["ran_on"], "executor");
    let jobs = executor.jobs.lock().unwrap();
    let (owner, turn, workspace, text) = &jobs[0];
    assert_eq!(
        (owner.as_str(), turn.as_str(), workspace.as_str()),
        (OWNER, TURN, WORKSPACE)
    );
    let job: ToolJob = serde_json::from_str(text).unwrap();
    assert_eq!(job.tool, "file_write");
    assert_eq!(job.owner, OWNER);
    let apps: Vec<_> = job.credentials.keys().cloned().collect();
    assert_eq!(apps, [DOCS, WORKSPACE]);
    assert_eq!(job.scope.apps.len(), 1);
}

#[flow_like_types::tokio::test]
async fn documents_in_a_light_tool_move_to_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let output = parse(
        &host
            .run("file_read", json!({ "file": "bot/files/a.pdf" }))
            .await,
    );
    assert_eq!(output["ran_on"], "executor");
    assert_eq!(executor.jobs.lock().unwrap().len(), 1);
}

#[flow_like_types::tokio::test]
async fn without_an_executor_heavy_calls_are_refused_and_nothing_is_offered() {
    let host = host_with(None, routing::host_tools(false, &[])).await;
    let refused = parse(&host.run("pdf_render", json!({ "file": "a.pdf" })).await);
    assert_eq!(refused["code"], "bot_tool_needs_executor");
    assert!(!host.handles("pdf_render"));
    assert!(host.handles("file_list"));
}

#[flow_like_types::tokio::test]
async fn an_executor_failure_is_reported_without_its_details() {
    let executor = Arc::new(FakeExecutor::default());
    executor.fail.store(true, Ordering::SeqCst);
    let host = host_with(Some(executor), routing::host_tools(true, &[])).await;
    let output = host
        .run("file_write", json!({ "path": "a", "content": "b" }))
        .await;
    assert_eq!(parse(&output)["code"], "bot_tool_executor_failed");
    assert!(!output.contains("10.0.0.7"));
}

#[flow_like_types::tokio::test]
async fn a_turn_runs_a_bounded_number_of_executor_jobs() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let mut last = Value::Null;
    for _ in 0..31 {
        last = parse(
            &host
                .run("file_write", json!({ "path": "a", "content": "b" }))
                .await,
        );
    }
    assert_eq!(last["code"], "bot_tool_turn_limit");
    assert_eq!(executor.jobs.lock().unwrap().len(), 30);
}

const ASKING_CALLS: [(&str, &str); 3] = [
    ("knowledge_drop", "table:pets"),
    ("code_run", "host:example.com"),
    ("data_sources", "connection:postgres:example.com::d"),
];

fn asking_args(tool: &str) -> Value {
    match tool {
        "knowledge_drop" => json!({ "index": "pets" }),
        "code_run" => {
            json!({ "language": "python", "code": "print(1)", "network_hosts": ["example.com"] })
        }
        _ => {
            json!({ "operation": "attach", "attach": { "name": "x", "kind": "postgres", "connection": { "secret": "db", "host": "example.com", "database": "d" } } })
        }
    }
}

#[flow_like_types::tokio::test]
async fn a_headless_turn_parks_a_call_that_asks_and_runs_the_rest() {
    let executor = Arc::new(FakeExecutor::default());
    for (tool, scope) in ASKING_CALLS {
        let host = host_with(
            Some(executor.clone()),
            routing::host_tools(true, &["python".to_string()]),
        )
        .await;
        let mutating = AtomicBool::new(false);
        let waiting = parse(
            &host
                .run_unattended(&mutating, tool, asking_args(tool))
                .await,
        );
        assert_eq!(waiting["code"], "bot_tool_waiting_for_approval", "{tool}");
        let card = host.parked().expect("a card");
        assert_eq!((card.tool.as_str(), card.scope.as_str()), (tool, scope));
        assert_eq!(card.turn_id, TURN);
        assert!(!mutating.load(Ordering::SeqCst));
        let after = parse(&host.run_unattended(&mutating, "file_list", json!({})).await);
        assert_eq!(after["code"], "bot_tool_waiting_for_approval");
    }
    assert!(executor.jobs.lock().unwrap().is_empty());

    let host = host_with(
        Some(executor.clone()),
        routing::host_tools(true, &["python".to_string()]),
    )
    .await;
    let mutating = AtomicBool::new(false);
    let ran = parse(
        &host
            .run_unattended(
                &mutating,
                "code_run",
                json!({ "language": "python", "code": "print(1)" }),
            )
            .await,
    );
    assert_eq!(ran["ran_on"], "executor");
    let written = parse(
        &host
            .run_unattended(
                &mutating,
                "file_write",
                json!({ "path": "a", "content": "b" }),
            )
            .await,
    );
    assert_eq!(written["status"], "ok");
    assert!(mutating.load(Ordering::SeqCst));
    assert!(host.parked().is_none());
}

#[flow_like_types::tokio::test]
async fn a_headless_turn_runs_what_a_grant_or_a_one_time_answer_covers() {
    let executor = Arc::new(FakeExecutor::default());
    let grants = Arc::new(MemoryGrants::default());
    grants.put("code_run", "host:example.com").await.unwrap();
    let host = host_with_grants(
        Some(executor.clone()),
        routing::host_tools(true, &["python".to_string()]),
        grants.clone(),
    )
    .await;
    let mutating = AtomicBool::new(false);
    let ran = parse(
        &host
            .run_unattended(&mutating, "code_run", asking_args("code_run"))
            .await,
    );
    assert_eq!(ran["ran_on"], "executor");
    assert!(host.parked().is_none());

    let hash =
        flow_like_types::tool_grant::args_hash("knowledge_drop", &asking_args("knowledge_drop"));
    grants
        .put("knowledge_drop", &format!("once:{hash}"))
        .await
        .unwrap();
    let dropped = parse(
        &host
            .run_unattended(&mutating, "knowledge_drop", asking_args("knowledge_drop"))
            .await,
    );
    assert_ne!(dropped["code"], "bot_tool_waiting_for_approval");
    assert!(host.parked().is_none());
    let again = parse(
        &host
            .run_unattended(&mutating, "knowledge_drop", asking_args("knowledge_drop"))
            .await,
    );
    assert_eq!(again["code"], "bot_tool_waiting_for_approval");
}

#[flow_like_types::tokio::test]
async fn a_refusal_the_owner_gave_is_told_to_the_model_and_runs_nothing() {
    let executor = Arc::new(FakeExecutor::default());
    let grants = Arc::new(MemoryGrants::default());
    let hash =
        flow_like_types::tool_grant::args_hash("knowledge_drop", &asking_args("knowledge_drop"));
    grants
        .put("knowledge_drop", &format!("deny:{hash}"))
        .await
        .unwrap();
    let host = host_with_grants(Some(executor), routing::host_tools(true, &[]), grants).await;
    let refused = parse(
        &host
            .run_unattended(
                &AtomicBool::new(false),
                "knowledge_drop",
                asking_args("knowledge_drop"),
            )
            .await,
    );
    assert_eq!(refused["code"], "bot_tool_refused_by_owner");
    assert!(host.parked().is_none());
}

#[test]
fn the_job_token_binds_the_job_owner_turn_and_workspace() {
    crate::backend_jwt::init_for_tests();
    let job = r#"{"tool":"file_write"}"#;
    let token = super::executor::mint(OWNER, TURN, WORKSPACE, job).unwrap();
    let claims = crate::execution::verify_execution_jwt(&token).unwrap();
    assert_eq!(claims.sub, OWNER);
    assert_eq!(claims.run_id, TURN);
    assert_eq!(claims.app_id, WORKSPACE);
    assert!(claims.board_id.is_empty());
    assert!(claims.unattended);
    assert_eq!(claims.dispatch_hash, Some(job_hash(job)));
    assert_ne!(
        claims.dispatch_hash,
        Some(job_hash(r#"{"tool":"file_read"}"#))
    );
}

#[test]
fn the_reply_text_matches_the_text_core_gives_a_local_host() {
    use flow_like::flow::copilot::tool_outcome_output;
    let mut output = flow_like_bot_tools::ToolOutput::new(json!({ "n": 1 }));
    output.images.push(flow_like_bot_tools::ToolImage {
        label: "p".into(),
        mime: "image/png".into(),
        data: flow_like_types::Bytes::from_static(b"png"),
    });
    let outcome = flow_like_bot_tools::ToolOutcome::from_output("pdf_render", output, 16 * 1024);
    assert_eq!(
        flow_like_bot_tools_host::outcome_output(&outcome),
        tool_outcome_output(&outcome)
    );
}

async fn bridge(
    host: Arc<BotToolsHost>,
) -> (
    Arc<ServerPlatformBridge>,
    Arc<InProcessChannel>,
    mpsc::UnboundedReceiver<GlobalChatFrame>,
) {
    let channel = InProcessChannel::register(
        flow_like_types::channel::new_request_id(),
        Duration::from_secs(30),
    )
    .await;
    let (frames, received) = mpsc::unbounded_channel();
    let bridge =
        Arc::new(ServerPlatformBridge::bot(channel.clone(), frames).with_bot_tools(Some(host)));
    (bridge, channel, received)
}

fn frame(frame: GlobalChatFrame) -> Value {
    match frame {
        GlobalChatFrame::ToolRequest(request) => request,
        GlobalChatFrame::Token(_) => panic!("expected a tool request"),
    }
}

async fn reply(channel: &Arc<InProcessChannel>, request: &Value, value: Value) {
    channel
        .push(ChannelPush {
            channel_id: channel.channel_id().to_string(),
            request_id: request["requestId"].as_str().map(str::to_string),
            kind: ChannelPushKind::Reply,
            value,
        })
        .await;
}

#[flow_like_types::tokio::test]
async fn the_bridge_runs_bot_tools_on_the_server_and_other_tools_in_the_browser() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let (bridge, channel, mut received) = bridge(host).await;

    let in_process = parse(&bridge.call("file_list", json!({})).await);
    assert_eq!(in_process["status"], "ok");
    let on_executor = parse(
        &bridge
            .call("file_write", json!({ "path": "a.md", "content": "x" }))
            .await,
    );
    assert_eq!(on_executor["ran_on"], "executor");
    assert!(matches!(
        received.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    let browser = {
        let bridge = bridge.clone();
        flow_like_types::tokio::spawn(async move {
            bridge
                .call(
                    "database_tool",
                    json!({ "operation": "list_tables", "app_id": DOCS }),
                )
                .await
        })
    };
    let request = frame(received.recv().await.unwrap());
    assert_eq!(request["toolName"], "database_tool");
    assert!(request.get("approvalOnly").is_none());
    reply(
        &channel,
        &request,
        json!({ "requestId": request["requestId"], "approved": true, "result": { "tables": [] } }),
    )
    .await;
    assert_eq!(parse(&browser.await.unwrap())["status"], "ok");
    channel.close().await;
}

#[flow_like_types::tokio::test]
async fn an_approval_gated_bot_tool_asks_the_browser_for_consent_only() {
    let host = host_with(None, routing::host_tools(false, &[])).await;
    let (bridge, channel, mut received) = bridge(host).await;
    let call = {
        let bridge = bridge.clone();
        flow_like_types::tokio::spawn(async move {
            bridge
                .call("knowledge_drop", json!({ "index": "pets" }))
                .await
        })
    };
    let request = frame(received.recv().await.unwrap());
    assert_eq!(request["toolName"], "knowledge_drop");
    assert_eq!(request["approvalOnly"], true);
    assert_eq!(request["approval"]["kind"], "mutating");
    reply(
        &channel,
        &request,
        json!({ "requestId": request["requestId"], "approved": true, "result": { "status": "ok", "native_approved": true } }),
    )
    .await;
    let ran = parse(&call.await.unwrap());
    assert_eq!(ran["tool"], "knowledge_drop");
    assert_ne!(ran["status"], "denied");
    channel.close().await;
}

#[flow_like_types::tokio::test]
async fn a_declined_or_unacknowledged_approval_runs_nothing() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(
        Some(executor.clone()),
        routing::host_tools(true, &["python".to_string()]),
    )
    .await;
    let (bridge, channel, mut received) = bridge(host).await;
    for (answer, status) in [
        (
            json!({ "approved": false, "error": "User denied the request." }),
            "denied",
        ),
        (
            json!({ "approved": true, "result": { "status": "ok" } }),
            "error",
        ),
    ] {
        let call = {
            let bridge = bridge.clone();
            flow_like_types::tokio::spawn(async move {
                bridge
                    .call(
                        "code_run",
                        json!({ "language": "python", "code": "1", "network_hosts": ["example.com"] }),
                    )
                    .await
            })
        };
        let request = frame(received.recv().await.unwrap());
        assert_eq!(request["approvalOnly"], true);
        let mut answer = answer;
        answer["requestId"] = request["requestId"].clone();
        reply(&channel, &request, answer).await;
        assert_eq!(parse(&call.await.unwrap())["status"], status);
    }
    assert!(executor.jobs.lock().unwrap().is_empty());
    channel.close().await;
}
