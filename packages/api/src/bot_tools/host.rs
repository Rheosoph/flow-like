use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flow_like::bit::Bit;
use flow_like::flow::copilot::tool_authorization::AuthorizationInference;
use flow_like::flow::copilot::tool_grants::{
    GrantCheck, GrantStore, HeadlessAnswer, check_call, documents_expensive_granted,
    expensive_documents_request, headless_answer_with_inference, record_answer, take_once,
};
use flow_like::flow::copilot::tool_spec::{ToolEffect, resolve_tool_approval, resolve_tool_effect};
use flow_like::flow::copilot::{HostTools, find_bot_tool_spec, tool_outcome_output};
use flow_like_bot_tools::MemorySessions;
use flow_like_bot_tools_host::{
    AppAccess, HostEnv, JobConnector, JobModels, JobScope, JobSecrets, ModelSource, SecretSource,
    ToolJob,
};
use flow_like_types::tokio::sync::OnceCell;
use flow_like_types::{bot_message::ApprovalRequest, tool_grant::ApprovalDecision};
use serde_json::{Value, json};

use crate::middleware::jwt::BotUser;

use super::{
    executor::ToolExecutor,
    routing::{Route, is_bot_tool, route},
};

const MAX_EXECUTOR_JOBS_PER_TURN: usize = 30;
const IMAGE_TOOL: &str = "image_generate";
const EXECUTOR_TIMEOUT: Duration = Duration::from_secs(300);

/// What the owner holds, resolved when the first tool runs: which apps the bot reaches with
/// which rights, and a credential scoped to each of them for the executor.
#[async_trait]
pub(crate) trait AppDirectory: Send + Sync {
    async fn scope(&self) -> Result<JobScope, String>;
    async fn credential(&self, app_id: &str) -> Result<Value, String>;
}

/// The models a tool may use during one turn, and the Bits an executor builds its own from.
pub(crate) struct TurnModels {
    pub source: Arc<dyn ModelSource>,
    pub chat: Option<Bit>,
    pub embedding: Option<Bit>,
    pub vision: Option<Bit>,
}

pub(crate) struct TurnDeps {
    pub owner: String,
    pub bot: BotUser,
    pub turn_id: String,
    pub workspace: String,
    pub host_tools: HostTools,
    pub directory: Arc<dyn AppDirectory>,
    pub access: Arc<dyn AppAccess>,
    pub models: TurnModels,
    pub executor: Option<Arc<dyn ToolExecutor>>,
    pub grants: Arc<dyn GrantStore>,
    pub secrets: Arc<dyn SecretSource>,
    pub connectors: Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>>,
}

/// One bot turn's bot tools on the server: the in-process set runs here, the rest goes to the
/// executor.
pub(crate) struct BotToolsHost {
    deps: TurnDeps,
    scope: OnceCell<JobScope>,
    sessions: Arc<MemorySessions>,
    executor_jobs: AtomicUsize,
    parked: Mutex<Option<(ApprovalRequest, Option<String>)>>,
    inference: Mutex<Option<Arc<dyn AuthorizationInference>>>,
    approval_check: flow_like_types::tokio::sync::Mutex<()>,
    notifier: Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>>,
    people: Option<Arc<dyn flow_like_bot_tools::PeopleBackend>>,
    user_context: Option<Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>>,
    configuration: Option<Arc<dyn flow_like_bot_tools::configuration::ConfigurationBackend>>,
    crew: Option<Arc<dyn flow_like_bot_tools::crew::CrewBackend>>,
    scheduler: Option<Arc<dyn flow_like_bot_tools::Scheduler>>,
    skills: Option<Arc<dyn flow_like_bot_tools::SkillHost>>,
    skill_jobs: Option<Arc<dyn super::skills::SkillJobs>>,
    mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
    images: Option<Arc<super::images::ImageBits>>,
    apps: Option<Arc<super::apps::AppTools>>,
}

fn refusal(tool: &str, code: &str, message: &str) -> String {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
        .to_string()
}

fn checked_skill_proposal(
    reply: &flow_like_bot_tools_host::ToolReply,
) -> Result<Option<&flow_like_types::bot_skill::CreateBotSkill>, &'static str> {
    let output = serde_json::from_str::<Value>(&reply.output).unwrap_or(Value::Null);
    match output["status"].as_str() {
        Some("ok") => reply
            .skill_proposal
            .as_ref()
            .filter(|proposal| output["proposed"] == true && output["name"] == proposal.name)
            .map(Some)
            .ok_or("The executor did not return a validated skill proposal. Nothing was saved."),
        Some("error" | "needs_approval") => Ok(None),
        _ => Err("The executor returned an invalid skill proposal result. Nothing was saved."),
    }
}

