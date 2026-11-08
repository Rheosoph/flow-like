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
    AppAccess, JobApp, JobScope, JobSecrets, ModelSource, ToolJob, ToolReply, job_hash,
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
use crate::middleware::jwt::BotUser;
use crate::routes::ai::global_chat::{GlobalChatFrame, ServerPlatformBridge};

const OWNER: &str = "user-1";
const TURN: &str = "turn-1";
const BOT: &str = "bot-1";
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
    skill_proposal: Mutex<Option<flow_like_types::bot_skill::CreateBotSkill>>,
    output: Mutex<Option<Value>>,
}

#[async_trait]
impl ToolExecutor for FakeExecutor {
    async fn run(
        &self,
        owner: &str,
        bot: &BotUser,
        workspace: &str,
        job: String,
        _timeout: Duration,
    ) -> Result<ToolReply, super::executor::ToolExecutorError> {
        self.jobs.lock().unwrap().push((
            owner.into(),
            bot.turn_id.clone(),
            workspace.into(),
            job.clone(),
        ));
        if self.fail.load(Ordering::SeqCst) {
            return Err(crate::execution::DispatchError::Network(
                "connection refused at 10.0.0.7".into(),
            )
            .into());
        }
        let job: ToolJob = serde_json::from_str(&job).unwrap();
        let proposal = self.skill_proposal.lock().unwrap().clone();
        let output = self.output.lock().unwrap().clone().unwrap_or_else(|| {
            let mut output = json!({ "status": "ok", "tool": job.tool, "ran_on": "executor" });
            if job.tool == "skill_propose" {
                output["proposed"] = json!(true);
                output["name"] = json!(proposal.as_ref().map(|proposal| proposal.name.as_str()));
            }
            output
        });
        Ok(ToolReply {
            output: output.to_string(),
            cpu_ms: 7,
            skill_proposal: proposal,
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

fn bot_user() -> BotUser {
    BotUser {
        bot_id: BOT.into(),
        turn_id: TURN.into(),
        conv_id: None,
    }
}

async fn host_with_grants(
    executor: Option<Arc<FakeExecutor>>,
    host_tools: HostTools,
    grants: Arc<MemoryGrants>,
) -> Arc<BotToolsHost> {
    let access = Arc::new(Access::new(&[WORKSPACE, DOCS]).await);
    Arc::new(BotToolsHost::new(TurnDeps {
        owner: OWNER.into(),
        bot: bot_user(),
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
        secrets: Arc::new(JobSecrets(
            [("PG_SALES", "pg-pa55"), ("OTHER", "other-value")]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        )),
        connectors: None,
    }))
}

fn parse(output: &str) -> Value {
    serde_json::from_str(output).expect("tool output is JSON")
}

#[derive(Default)]
struct Configuration {
    updates: Mutex<Vec<Value>>,
}

#[async_trait]
impl flow_like_bot_tools::configuration::ConfigurationBackend for Configuration {
    async fn inspect(&self) -> flow_like_types::Result<Value> {
        Ok(json!({"id": BOT, "name": "Assistant", "capabilities": []}))
    }

    async fn models(&self) -> flow_like_types::Result<Value> {
        Ok(json!({"models": [{"id": "profile-chat", "roles": ["primary"]}]}))
    }

    async fn update(&self, changes: Value) -> flow_like_types::Result<Value> {
        self.updates.lock().unwrap().push(changes.clone());
        Ok(changes)
    }
}

struct NoConfigurationApps;

#[async_trait]
impl AppDirectory for NoConfigurationApps {
    async fn scope(&self) -> Result<JobScope, String> {
        Err("This bot has no app scope".into())
    }

    async fn credential(&self, _app_id: &str) -> Result<Value, String> {
        Err("Configuration must not request app credentials".into())
    }
}

async fn configuration_host(
    grants: Arc<MemoryGrants>,
) -> (Arc<BotToolsHost>, Arc<Configuration>) {
    let configuration = Arc::new(Configuration::default());
    let host = BotToolsHost::new(TurnDeps {
        owner: OWNER.into(),
        bot: bot_user(),
        turn_id: TURN.into(),
        workspace: String::new(),
        host_tools: routing::host_tools(false, &[]),
        directory: Arc::new(NoConfigurationApps),
        access: Arc::new(Access::new(&[]).await),
        models: TurnModels {
            source: Arc::new(NoModels),
            chat: None,
            embedding: None,
        },
        executor: None,
        grants,
        secrets: Arc::new(JobSecrets(Default::default())),
        connectors: None,
    })
    .with_configuration(configuration.clone());
    (Arc::new(host), configuration)
}

#[flow_like_types::tokio::test]
async fn configuration_discovery_needs_no_workspace_apps_or_executor() {
    let (host, configuration) = configuration_host(Arc::new(MemoryGrants::default())).await;
    for tool in ["bot_configuration", "bot_models"] {
        let output = parse(&host.run_unattended(&AtomicBool::new(false), tool, json!({})).await);
        assert_eq!(output["status"], "ok", "{output}");
        assert!(host.parked().is_none());
    }
    assert!(configuration.updates.lock().unwrap().is_empty());
}

struct ContextWithoutApps;

#[async_trait]
impl flow_like_bot_tools::user_context::UserContextBackend for ContextWithoutApps {
    async fn available(&self) -> flow_like_types::Result<flow_like_bot_tools::user_context::UserContextAvailability> {
        Ok(flow_like_bot_tools::user_context::UserContextAvailability {
            participants: vec![flow_like_bot_tools::user_context::UserContextParticipant {
                id: OWNER.into(), name: "Owner".into(),
            }],
            fields: flow_like_types::bot_user_context::UserContextField::ALL.to_vec(),
            device_support: "unknown_until_shared".into(),
        })
    }

    async fn request(&self, args: flow_like_bot_tools::user_context::UserContextRequestArgs) -> flow_like_types::Result<flow_like_bot_tools::user_context::UserContextRequestReceipt> {
        Ok(flow_like_bot_tools::user_context::UserContextRequestReceipt {
            request_id: "context-request".into(), conversation_id: "room".into(),
            subject_id: args.subject_id, expires_at: 100_000,
        })
    }
}

#[flow_like_types::tokio::test]
async fn context_exchange_needs_no_workspace_app_scope_or_owner_approval() {
    let host = BotToolsHost::new(TurnDeps {
        owner: OWNER.into(), bot: bot_user(), turn_id: TURN.into(), workspace: String::new(),
        host_tools: routing::host_tools(false, &[]),
        directory: Arc::new(NoConfigurationApps),
        access: Arc::new(Access::new(&[]).await),
        models: TurnModels { source: Arc::new(NoModels), chat: None, embedding: None },
        executor: None,
        grants: Arc::new(MemoryGrants::default()),
        secrets: Arc::new(JobSecrets(Default::default())),
        connectors: None,
    }).with_user_context(Arc::new(ContextWithoutApps));
    let mutating = AtomicBool::new(false);
    let available = parse(&host.run_unattended(&mutating, "user_context_available", json!({})).await);
    assert_eq!(available["status"], "ok", "{available}");
    assert_eq!(available["participants"][0]["id"], OWNER);
    let requested = parse(&host.run_unattended(&mutating, "user_context_request", json!({
        "subject_id": OWNER, "fields": ["timezone"], "purpose": "Find a meeting time",
    })).await);
    assert_eq!(requested["status"], "pending", "{requested}");
    assert!(requested.get("values").is_none());
    let proposed = parse(&host.run_unattended(&mutating, "user_context_set_status", json!({
        "subject_id": OWNER, "status": "focus", "purpose": "Time to write", "duration_minutes": 60,
    })).await);
    assert_eq!(proposed["status"], "pending", "{proposed}");
    assert!(host.parked().is_none());

    let channel = InProcessChannel::register("user-context-without-apps", Duration::from_secs(30)).await;
    let (frames, mut received) = mpsc::unbounded_channel();
    let plain = ServerPlatformBridge::orchestrator(channel.clone(), frames.clone());
    let refused = parse(&plain.call("user_context_available", json!({})).await);
    assert_eq!(refused["code"], "platform_tool_not_advertised");
    let bot_without_host = ServerPlatformBridge::bot(channel.clone(), frames.clone());
    let refused = parse(&bot_without_host.call("user_context_available", json!({})).await);
    assert_eq!(refused["code"], "platform_tool_not_advertised");
    let enabled = ServerPlatformBridge::orchestrator(channel, frames)
        .with_bot_tools(Some(Arc::new(host)));
    let listed = parse(&enabled.call("user_context_available", json!({})).await);
    assert_eq!(listed["participants"][0]["id"], OWNER);
    assert!(received.try_recv().is_err(), "Context runs on the host without a browser tool request");
}

#[flow_like_types::tokio::test]
async fn configuration_changes_ignore_standing_grants_and_consume_exact_approval_once() {
    let tool = "bot_update_configuration";
    let args = json!({"changes": {"name": "Research assistant", "capabilities": ["internet"]}});
    let grants = Arc::new(MemoryGrants::default());
    grants.put(tool, "*").await.unwrap();
    let (host, configuration) = configuration_host(grants.clone()).await;
    let mutating = AtomicBool::new(false);
    let waiting = parse(&host.run_unattended(&mutating, tool, args.clone()).await);
    assert_eq!(waiting["code"], "bot_tool_waiting_for_approval");
    assert!(host.parked().unwrap().scope.is_empty());
    assert!(configuration.updates.lock().unwrap().is_empty());
    assert!(!mutating.load(Ordering::SeqCst));

    let hash = flow_like_types::tool_grant::args_hash(tool, &args);
    grants.put(tool, &format!("once:{hash}")).await.unwrap();
    let (resumed, configuration) = configuration_host(grants).await;
    let applied = parse(&resumed.run_unattended(&mutating, tool, args.clone()).await);
    assert_eq!(applied["status"], "ok", "{applied}");
    assert!(mutating.load(Ordering::SeqCst));
    assert_eq!(*configuration.updates.lock().unwrap(), vec![args["changes"].clone()]);
    let repeated = parse(&resumed.run_unattended(&mutating, tool, args).await);
    assert_eq!(repeated["code"], "bot_tool_waiting_for_approval");
    assert_eq!(configuration.updates.lock().unwrap().len(), 1);
}

#[flow_like_types::tokio::test]
async fn light_tools_run_in_the_api_and_never_reach_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let listed = parse(&host.run("file_list", json!({})).await);
    assert_eq!(listed["status"], "ok");
    assert!(executor.jobs.lock().unwrap().is_empty());
}

struct Skills(
    flow_like_types::bot_skill::BotSkill,
    Mutex<Vec<flow_like_types::bot_skill::CreateBotSkill>>,
    AtomicBool,
);

#[async_trait]
impl flow_like_bot_tools::SkillHost for Skills {
    async fn get(
        &self,
        name: &str,
    ) -> flow_like_types::Result<Option<flow_like_types::bot_skill::BotSkill>> {
        Ok((self.0.name == name).then(|| self.0.clone()))
    }

    async fn propose(
        &self,
        request: flow_like_types::bot_skill::CreateBotSkill,
    ) -> flow_like_types::Result<flow_like_types::bot_skill::BotSkill> {
        if self.2.load(Ordering::SeqCst) {
            return Err(flow_like_types::anyhow!("The bot already has 20 skills waiting for approval"));
        }
        self.1.lock().unwrap().push(request);
        Ok(self.0.clone())
    }
}

#[async_trait]
impl super::skills::SkillJobs for Skills {
    async fn job(&self, name: &str) -> Result<flow_like_bot_tools_host::SkillCallJob, String> {
        assert_eq!(name, self.0.name);
        if self.0.status != flow_like_types::bot_skill::SkillStatus::Active {
            return Err(format!("You have no active skill named '{name}'"));
        }
        Ok(flow_like_bot_tools_host::SkillCallJob {
            skill: self.0.clone(),
            package_id: self.0.source.package_id().unwrap_or_default().into(),
            package: self.0.source.package_id().map(|_| flow_like_types::dispatch::WasmPackageRef {
                version: "1.0.0".into(),
                wasm_hash: "hash".into(),
                wasm_url: "https://example.test/skill.wasm".into(),
                cwasm_url: "https://example.test/skill.cwasm".into(),
                cwasm_checksum: "checksum".into(),
                permissions: None,
            }),
        })
    }
}

fn package_skill() -> flow_like_types::bot_skill::BotSkill {
    use flow_like_types::bot_skill::{BotSkill, SkillSource, SkillStatus};
    BotSkill {
        id: "skill-1".into(),
        bot_id: BOT.into(),
        name: "report".into(),
        description: "Build a report".into(),
        body: "Use the report tool.".into(),
        source: SkillSource::Package {
            id: "com.example.reports".into(),
            version: "1.0.0".into(),
        },
        status: SkillStatus::Active,
        tools: vec!["build_report".into()],
        updated_at: "2026-11-05T00:00:00Z".into(),
    }
}

async fn skill_host(
    executor: Option<Arc<FakeExecutor>>,
    skill: flow_like_types::bot_skill::BotSkill,
) -> (BotToolsHost, Arc<Skills>) {
    let tools = routing::host_tools(executor.is_some(), &[]);
    let host = host_with(executor, tools).await;
    let host = Arc::try_unwrap(host).unwrap_or_else(|_| panic!("host is not shared"));
    let skills = Arc::new(Skills(skill, Mutex::new(Vec::new()), AtomicBool::new(false)));
    (host.with_skills(skills.clone()).with_skill_jobs(skills.clone()), skills)
}

#[flow_like_types::tokio::test]
async fn package_skill_load_discovers_tools_on_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let (host, _) = skill_host(Some(executor.clone()), package_skill()).await;
    let output = parse(&host.run("skill_load", json!({ "name": " report " })).await);
    assert_eq!(output["ran_on"], "executor", "{output}");
    let jobs = executor.jobs.lock().unwrap();
    assert_eq!(jobs.len(), 1);
    let job: ToolJob = serde_json::from_str(&jobs[0].3).unwrap();
    assert_eq!(job.tool, "skill_load");
    let package = job.skill_call.expect("the executor needs the skill package");
    assert_eq!(package.skill.name, "report");
    assert_eq!(package.skill.tools, ["build_report"]);
    assert_eq!(package.package_id, "com.example.reports");
}

#[flow_like_types::tokio::test]
async fn instruction_skill_load_also_runs_on_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let mut skill = package_skill();
    skill.source = flow_like_types::bot_skill::SkillSource::Owner;
    skill.tools.clear();
    let (host, _) = skill_host(Some(executor.clone()), skill).await;
    let output = parse(&host.run("skill_load", json!({ "name": "report" })).await);
    assert_eq!(output["ran_on"], "executor", "{output}");
    let jobs = executor.jobs.lock().unwrap();
    let job: ToolJob = serde_json::from_str(&jobs[0].3).unwrap();
    let skill = job.skill_call.expect("the executor receives the approved instructions");
    assert_eq!(skill.skill.name, "report");
    assert!(skill.package.is_none());
}

#[flow_like_types::tokio::test]
async fn inactive_package_skills_are_hidden_without_dispatching() {
    use flow_like_types::bot_skill::SkillStatus;
    for status in [SkillStatus::Pending, SkillStatus::Disabled] {
        let executor = Arc::new(FakeExecutor::default());
        let mut skill = package_skill();
        skill.status = status;
        let (host, _) = skill_host(Some(executor.clone()), skill).await;
        let output = parse(&host.run("skill_load", json!({ "name": "report" })).await);
        assert_eq!(output["code"], "bot_tool_job_failed", "{output}");
        assert!(executor.jobs.lock().unwrap().is_empty());
    }
}

#[flow_like_types::tokio::test]
async fn skill_tools_without_an_executor_never_fall_back_to_the_api() {
    let (host, skills) = skill_host(None, package_skill()).await;
    for tool in ["skill_load", "skill_propose", "skill_call", "skill_run"] {
        let output = parse(&host.run(tool, json!({ "name": "report" })).await);
        assert_eq!(output["code"], "bot_tool_needs_executor", "{tool}: {output}");
    }
    assert!(skills.1.lock().unwrap().is_empty());
}

#[flow_like_types::tokio::test]
async fn skill_proposals_are_persisted_only_after_executor_validation() {
    let executor = Arc::new(FakeExecutor::default());
    *executor.skill_proposal.lock().unwrap() = Some(flow_like_types::bot_skill::CreateBotSkill {
        name: "validated".into(),
        description: "Validated on the executor".into(),
        body: "Follow this procedure.".into(),
        tools: Vec::new(),
    });
    let (host, skills) = skill_host(Some(executor.clone()), package_skill()).await;
    let output = parse(&host.run("skill_propose", json!({ "name": "raw" })).await);
    assert_eq!(output["ran_on"], "executor", "{output}");
    assert_eq!(skills.1.lock().unwrap()[0].name, "validated");
    executor.fail.store(true, Ordering::SeqCst);
    let failed = parse(&host.run("skill_propose", json!({ "name": "raw" })).await);
    assert_eq!(failed["status"], "error");
    assert_eq!(skills.1.lock().unwrap().len(), 1);
}

#[flow_like_types::tokio::test]
async fn missing_or_failed_skill_proposals_are_never_persisted() {
    let executor = Arc::new(FakeExecutor::default());
    let (host, skills) = skill_host(Some(executor.clone()), package_skill()).await;
    let missing = parse(&host.run("skill_propose", json!({ "name": "report" })).await);
    assert_eq!(missing["code"], "bot_tool_invalid_reply", "{missing}");
    *executor.skill_proposal.lock().unwrap() = Some(flow_like_types::bot_skill::CreateBotSkill {
        name: "report".into(),
        description: "A report".into(),
        body: "Collect the facts.".into(),
        tools: Vec::new(),
    });
    *executor.output.lock().unwrap() = Some(json!({ "status": "error", "error": { "code": "invalid_args" } }));
    let failed = parse(&host.run("skill_propose", json!({ "name": "report" })).await);
    assert_eq!(failed["status"], "error", "{failed}");
    assert!(skills.1.lock().unwrap().is_empty());

    *executor.output.lock().unwrap() = None;
    skills.2.store(true, Ordering::SeqCst);
    let refused = parse(&host.run("skill_propose", json!({ "name": "report" })).await);
    assert_eq!(refused["status"], "error", "{refused}");
    assert!(refused["message"].as_str().unwrap().contains("20 skills"));
    assert!(skills.1.lock().unwrap().is_empty());
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
    assert_eq!(job.bot_id, BOT);
    let apps: Vec<_> = job.credentials.keys().cloned().collect();
    assert_eq!(apps, [DOCS, WORKSPACE]);
    assert_eq!(job.scope.apps.len(), 1);
}

#[flow_like_types::tokio::test]
async fn a_job_carries_only_the_secrets_its_arguments_name() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    host.run(
        "data_sources",
        json!({ "operation": "attach", "attach": { "kind": "postgres", "name": "sales",
            "connection": { "secret": "PG_SALES", "host": "db.example.com", "database": "sales" } } }),
    )
    .await;
    let jobs = executor.jobs.lock().unwrap();
    let job: ToolJob = serde_json::from_str(&jobs[0].3).unwrap();
    let names: Vec<&String> = job.secrets.0.keys().collect();
    assert_eq!(names, ["PG_SALES"]);
    assert_eq!(job.secrets.0["PG_SALES"], "pg-pa55");
}

