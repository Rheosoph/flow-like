use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flow_like::bit::Bit;
use flow_like::flow::copilot::tool_grants::{
    GrantCheck, GrantStore, HeadlessAnswer, check_call, documents_expensive_granted,
    expensive_documents_request, headless_answer, record_answer, take_once,
};
use flow_like::flow::copilot::tool_spec::resolve_tool_approval;
use flow_like::flow::copilot::{HostTools, find_bot_tool_spec, tool_outcome_output};
use flow_like_bot_tools::MemorySessions;
use flow_like_bot_tools_host::{AppAccess, HostEnv, JobModels, JobScope, ModelSource, ToolJob};
use flow_like_types::tokio::sync::OnceCell;
use flow_like_types::{bot_message::ApprovalRequest, tool_grant::ApprovalDecision};
use serde_json::{Value, json};

use super::{
    executor::ToolExecutor,
    routing::{Route, is_bot_tool, route},
};

const MAX_EXECUTOR_JOBS_PER_TURN: usize = 30;
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
}

pub(crate) struct TurnDeps {
    pub owner: String,
    pub turn_id: String,
    pub workspace: String,
    pub host_tools: HostTools,
    pub directory: Arc<dyn AppDirectory>,
    pub access: Arc<dyn AppAccess>,
    pub models: TurnModels,
    pub executor: Option<Arc<dyn ToolExecutor>>,
    pub grants: Arc<dyn GrantStore>,
}

/// One bot turn's bot tools on the server: the in-process set runs here, the rest goes to the
/// executor.
pub(crate) struct BotToolsHost {
    deps: TurnDeps,
    scope: OnceCell<JobScope>,
    sessions: Arc<MemorySessions>,
    executor_jobs: AtomicUsize,
    parked: Mutex<Option<ApprovalRequest>>,
    notifier: Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>>,
    scheduler: Option<Arc<dyn flow_like_bot_tools::Scheduler>>,
    mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
}

fn refusal(tool: &str, code: &str, message: &str) -> String {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
        .to_string()
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
            notifier: None,
            scheduler: None,
            mail: None,
        }
    }

    pub(crate) fn with_mail(
        mut self,
        mail: Arc<dyn flow_like_bot_tools::email::MailBackend>,
    ) -> Self {
        self.mail = Some(mail);
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

    pub(crate) fn handles(&self, tool: &str) -> bool {
        is_bot_tool(tool) && self.deps.host_tools.runs(tool)
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
            .clone()
    }

    fn park(&self, tool: &str, card: ApprovalRequest) -> String {
        self.parked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(card);
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
        if self.parked().is_some() {
            return self.park_notice(tool);
        }
        let mut expensive = false;
        match find_bot_tool_spec(tool).map(|spec| resolve_tool_approval(&spec, &args)) {
            Some(approval) if approval.kind != "none" => {
                match headless_answer(
                    self.deps.grants.as_ref(),
                    &self.deps.turn_id,
                    tool,
                    &args,
                    None,
                    &approval.description,
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
        if flow_like_bot_tools::spec(tool).is_some_and(|spec| spec.mutating) {
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
        let scope = match self.scope().await {
            Ok(scope) => scope,
            Err(message) => return refusal(tool, "bot_scope_unavailable", &message),
        };
        let mut env = HostEnv::new(
            &self.deps.workspace,
            scope,
            self.deps.access.clone(),
            self.deps.models.source.clone(),
        )
        .with_sessions(self.sessions.clone())
        .with_object_writes(false)
        .with_expensive_documents(expensive_documents);
        if let Some(notifier) = &self.notifier {
            env = env.with_notifier(notifier.clone());
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
        let job = ToolJob {
            tool: tool.to_string(),
            args,
            owner: self.deps.owner.clone(),
            turn_id: self.deps.turn_id.clone(),
            workspace,
            scope,
            credentials,
            models: JobModels {
                chat: bit_json(&self.deps.models.chat),
                embedding: bit_json(&self.deps.models.embedding),
            },
            allow_expensive_documents: expensive_documents,
        };
        serde_json::to_string(&job).map_err(|error| format!("Cannot encode the tool job: {error}"))
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
                &self.deps.turn_id,
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
            Ok(reply) => reply.output,
            Err(message) => {
                tracing::warn!(tool, turn_id = %self.deps.turn_id, %message, "[bot_tools] executor tool job failed");
                refusal(
                    tool,
                    "bot_tool_executor_failed",
                    "The executor could not run this tool. Nothing was changed unless the result says so.",
                )
            }
        }
    }
}