fn needs_cost_approval(output: &str) -> bool {
    serde_json::from_str::<Value>(output)
        .ok()
        .is_some_and(|value| value["code"] == flow_like_bot_tools::error::code::NEEDS_APPROVAL)
}

fn cost_message(output: &str) -> String {
    serde_json::from_str::<Value>(output)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| "This document call is above the cost limits.".to_string())
}

fn private_model(bit: &Bit) -> bool {
    bit.try_to_provider()
        .is_some_and(|provider| provider.provider_name.starts_with("custom:"))
        || bit
            .try_to_embedding()
            .is_some_and(|embedding| embedding.provider.provider_name.starts_with("custom:"))
}

fn bit_json(bit: &Option<Bit>) -> Option<Value> {
    bit.as_ref()
        .filter(|bit| !private_model(bit))
        .and_then(|bit| serde_json::to_value(bit).ok())
}

impl BotToolsHost {
    pub(crate) fn new(deps: TurnDeps) -> Self {
        Self {
            deps,
            scope: OnceCell::new(),
            sessions: Arc::new(MemorySessions::default()),
            executor_jobs: AtomicUsize::new(0),
            parked: Mutex::new(None),
            inference: Mutex::new(None),
            approval_check: flow_like_types::tokio::sync::Mutex::new(()),
            notifier: None,
            people: None,
            user_context: None,
            configuration: None,
            crew: None,
            scheduler: None,
            skills: None,
            skill_jobs: None,
            mail: None,
            images: None,
            apps: None,
        }
    }

    pub(crate) fn set_inference(&self, inference: Arc<dyn AuthorizationInference>) {
        *self
            .inference
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(inference);
    }

    pub(crate) fn with_user_context(
        mut self,
        context: Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>,
    ) -> Self {
        self.user_context = Some(context);
        self
    }

    pub(crate) fn with_configuration(
        mut self,
        configuration: Arc<dyn flow_like_bot_tools::configuration::ConfigurationBackend>,
    ) -> Self {
        self.configuration = Some(configuration);
        self
    }

    /// Flux's management of its owner's bots; other bots never get one.
    pub(crate) fn with_crew(
        mut self,
        crew: Arc<dyn flow_like_bot_tools::crew::CrewBackend>,
    ) -> Self {
        self.crew = Some(crew);
        self
    }

    pub(crate) fn with_images(mut self, images: Arc<super::images::ImageBits>) -> Self {
        self.images = Some(images);
        self
    }

    pub(crate) fn with_mail(
        mut self,
        mail: Arc<dyn flow_like_bot_tools::email::MailBackend>,
    ) -> Self {
        self.mail = Some(mail);
        self
    }

    pub(crate) fn with_skill_jobs(mut self, jobs: Arc<dyn super::skills::SkillJobs>) -> Self {
        self.skill_jobs = Some(jobs);
        self
    }

    /// What an executor needs to discover or run a package skill's tools: the active skill
    /// and the package the workspace pins.
    async fn skill_job(
        &self,
        tool: &str,
        args: &Value,
    ) -> Result<Option<flow_like_bot_tools_host::SkillCallJob>, String> {
        let name_arg = match tool {
            "skill_load" => "name",
            "skill_call" | "skill_run" => "skill",
            _ => return Ok(None),
        };
        let jobs = self
            .skill_jobs
            .as_ref()
            .ok_or_else(|| "This server cannot run package skills".to_string())?;
        let name = args
            .get(name_arg)
            .and_then(Value::as_str)
            .unwrap_or_default();
        jobs.job(name.trim()).await.map(Some)
    }

    pub(crate) fn with_skills(mut self, skills: Arc<dyn flow_like_bot_tools::SkillHost>) -> Self {
        self.skills = Some(skills);
        self
    }

    pub(crate) fn with_scheduler(
        mut self,
        scheduler: Arc<dyn flow_like_bot_tools::Scheduler>,
    ) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub(crate) fn with_notifier(
        mut self,
        notifier: Arc<dyn flow_like_bot_tools::OwnerNotifier>,
    ) -> Self {
        self.notifier = Some(notifier);
        self
    }

    pub(crate) fn with_people(
        mut self,
        people: Arc<dyn flow_like_bot_tools::PeopleBackend>,
    ) -> Self {
        self.people = Some(people);
        self
    }

