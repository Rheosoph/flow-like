//! The server's calls to connected calendars. Fetching a provider's answer and parsing it runs on
//! the executor as an internal `calendar_job` tool job; this process keeps the trusted half:
//! which calendars a bot reads, the policy and approvals (core decides them before a tool runs),
//! the credential resolved for each call and handed to that one job, the bot's own event rows,
//! and the checks of every answer (`flow_like_bot_tools::calendar`). Without an executor,
//! connected calendars are refused and the bot's own calendar keeps working.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flow_like_bot_tools::calendar::{CALENDAR_JOB, CalendarCall, CalendarJobs, JobCalendar};
use flow_like_bot_tools::error::{ToolError, ToolResult, code};
use flow_like_bot_tools_host::{JobConnector, JobScope, ToolJob};
use flow_like_types::data_classification::DataClassification;
use flow_like_types::tokio::sync::OnceCell;
use serde_json::Value;

use super::executor::ToolExecutor;
use crate::{credentials::CredentialsAccess, middleware::jwt::BotUser, state::AppState};

const JOB_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_PREFIX: &str = "calendar-";
const NO_EXECUTOR: &str = "This server has no executor configured, so it cannot reach connected calendars; the bot's own calendar works";

/// The run, workspace and workspace credential one calendar job carries.
pub(crate) struct JobIdentity {
    pub principal: BotUser,
    pub workspace: String,
    /// A read-only credential for the workspace: the executor starts every job in it.
    pub credential: Value,
}

/// Where calendar jobs get their identity.
#[async_trait]
pub(crate) trait JobBinding: Send + Sync {
    async fn bind(&self) -> Result<JobIdentity, String>;
}

/// The run a calendar job belongs to: the turn's own, or outside a turn a fresh
/// `calendar-<uuid>` run per job, like `connector-refresh-…` and `skill-import-…` jobs.
fn principal_for(turn: Option<&BotUser>, bot_id: &str) -> BotUser {
    turn.cloned().unwrap_or_else(|| BotUser {
        bot_id: bot_id.to_string(),
        turn_id: format!("{RUN_PREFIX}{}", uuid::Uuid::new_v4().simple()),
        conv_id: None,
    })
}

/// The binding of a bot's calendar jobs on this server: its workspace (made on first use, as
/// for a connector refresh) with the owner's read-only credential.
pub(crate) struct ServerBinding {
    state: AppState,
    owner: String,
    bot_id: String,
    turn: Option<BotUser>,
    workspace: OnceCell<String>,
}

impl ServerBinding {
    fn new(state: &AppState, owner: &str, bot_id: &str, turn: Option<(BotUser, &str)>) -> Self {
        let workspace = OnceCell::new();
        let turn = turn.map(|(principal, turn_workspace)| {
            if !turn_workspace.is_empty() {
                let _ = workspace.set(turn_workspace.to_string());
            }
            principal
        });
        Self {
            state: state.clone(),
            owner: owner.to_string(),
            bot_id: bot_id.to_string(),
            turn,
            workspace,
        }
    }
}

#[async_trait]
impl JobBinding for ServerBinding {
    async fn bind(&self) -> Result<JobIdentity, String> {
        let workspace = self
            .workspace
            .get_or_try_init(|| async {
                crate::routes::bots::workspace::ensure(&self.state, &self.owner, &self.bot_id)
                    .await
                    .map(|ensured| ensured.app_id)
                    .map_err(|error| {
                        format!(
                            "Connected calendars need the bot's workspace, which is not available: {error}"
                        )
                    })
            })
            .await?
            .clone();
        let credential = self
            .state
            .scoped_credentials(&self.owner, &workspace, CredentialsAccess::ReadAppContent)
            .await
            .map_err(|error| {
                format!("Scoping the workspace credential for a calendar job failed: {error}")
            })?
            .into_shared_credentials();
        let credential = serde_json::to_value(credential).map_err(|error| {
            format!("Encoding the workspace credential for a calendar job failed: {error}")
        })?;
        Ok(JobIdentity {
            principal: principal_for(self.turn.as_ref(), &self.bot_id),
            workspace,
            credential,
        })
    }
}