#[flow_like_types::tokio::test]
async fn the_secret_list_runs_in_the_api_with_names_only() {
    let executor = Arc::new(FakeExecutor::default());
    let host = host_with(Some(executor.clone()), routing::host_tools(true, &[])).await;
    let listed = host.run("secret_list", json!({})).await;
    assert_eq!(parse(&listed)["status"], "ok");
    assert!(listed.contains("PG_SALES") && listed.contains("OTHER"));
    assert!(!listed.contains("pg-pa55") && !listed.contains("other-value"));
    assert!(executor.jobs.lock().unwrap().is_empty());
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
    assert!(output.contains("Execution and saved files are unconfirmed"));
    assert!(!output.contains("Nothing was changed"));
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

const ASKING_CALLS: [(&str, &str); 2] = [
    ("knowledge_drop", "table:pets"),
    ("code_run", "host:example.com"),
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
    let token = super::executor::mint(OWNER, &bot_user(), WORKSPACE, job).unwrap();
    let claims = crate::execution::verify_execution_jwt(&token).unwrap();
    assert_eq!(claims.sub, OWNER);
    assert_eq!(claims.bot_id.as_deref(), Some(BOT));
    assert_eq!(claims.conv_id, None);
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
async fn every_configuration_change_requires_fresh_browser_approval() {
    let tool = "bot_update_configuration";
    let args = json!({"changes": {"instructions": "Ask before sending a message."}});
    let grants = Arc::new(MemoryGrants::default());
    grants.put(tool, "*").await.unwrap();
    let (host, configuration) = configuration_host(grants.clone()).await;
    let (bridge, channel, mut received) = bridge(host).await;
    for approved in [true, false, true] {
        let count_before = configuration.updates.lock().unwrap().len();
        let call = {
            let (bridge, args) = (bridge.clone(), args.clone());
            flow_like_types::tokio::spawn(async move { bridge.call(tool, args).await })
        };
        let request = frame(received.recv().await.unwrap());
        assert_eq!(request["toolName"], tool);
        assert_eq!(request["approvalOnly"], true);
        assert_eq!(request["arguments"], args);
        assert_eq!(configuration.updates.lock().unwrap().len(), count_before);
        let response = if approved {
            json!({"requestId": request["requestId"], "approved": true,
                "result": {"status": "ok", "native_approved": true, "native_remember": true}})
        } else {
            json!({"requestId": request["requestId"], "approved": false,
                "error": "User denied the request."})
        };
        reply(&channel, &request, response).await;
        let output = parse(&call.await.unwrap());
        assert_eq!(output["status"], if approved { "ok" } else { "denied" });
        assert_eq!(configuration.updates.lock().unwrap().len(), count_before + usize::from(approved));
    }
    assert_eq!(grants.list().await.unwrap(), vec![GrantKey::new(tool, "*")]);
    channel.close().await;
}

#[flow_like_types::tokio::test]
async fn flux_runs_configuration_on_the_server_and_keeps_its_authoring_bridge() {
    let (host, configuration) = configuration_host(Arc::new(MemoryGrants::default())).await;
    let channel = InProcessChannel::register(
        flow_like_types::channel::new_request_id(),
        Duration::from_secs(30),
    )
    .await;
    let (frames, mut received) = mpsc::unbounded_channel();
    let bridge = Arc::new(
        ServerPlatformBridge::orchestrator(channel.clone(), frames).with_bot_tools(Some(host)),
    );
    for tool in ["bot_configuration", "bot_models"] {
        assert_eq!(parse(&bridge.call(tool, json!({})).await)["status"], "ok");
        assert!(received.try_recv().is_err());
    }
    let call = {
        let bridge = bridge.clone();
        flow_like_types::tokio::spawn(async move {
            bridge.call("bot_update_configuration", json!({"changes": {"name": "Flow helper"}})).await
        })
    };
    let request = frame(received.recv().await.unwrap());
    assert_eq!(request["approvalOnly"], true);
    reply(&channel, &request, json!({
        "requestId": request["requestId"], "approved": true,
        "result": {"status": "ok", "native_approved": true},
    })).await;
    assert_eq!(parse(&call.await.unwrap())["status"], "ok");
    assert_eq!(configuration.updates.lock().unwrap().len(), 1);

    let authoring = {
        let bridge = bridge.clone();
        flow_like_types::tokio::spawn(async move {
            bridge.call("list_apps", json!({})).await
        })
    };
    let request = frame(received.recv().await.unwrap());
    assert_eq!(request["toolName"], "list_apps");
    assert!(request.get("approvalOnly").is_none());
    reply(&channel, &request, json!({
        "requestId": request["requestId"], "approved": true, "result": {"apps": []},
    })).await;
    assert_eq!(parse(&authoring.await.unwrap())["status"], "ok");
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

fn image_profile() -> flow_like::profile::Profile {
    let bit = flow_like::bit::Bit {
        id: "my-image".into(),
        bit_type: flow_like::bit::BitTypes::ImageGeneration,
        parameters: json!({"provider": {
            "provider_name": "custom:openai", "model_id": "gpt-image-1",
            "params": {"api_key": "sk-private-key"}
        }}),
        ..Default::default()
    };
    let mut profile = flow_like::profile::Profile::default();
    profile.custom_bits = vec![flow_like::profile::ProfileCustomBit(bit)];
    profile
}

fn image_bits(image_model: Option<&str>) -> Arc<super::images::ImageBits> {
    let state = Arc::new(flow_like::state::FlowLikeState::new(
        flow_like::state::FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(
            InMemory::new(),
        ))),
        flow_like::utils::http::HTTPClient::new_without_refetch(),
    ));
    Arc::new(super::images::ImageBits::new(
        Arc::new(image_profile()),
        state,
        image_model.map(str::to_string),
    ))
}

async fn image_host(
    executor: Option<Arc<FakeExecutor>>,
    images: Option<Arc<super::images::ImageBits>>,
) -> Arc<BotToolsHost> {
    let access = Arc::new(Access::new(&[WORKSPACE, DOCS]).await);
    let mut host = BotToolsHost::new(TurnDeps {
        owner: OWNER.into(),
        bot: bot_user(),
        turn_id: TURN.into(),
        workspace: WORKSPACE.into(),
        host_tools: routing::host_tools(executor.is_some(), &[]),
        directory: Arc::new(Directory { scope: job_scope() }),
        access,
        models: TurnModels {
            source: Arc::new(NoModels),
            chat: None,
            embedding: None,
        },
        executor: executor.map(|executor| executor as Arc<dyn ToolExecutor>),
        grants: Arc::new(MemoryGrants::default()),
        secrets: Arc::new(JobSecrets::default()),
        connectors: None,
    });
    if let Some(images) = images {
        host = host.with_images(images);
    }
    Arc::new(host)
}

#[test]
fn image_generate_is_an_executor_tool() {
    assert!(!routing::host_tools(false, &[]).runs("image_generate"));
    assert!(routing::host_tools(true, &[]).runs("image_generate"));
    assert_eq!(
        routing::route("image_generate", &json!({}), true),
        Some(routing::Route::Executor)
    );
    assert!(matches!(
        routing::route("image_generate", &json!({}), false),
        Some(routing::Route::Refuse(_))
    ));
}

#[flow_like_types::tokio::test]
async fn the_job_carries_the_owners_image_bit_to_the_executor_and_counts_saved_images() {
    let executor = Arc::new(FakeExecutor::default());
    let bits = image_bits(Some("my-image"));
    let host = image_host(Some(executor.clone()), Some(bits.clone())).await;
    let output = parse(
        &host
            .run("image_generate", json!({ "prompt": "a lighthouse" }))
            .await,
    );
    assert_eq!(output["ran_on"], "executor", "{output}");
    assert!(!output.to_string().contains("sk-private-key"));
    let jobs = executor.jobs.lock().unwrap();
    let job: ToolJob = serde_json::from_str(&jobs[0].3).unwrap();
    let image = job
        .models
        .image
        .as_ref()
        .expect("the job names the image model");
    assert_eq!(image.bit["id"], "my-image");
    assert_eq!(image.bit["parameters"]["provider"]["params"]["api_key"], "sk-private-key");
    assert_eq!(image.left, 8);
    assert!(!format!("{:?}", job.models).contains("sk-private-key"));
    drop(jobs);
    bits.record(&json!({ "status": "ok", "saved": [{}, {}, {}] }).to_string());
    bits.record(&json!({ "status": "error", "saved": [{}] }).to_string());
    assert_eq!(bits.left(), 5);
}

#[flow_like_types::tokio::test]
async fn other_jobs_never_carry_the_image_bit() {
    let executor = Arc::new(FakeExecutor::default());
    let host = image_host(Some(executor.clone()), Some(image_bits(Some("my-image")))).await;
    host.run("file_write", json!({ "path": "bot/a.md", "content": "x" }))
        .await;
    let jobs = executor.jobs.lock().unwrap();
    assert!(!jobs[0].3.contains("sk-private-key"));
}

#[flow_like_types::tokio::test]
async fn an_image_job_without_a_usable_model_is_refused_before_the_executor() {
    let executor = Arc::new(FakeExecutor::default());
    let host = image_host(Some(executor.clone()), Some(image_bits(None))).await;
    let output = parse(&host.run("image_generate", json!({ "prompt": "a" })).await);
    assert_eq!(output["status"], "error");
    assert!(executor.jobs.lock().unwrap().is_empty());
}

#[flow_like_types::tokio::test]
async fn without_an_executor_image_generate_is_refused_plainly() {
    let host = image_host(None, Some(image_bits(Some("my-image")))).await;
    let output = parse(&host.run("image_generate", json!({ "prompt": "a" })).await);
    assert_eq!(output["code"], "bot_tool_needs_executor");
}