    /// Lets this turn use apps on the server; only a headless turn does, an open chat runs them
    /// in its window.
    pub(crate) fn with_apps(mut self, apps: Arc<super::apps::AppTools>) -> Self {
        self.apps = Some(apps);
        self
    }

    fn app_tools(&self, tool: &str) -> Option<&Arc<super::apps::AppTools>> {
        self.apps
            .as_ref()
            .filter(|_| super::apps::is_app_tool(tool))
    }

    pub(crate) fn handles(&self, tool: &str) -> bool {
        self.app_tools(tool).is_some() || (is_bot_tool(tool) && self.deps.host_tools.runs(tool))
    }

    async fn scope(&self) -> Result<&JobScope, String> {
        self.scope
            .get_or_try_init(|| self.deps.directory.scope())
            .await
    }

    /// Runs `tool` where it belongs and returns the text for the model.
    pub(crate) async fn run(&self, tool: &str, args: Value) -> String {
        self.run_allowing(tool, args, false).await
    }

    async fn run_allowing(&self, tool: &str, args: Value, expensive_documents: bool) -> String {
        if let Some(apps) = self.app_tools(tool) {
            return apps.run(tool, args).await;
        }
        match route(tool, &args, self.deps.executor.is_some()) {
            None => refusal(
                tool,
                "bot_tool_unknown",
                "This host does not run this tool.",
            ),
            Some(Route::Refuse(message)) => refusal(tool, "bot_tool_needs_executor", message),
            Some(Route::InProcess) => self.run_in_process(tool, args, expensive_documents).await,
            Some(Route::Executor) => self.run_on_executor(tool, args, expensive_documents).await,
        }
    }

    /// Whether a standing grant or the owner's one-time answer already covers an asking call, so
    /// an open chat need not ask.
    pub(crate) async fn covers(&self, tool: &str, args: &Value) -> bool {
        check_call(self.deps.grants.as_ref(), tool, args, None)
            .await
            .is_ok_and(GrantCheck::runs)
    }

    /// Stores the owner's "don't ask again" for this call.
    pub(crate) async fn remember(&self, tool: &str, args: &Value) {
        let stored = record_answer(
            self.deps.grants.as_ref(),
            tool,
            args,
            None,
            ApprovalDecision::Always,
        )
        .await;
        if let Err(error) = stored {
            tracing::warn!(%error, tool, "[bot_tools] the owner's standing approval was not stored");
        }
    }