/// The tool job of one calendar call: no app in scope, one read-only credential for the
/// workspace, the call's connectors with the sign-in resolved for this job only.
pub(crate) fn calendar_job(
    owner: &str,
    identity: &JobIdentity,
    call: &CalendarCall,
    connectors: Vec<JobCalendar>,
) -> Result<ToolJob, String> {
    Ok(ToolJob {
        tool: CALENDAR_JOB.to_string(),
        args: serde_json::to_value(call)
            .map_err(|error| format!("Cannot encode the calendar {} call: {error}", call.name()))?,
        owner: owner.to_string(),
        bot_id: identity.principal.bot_id.clone(),
        turn_id: identity.principal.turn_id.clone(),
        workspace: identity.workspace.clone(),
        scope: JobScope {
            storage: false,
            data_connections: false,
            classification: DataClassification::Confidential,
            apps: Vec::new(),
        },
        credentials: BTreeMap::from([(identity.workspace.clone(), identity.credential.clone())]),
        models: Default::default(),
        allow_expensive_documents: false,
        secrets: Default::default(),
        connectors: connectors
            .into_iter()
            .map(|connector| JobConnector {
                authorization: connector.credential.header().map(str::to_string),
                entry: connector.entry,
            })
            .collect(),
        skill_call: None,
        files: Vec::new(),
    })
}

/// The server's [`CalendarJobs`]: each call runs on the executor, signed for its bot and bound
/// to its exact job text like every other tool job.
pub(crate) struct ExecutorCalendars {
    owner: String,
    executor: Option<Arc<dyn ToolExecutor>>,
    binding: Arc<dyn JobBinding>,
}

impl ExecutorCalendars {
    pub(crate) fn new(
        owner: &str,
        executor: Option<Arc<dyn ToolExecutor>>,
        binding: Arc<dyn JobBinding>,
    ) -> Self {
        Self {
            owner: owner.to_string(),
            executor,
            binding,
        }
    }

    /// The calendar calls of `bot_id` outside a turn: its calendar view, its sync, a CalDAV
    /// discovery.
    pub(crate) fn unbound(state: &AppState, owner: &str, bot_id: &str) -> Self {
        Self::new(
            owner,
            super::executor_for(state),
            Arc::new(ServerBinding::new(state, owner, bot_id, None)),
        )
    }

    /// The calendar calls of one turn, run under the turn's identity and workspace.
    pub(crate) fn in_turn(
        state: &AppState,
        owner: &str,
        principal: BotUser,
        workspace: &str,
    ) -> Self {
        let bot_id = principal.bot_id.clone();
        Self::new(
            owner,
            super::executor_for(state),
            Arc::new(ServerBinding::new(
                state,
                owner,
                &bot_id,
                Some((principal, workspace)),
            )),
        )
    }
}

fn failed(call: &CalendarCall, why: &str) -> ToolError {
    ToolError::new(
        code::FAILED,
        format!(
            "The calendar {} call could not run on the executor: {why}",
            call.name()
        ),
    )
}

#[async_trait]
impl CalendarJobs for ExecutorCalendars {
    fn ready(&self) -> ToolResult<()> {
        match self.executor {
            Some(_) => Ok(()),
            None => Err(ToolError::new(code::NOT_YET, NO_EXECUTOR)),
        }
    }