    /// The approval card of the call that parked this turn, once there is one.
    pub(crate) fn parked(&self) -> Option<ApprovalRequest> {
        self.parked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|(card, _)| card.clone())
    }

    pub(crate) fn parked_recovery_call_id(&self) -> Option<String> {
        self.parked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(|(_, call_id)| call_id.clone())
    }

    fn park(&self, tool: &str, card: ApprovalRequest) -> String {
        self.parked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert_with(|| {
                (
                    card,
                    flow_like::flow::copilot::run_recovery::current_recovery_call_id(),
                )
            });
        json!({
            "status": "waiting",
            "tool": tool,
            "code": "bot_tool_waiting_for_approval",
            "retryable": false,
            "message": "Your owner was asked to approve this call. This turn pauses and continues after their answer. Call no further tools and write nothing more now."
        })
        .to_string()
    }

    async fn expensive_documents_allowed(&self, tool: &str, args: &Value) -> bool {
        let spec = flow_like_bot_tools::spec(tool);
        if !spec.is_some_and(|spec| matches!(spec.module, "documents" | "excel")) {
            return false;
        }
        let grants = self.deps.grants.as_ref();
        documents_expensive_granted(grants).await.unwrap_or(false)
            || take_once(grants, tool, args).await.unwrap_or(false)
    }

    /// The headless path: nobody can approve right now, so a call that asks runs under a standing
    /// grant (or the owner's one-time answer) and otherwise parks the turn behind an approval
    /// card. A tool that changes anything ends the turn's retries.
    pub(crate) async fn run_unattended(
        &self,
        mutating: &AtomicBool,
        tool: &str,
        args: Value,
    ) -> String {
        let approval_check = self.approval_check.lock().await;
        if self.parked().is_some() {
            return self.park_notice(tool);
        }
        let mut expensive = false;
        match find_bot_tool_spec(tool).map(|spec| resolve_tool_approval(&spec, &args)) {
            Some(approval) if approval.kind != "none" => {
                let inference = self
                    .inference
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                match headless_answer_with_inference(
                    self.deps.grants.as_ref(),
                    &self.deps.turn_id,
                    tool,
                    &args,
                    None,
                    &approval.description,
                    inference.as_deref(),
                )
                .await
                {
                    HeadlessAnswer::Run => {}
                    HeadlessAnswer::Refuse(message) => {
                        return refusal(tool, "bot_tool_refused_by_owner", &message);
                    }
                    HeadlessAnswer::Park(card) => return self.park(tool, card),
                }
            }
            _ => expensive = self.expensive_documents_allowed(tool, &args).await,
        }
        drop(approval_check);
        if flow_like_bot_tools::spec(tool).is_some_and(|spec| spec.mutating)
            || (self.app_tools(tool).is_some()
                && find_bot_tool_spec(tool)
                    .is_some_and(|spec| resolve_tool_effect(&spec, &args) != ToolEffect::ReadOnly))
        {
            mutating.store(true, Ordering::SeqCst);
        }
        let output = self.run_allowing(tool, args.clone(), expensive).await;
        if !expensive && needs_cost_approval(&output) {
            let card = expensive_documents_request(
                &self.deps.turn_id,
                tool,
                &args,
                &cost_message(&output),
            );
            return self.park(tool, card);
        }
        output
    }

    fn park_notice(&self, tool: &str) -> String {
        self.park(tool, self.parked().expect("a parked turn has a card"))
    }

    async fn run_in_process(&self, tool: &str, args: Value, expensive_documents: bool) -> String {
        // Configuration and participant context do not read apps or need a workspace.
        let empty_scope = JobScope {
            storage: false,
            data_connections: false,
            classification: flow_like_types::data_classification::DataClassification::Internal,
            apps: Vec::new(),
        };
        let scope = if flow_like_bot_tools::configuration::is_settings_tool(tool)
            || flow_like_bot_tools::user_context::is_user_context_tool(tool)
        {
            &empty_scope
        } else {
            match self.scope().await {
                Ok(scope) => scope,
                Err(message) => return refusal(tool, "bot_scope_unavailable", &message),
            }
        };
        let mut env = HostEnv::new(
            &self.deps.workspace,
            scope,
            self.deps.access.clone(),
            self.deps.models.source.clone(),
        )
        .with_sessions(self.sessions.clone())
        .with_object_writes(false)
        .with_expensive_documents(expensive_documents)
        .with_secrets(self.deps.secrets.clone());
        if let Some(connectors) = &self.deps.connectors {
            env = env.with_connectors(connectors.clone());
        }
        if let Some(notifier) = &self.notifier {
            env = env.with_notifier(notifier.clone());
        }
        if let Some(context) = &self.user_context {
            env = env.with_user_context(context.clone());
        }
        if let Some(configuration) = &self.configuration {
            env = env.with_configuration(configuration.clone());
        }
        if let Some(crew) = &self.crew {
            env = env.with_crew(crew.clone());
        }
        if let Some(people) = &self.people {
            env = env.with_people(people.clone());
        }
        if let Some(scheduler) = &self.scheduler {
            env = env.with_scheduler(scheduler.clone());
        }
        if let Some(mail) = &self.mail {
            env = env.with_mail(mail.clone());
        }
        tool_outcome_output(&flow_like_bot_tools::run(tool, args, &env).await)
    }

    async fn build_job(
        &self,
        tool: &str,
        args: Value,
        expensive_documents: bool,
    ) -> Result<String, String> {
        let scope = self.scope().await?.clone();
        let workspace = self.deps.workspace.clone();
        let mut credentials = BTreeMap::new();
        let granted = std::iter::once(workspace.as_str()).chain(
            scope
                .apps
                .iter()
                .filter(|app| app.files || app.tables)
                .map(|app| app.app_id.as_str()),
        );
        for app_id in granted {
            let credential = self.deps.directory.credential(app_id).await?;
            credentials.insert(app_id.to_string(), credential);
        }
        let image = match (&self.images, tool == IMAGE_TOOL) {
            (Some(images), true) => Some(images.job_image().await?),
            _ => None,
        };
        let args_for_connectors = args.clone();
        let mut secrets = BTreeMap::new();
        for name in flow_like_bot_tools::secrets::referenced_secrets(&args) {
            let value = self
                .deps
                .secrets
                .secret(&name)
                .await
                .map_err(|error| error.to_string())?;
            if let Some(value) = value {
                secrets.insert(name, value);
            }
        }
        let job = ToolJob {
            tool: tool.to_string(),
            args,
            owner: self.deps.owner.clone(),
            bot_id: self.deps.bot.bot_id.clone(),
            turn_id: self.deps.turn_id.clone(),
            workspace,
            scope,
            credentials,
            models: JobModels {
                chat: bit_json(&self.deps.models.chat),
                embedding: bit_json(&self.deps.models.embedding),
                vision: bit_json(&self.deps.models.vision),
                image,
            },
            allow_expensive_documents: expensive_documents,
            secrets: JobSecrets(secrets),
            connectors: self.job_connectors(tool, &args_for_connectors).await?,
            skill_call: self.skill_job(tool, &args_for_connectors).await?,
        };
        serde_json::to_string(&job).map_err(|error| format!("Cannot encode the tool job: {error}"))
    }

    /// The connector a connector call uses, with the sign-in resolved for this job only. A
    /// call takes one connector; a schema lookup without one takes every MCP server.
    async fn job_connectors(&self, tool: &str, args: &Value) -> Result<Vec<JobConnector>, String> {
        use flow_like_bot_tools::services::{
            CONNECTOR_CALL, CONNECTOR_REQUEST, CONNECTOR_TOOLS, ConnectorEntry,
        };
        use flow_like_types::bot_connector::ConnectorKind;
        if ![CONNECTOR_TOOLS, CONNECTOR_CALL, CONNECTOR_REQUEST].contains(&tool) {
            return Ok(Vec::new());
        }
        let Some(source) = &self.deps.connectors else {
            return Ok(Vec::new());
        };
        let wanted = args
            .get("connector")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|wanted| !wanted.is_empty());
        let chosen = |entry: &ConnectorEntry| match wanted {
            Some(wanted) => {
                entry.rule.id == wanted || entry.rule.label.eq_ignore_ascii_case(wanted)
            }
            None => tool == CONNECTOR_TOOLS && entry.kind == ConnectorKind::Mcp,
        };
        if tool != CONNECTOR_TOOLS && !source.budget().take() {
            return Err("At most 20 connector calls per turn".to_string());
        }
        let mut connectors = Vec::new();
        for entry in source
            .entries()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(chosen)
        {
            let credential = source
                .credential(&entry.rule.id)
                .await
                .map_err(|error| error.to_string())?;
            connectors.push(JobConnector {
                authorization: credential.header().map(str::to_string),
                entry,
            });
        }
        Ok(connectors)
    }

    async fn run_on_executor(&self, tool: &str, args: Value, expensive_documents: bool) -> String {
        let Some(executor) = &self.deps.executor else {
            return refusal(
                tool,
                "bot_tool_needs_executor",
                "No executor is configured.",
            );
        };
        if self.executor_jobs.fetch_add(1, Ordering::SeqCst) >= MAX_EXECUTOR_JOBS_PER_TURN {
            return refusal(
                tool,
                "bot_tool_turn_limit",
                &format!(
                    "This turn already ran {MAX_EXECUTOR_JOBS_PER_TURN} heavy tool calls. Answer with what you have."
                ),
            );
        }
        let job = match self.build_job(tool, args, expensive_documents).await {
            Ok(job) => job,
            Err(message) => return refusal(tool, "bot_tool_job_failed", &message),
        };
        let started = std::time::Instant::now();
        let reply = executor
            .run(
                &self.deps.owner,
                &self.deps.bot,
                &self.deps.workspace,
                job,
                EXECUTOR_TIMEOUT,
            )
            .await;
        let micros = started.elapsed().as_micros().min(i64::MAX as u128) as i64;
        tracing::info!(
            tool,
            turn_id = %self.deps.turn_id,
            owner = %self.deps.owner,
            duration_ms = micros / 1000,
            compute_cost_micro_dollars = crate::compute_cost::compute_cost_micro_dollars(micros, 1),
            "[bot_tools] executor tool job finished"
        );
        match reply {
            Ok(reply) => {
                if tool == "skill_propose" {
                    match checked_skill_proposal(&reply) {
                        Ok(Some(proposal)) => {
                            let Some(skills) = &self.skills else {
                                return refusal(
                                    tool,
                                    "failed",
                                    "This server does not keep skills for the bot",
                                );
                            };
                            if let Err(error) = skills.propose(proposal.clone()).await {
                                return refusal(tool, "failed", &error.to_string());
                            }
                        }
                        Ok(None) => {}
                        Err(message) => return refusal(tool, "bot_tool_invalid_reply", message),
                    }
                }
                if let (Some(images), true) = (&self.images, tool == IMAGE_TOOL) {
                    images.record(&reply.output);
                }
                reply.output
            }
            Err(error) => {
                tracing::warn!(tool, turn_id = %self.deps.turn_id, %error, "[bot_tools] executor tool job failed");
                refusal(tool, error.code, error.message)
            }
        }
    }
}