    async fn run(&self, call: &CalendarCall, connectors: Vec<JobCalendar>) -> ToolResult<String> {
        let Some(executor) = &self.executor else {
            return Err(ToolError::new(code::NOT_YET, NO_EXECUTOR));
        };
        let identity = self
            .binding
            .bind()
            .await
            .map_err(|message| failed(call, &message))?;
        let job = calendar_job(&self.owner, &identity, call, connectors)
            .and_then(|job| {
                serde_json::to_string(&job)
                    .map_err(|error| format!("Cannot encode the calendar job: {error}"))
            })
            .map_err(|message| failed(call, &message))?;
        let started = Instant::now();
        let reply = executor
            .run(
                &self.owner,
                &identity.principal,
                &identity.workspace,
                job,
                JOB_TIMEOUT,
            )
            .await;
        let micros = started.elapsed().as_micros().min(i64::MAX as u128) as i64;
        tracing::info!(
            op = call.name(),
            bot_id = %identity.principal.bot_id,
            run_id = %identity.principal.turn_id,
            owner = %self.owner,
            duration_ms = micros / 1000,
            compute_cost_micro_dollars = crate::compute_cost::compute_cost_micro_dollars(micros, 1),
            "[bot_tools] calendar job finished"
        );
        match reply {
            Ok(reply) => Ok(reply.output),
            Err(error) => {
                tracing::warn!(
                    op = call.name(),
                    bot_id = %identity.principal.bot_id,
                    %error,
                    "[bot_tools] calendar job failed on the executor"
                );
                let refused = failed(call, error.message);
                Err(match error.code {
                    "bot_tool_executor_busy" => ToolError::new(code::RATE_LIMITED, refused.message),
                    _ => refused,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use flow_like_bot_tools::calendar::{
        ExternalEvent, JobAnswer, JobListing, JobReply, Listed, MAX_REPLY_BYTES,
    };
    use flow_like_bot_tools::services::{CallBudget, ConnectorEntry, ConnectorSource, Credential};
    use flow_like_bot_tools_host::ToolReply;
    use flow_like_types::bot_calendar::{CalendarEntrySource, CalendarWake};
    use flow_like_types::bot_connector::{ConnectorKind, ConnectorRule};
    use flow_like_types::bot_task::{BotTask, CreateBotTask};
    use serde_json::json;

    use super::*;
    use crate::bot_tools::executor::ToolExecutorError;
    use crate::execution::DispatchError;

    const OWNER: &str = "user-1";
    const BOT: &str = "bot-1";
    const WORKSPACE: &str = "ws-bot";
    const TOKEN: &str = "Bearer ya29.secret-calendar-token";

    struct FixedBinding;

    #[async_trait]
    impl JobBinding for FixedBinding {
        async fn bind(&self) -> Result<JobIdentity, String> {
            Ok(JobIdentity {
                principal: BotUser {
                    bot_id: BOT.into(),
                    turn_id: "turn-1".into(),
                    conv_id: Some("conv-1".into()),
                },
                workspace: WORKSPACE.into(),
                credential: json!({ "scoped": "read" }),
            })
        }
    }

    struct NeverBound;

    #[async_trait]
    impl JobBinding for NeverBound {
        async fn bind(&self) -> Result<JobIdentity, String> {
            panic!("a job without an executor must not be bound");
        }
    }

    /// Owner, principal, workspace, job text and timeout of one dispatched job.
    type Dispatched = (String, BotUser, String, String, Duration);

    #[derive(Default)]
    struct FakeExecutor {
        jobs: Mutex<Vec<Dispatched>>,
        output: Mutex<Option<String>>,
        failure: Mutex<Option<DispatchError>>,
    }

    #[async_trait]
    impl ToolExecutor for FakeExecutor {
        async fn run(
            &self,
            owner: &str,
            bot: &BotUser,
            workspace: &str,
            job: String,
            timeout: Duration,
        ) -> Result<ToolReply, ToolExecutorError> {
            self.jobs.lock().unwrap().push((
                owner.into(),
                bot.clone(),
                workspace.into(),
                job,
                timeout,
            ));
            if let Some(failure) = self.failure.lock().unwrap().take() {
                return Err(failure.into());
            }
            Ok(ToolReply {
                output: self.output.lock().unwrap().clone().unwrap_or_default(),
                cpu_ms: 1,
                skill_proposal: None,
            })
        }
    }

    fn google() -> ConnectorEntry {
        ConnectorEntry {
            rule: ConnectorRule {
                id: "c-google".into(),
                label: "Work".into(),
                kind: Some(ConnectorKind::Oauth),
                ..ConnectorRule::default()
            },
            kind: ConnectorKind::Oauth,
            url: None,
            provider_id: Some("google".into()),
            api_hosts: vec!["www.googleapis.com".into()],
            calendar: None,
        }
    }

    fn reply_text(reply: &JobReply) -> String {
        let output = flow_like_bot_tools::ToolOutput::new(json!({ "calendar": reply }));
        flow_like_bot_tools::ToolOutcome::from_output(CALENDAR_JOB, output, MAX_REPLY_BYTES)
            .envelope()
            .to_string()
    }

    fn calendars_with(executor: Arc<FakeExecutor>) -> ExecutorCalendars {
        ExecutorCalendars::new(OWNER, Some(executor), Arc::new(FixedBinding))
    }

    #[tokio::test]
    async fn a_calendar_job_carries_the_call_its_sign_in_and_only_a_read_credential() {
        let executor = Arc::new(FakeExecutor::default());
        *executor.output.lock().unwrap() = Some("{}".into());
        let jobs = calendars_with(executor.clone());
        let output = jobs
            .run(
                &CalendarCall::Get {
                    event_id: "g1".into(),
                },
                vec![JobCalendar {
                    entry: google(),
                    credential: Credential::authorization(TOKEN),
                }],
            )
            .await
            .unwrap();
        assert_eq!(output, "{}");
        let (owner, bot, workspace, text, timeout) = executor.jobs.lock().unwrap()[0].clone();
        assert_eq!((owner.as_str(), workspace.as_str()), (OWNER, WORKSPACE));
        assert_eq!((bot.bot_id.as_str(), bot.turn_id.as_str()), (BOT, "turn-1"));
        assert_eq!(timeout, JOB_TIMEOUT);
        let job: ToolJob = serde_json::from_str(&text).unwrap();
        assert_eq!(job.tool, CALENDAR_JOB);
        assert_eq!(job.args, json!({ "op": "get", "event_id": "g1" }));
        assert_eq!((job.owner.as_str(), job.bot_id.as_str()), (OWNER, BOT));
        assert_eq!(
            (job.turn_id.as_str(), job.workspace.as_str()),
            ("turn-1", WORKSPACE)
        );
        assert!(job.scope.apps.is_empty() && !job.scope.storage && !job.scope.data_connections);
        assert_eq!(job.credentials.keys().collect::<Vec<_>>(), [WORKSPACE]);
        assert_eq!(job.credentials[WORKSPACE], json!({ "scoped": "read" }));
        assert!(job.secrets.is_empty() && job.files.is_empty() && job.skill_call.is_none());
        assert_eq!(job.connectors.len(), 1);
        assert_eq!(job.connectors[0].entry.rule.id, "c-google");
        assert_eq!(job.connectors[0].authorization.as_deref(), Some(TOKEN));
        assert!(!format!("{job:?}").contains("secret-calendar-token"));
    }

    #[tokio::test]
    async fn every_call_kind_travels_as_its_own_op() {
        let executor = Arc::new(FakeExecutor::default());
        let jobs = calendars_with(executor.clone());
        let from = chrono::Utc::now();
        let calls = [
            CalendarCall::List {
                from,
                to: from + chrono::Duration::days(1),
            },
            CalendarCall::Create {
                draft: Default::default(),
            },
            CalendarCall::Patch {
                event_id: "g1".into(),
                draft: Default::default(),
            },
            CalendarCall::Delete {
                event_id: "g1".into(),
            },
            CalendarCall::Discover,
        ];
        for call in &calls {
            jobs.run(call, Vec::new()).await.unwrap();
        }
        let ops: Vec<String> = executor
            .jobs
            .lock()
            .unwrap()
            .iter()
            .map(|(.., text, _)| {
                let job: ToolJob = serde_json::from_str(text).unwrap();
                assert!(serde_json::from_value::<CalendarCall>(job.args.clone()).is_ok());
                job.args["op"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(ops, ["list", "create", "patch", "delete", "discover"]);
    }

    #[tokio::test]
    async fn without_an_executor_nothing_is_built_and_the_refusal_says_why() {
        let jobs = ExecutorCalendars::new(OWNER, None, Arc::new(NeverBound));
        let ready = jobs.ready().unwrap_err();
        assert_eq!(ready.code, code::NOT_YET);
        assert!(ready.message.contains("no executor"));
        let refused = jobs
            .run(&CalendarCall::Discover, Vec::new())
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_YET);
    }

    #[tokio::test]
    async fn an_executor_failure_names_the_call_and_never_the_sign_in() {
        let executor = Arc::new(FakeExecutor::default());
        *executor.failure.lock().unwrap() = Some(DispatchError::ToolHttp {
            status: 500,
            message: format!("upstream echoed {TOKEN}"),
        });
        let jobs = calendars_with(executor.clone());
        let error = jobs
            .run(
                &CalendarCall::Delete {
                    event_id: "g1".into(),
                },
                vec![JobCalendar {
                    entry: google(),
                    credential: Credential::authorization(TOKEN),
                }],
            )
            .await
            .unwrap_err();
        assert!(error.message.contains("delete"), "{}", error.message);
        assert!(!error.message.contains("secret-calendar-token"));
        *executor.failure.lock().unwrap() = Some(DispatchError::ToolHttp {
            status: 429,
            message: String::new(),
        });
        let busy = jobs
            .run(&CalendarCall::Discover, Vec::new())
            .await
            .unwrap_err();
        assert_eq!(busy.code, code::RATE_LIMITED);
    }

    #[test]
    fn outside_a_turn_every_job_gets_its_own_run_without_a_conversation() {
        let first = principal_for(None, BOT);
        let second = principal_for(None, BOT);
        assert!(first.turn_id.starts_with(RUN_PREFIX));
        assert_ne!(first.turn_id, second.turn_id);
        assert_eq!((first.bot_id.as_str(), first.conv_id), (BOT, None));
        let turn = BotUser {
            bot_id: BOT.into(),
            turn_id: "turn-9".into(),
            conv_id: Some("conv-9".into()),
        };
        assert_eq!(principal_for(Some(&turn), BOT).turn_id, "turn-9");
    }

    /// The server's connector source as the calendar module sees it: rows and credentials here,
    /// calls through [`ExecutorCalendars`].
    struct ServerSource {
        budget: CallBudget,
        jobs: ExecutorCalendars,
    }

    #[async_trait]
    impl ConnectorSource for ServerSource {
        async fn entries(&self) -> flow_like_types::Result<Vec<ConnectorEntry>> {
            Ok(vec![google()])
        }

        async fn credential(&self, _connector_id: &str) -> flow_like_types::Result<Credential> {
            Ok(Credential::authorization(TOKEN))
        }

        fn budget(&self) -> &CallBudget {
            &self.budget
        }

        fn calendar_jobs(&self) -> Option<&dyn CalendarJobs> {
            Some(&self.jobs)
        }
    }

    struct NoTasks;

    #[async_trait]
    impl flow_like_bot_tools::Scheduler for NoTasks {
        async fn create(&self, _request: CreateBotTask) -> flow_like_types::Result<BotTask> {
            Err(flow_like_types::anyhow!("no task rows in this test"))
        }

        async fn list(&self) -> flow_like_types::Result<Vec<BotTask>> {
            Ok(Vec::new())
        }

        async fn cancel(&self, _task_id: &str) -> flow_like_types::Result<bool> {
            Ok(false)
        }

        async fn calendar_wake(&self) -> flow_like_types::Result<CalendarWake> {
            Ok(CalendarWake::default())
        }
    }

    #[tokio::test]
    async fn the_calendar_view_lists_connected_events_from_the_executors_typed_answer() {
        let from = chrono::Utc::now();
        let event = ExternalEvent::new(
            "g1",
            Some("Dentist"),
            from + chrono::Duration::hours(2),
            None,
            false,
            None,
            None,
            None,
            false,
            true,
        );
        let executor = Arc::new(FakeExecutor::default());
        *executor.output.lock().unwrap() = Some(reply_text(&JobReply {
            answer: JobAnswer::Listed {
                listings: vec![JobListing {
                    connector_id: "c-google".into(),
                    listed: Some(Listed {
                        events: vec![event],
                        complete: true,
                        unreadable: 0,
                    }),
                    error: None,
                }],
            },
            audits: Vec::new(),
        }));
        let source = ServerSource {
            budget: CallBudget::default(),
            jobs: calendars_with(executor.clone()),
        };
        let range = flow_like_bot_tools::calendar::list_range(
            Some(&source),
            &NoTasks,
            from,
            from + chrono::Duration::days(1),
            from,
        )
        .await
        .unwrap();
        assert_eq!(range.entries.len(), 1);
        assert_eq!(range.entries[0].title, "Dentist");
        assert_eq!(range.entries[0].source, CalendarEntrySource::External);
        assert_eq!(range.sources[0].error, None);
        let jobs = executor.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        let job: ToolJob = serde_json::from_str(&jobs[0].3).unwrap();
        assert_eq!(job.args["op"], "list");
        assert_eq!(job.connectors[0].authorization.as_deref(), Some(TOKEN));
    }

    #[tokio::test]
    async fn without_an_executor_the_view_names_the_calendar_unreadable() {
        let source = ServerSource {
            budget: CallBudget::default(),
            jobs: ExecutorCalendars::new(OWNER, None, Arc::new(NeverBound)),
        };
        let from = chrono::Utc::now();
        let range = flow_like_bot_tools::calendar::list_range(
            Some(&source),
            &NoTasks,
            from,
            from + chrono::Duration::days(1),
            from,
        )
        .await
        .unwrap();
        assert!(range.entries.is_empty());
        assert!(
            range.sources[0]
                .error
                .as_deref()
                .unwrap()
                .contains("no executor")
        );
    }
}
