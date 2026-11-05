//! `POST /tool`: one bot tool call for the API.
//!
//! The API signs an unattended executor token (sub = the bot's owner as payer, bot_id = the bot,
//! app = the bot's workspace, run = the turn) whose `dispatch_hash` covers the exact job text, and scopes one credential per
//! app: read and write on the workspace, read on every app the bot may use. The executor trusts
//! nothing in the job beyond that signature. Models go through the API with the same token.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::StatusCode;
use flow_like::bit::Bit;
use flow_like::credentials::{SharedCredentials, StoreType};
use flow_like::flow::execution::ExecutionEnvironment;
use flow_like::flow_like_model_provider::provider::ModelProviderConfiguration;
use flow_like::models::llm::ModelUsageContext;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_bot_tools::{run, MemorySessions};
use flow_like_bot_tools_host::{
    job_hash, outcome_output, AppAccess, HostEnv, JobModels, StateModels, ToolJob, ToolJobEnvelope,
    ToolReply,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;

use crate::jwt::ExecutorClaims;

const MAX_TOOL_SECS: u64 = 300;

pub(crate) type Refusal = (StatusCode, String);

fn unauthorized(message: impl Into<String>) -> Refusal {
    (StatusCode::UNAUTHORIZED, message.into())
}

fn forbidden(message: impl Into<String>) -> Refusal {
    (StatusCode::FORBIDDEN, message.into())
}

/// The job the verified `claims` authorize: the token must bind exactly this job text, name the
/// bot's owner, workspace and turn, and belong to no board, event, page or replay run. Every app
/// the job's scope can reach needs its own credential, and no other credential may ride along.
pub(crate) fn authorize(
    claims: &ExecutorClaims,
    envelope: &ToolJobEnvelope,
) -> Result<ToolJob, Refusal> {
    match &claims.dispatch_hash {
        Some(signed) if *signed == job_hash(&envelope.job) => {}
        _ => return Err(unauthorized("The token does not bind this tool job")),
    }
    let job: ToolJob = serde_json::from_str(&envelope.job).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("Malformed tool job: {error}"),
        )
    })?;
    let own_run = claims.board_id.is_empty()
        && claims.event_id.is_none()
        && claims.page_execution.is_none()
        && claims.shadow != Some(true);
    if !own_run {
        return Err(forbidden(
            "The token belongs to a board run, not a bot turn",
        ));
    }
    if job.bot_id.is_empty() || claims.bot_id.as_deref() != Some(job.bot_id.as_str()) {
        return Err(forbidden("The token does not name this bot"));
    }
    if job.owner.is_empty()
        || claims.sub != job.owner
        || claims.run_id != job.turn_id
        || claims.app_id != job.workspace
    {
        return Err(forbidden(
            "The token does not name this bot's owner, turn and workspace",
        ));
    }
    let reachable: Vec<&str> = job
        .scope
        .apps
        .iter()
        .filter(|app| app.files || app.tables)
        .map(|app| app.app_id.as_str())
        .collect();
    if !job.credentials.contains_key(&job.workspace) {
        return Err(forbidden("The job carries no credential for the workspace"));
    }
    if let Some(app) = reachable
        .iter()
        .find(|app| !job.credentials.contains_key(**app))
    {
        return Err(forbidden(format!(
            "The job carries no credential for app '{app}'"
        )));
    }
    if let Some(app) = job
        .credentials
        .keys()
        .find(|app| **app != job.workspace && !reachable.contains(&app.as_str()))
    {
        return Err(forbidden(format!(
            "The job carries a credential for app '{app}' outside the bot's scope"
        )));
    }
    Ok(job)
}

struct JobAccess {
    credentials: BTreeMap<String, SharedCredentials>,
}

impl JobAccess {
    fn parse(job: &ToolJob) -> Result<Self, Refusal> {
        let credentials = job
            .credentials
            .iter()
            .map(|(app, value)| {
                serde_json::from_value::<SharedCredentials>(value.clone())
                    .map(|credentials| (app.clone(), credentials))
                    .map_err(|error| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("Malformed credential for app '{app}': {error}"),
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { credentials })
    }

    fn credential(&self, app_id: &str) -> flow_like_types::Result<&SharedCredentials> {
        self.credentials
            .get(app_id)
            .ok_or_else(|| flow_like_types::anyhow!("No credential for app '{app_id}'"))
    }
}

#[async_trait]
impl AppAccess for JobAccess {
    async fn store(&self, app_id: &str) -> flow_like_types::Result<FlowLikeStore> {
        self.credential(app_id)?
            .to_store_type(StoreType::Content)
            .await
    }

    async fn connection(&self, app_id: &str) -> flow_like_types::Result<Connection> {
        self.credential(app_id)?
            .to_db(app_id)
            .await?
            .execute()
            .await
            .map_err(Into::into)
    }
}

async fn tool_state(workspace: &SharedCredentials) -> Result<Arc<FlowLikeState>, Refusal> {
    let content = workspace
        .to_store_type(StoreType::Content)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let mut config = FlowLikeConfig::with_default_store(content);
    config.stores.app_meta_store = None;
    let mut state = FlowLikeState::new_with_model_config(
        config,
        HTTPClient::new_without_refetch(),
        ModelProviderConfiguration::default(),
    );
    state.execution_environment = ExecutionEnvironment::server_default();
    Ok(Arc::new(state))
}

fn bit_of(value: &Option<serde_json::Value>) -> Option<Bit> {
    value
        .as_ref()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn models(
    job: &ToolJob,
    claims: &ExecutorClaims,
    token: &str,
    state: Arc<FlowLikeState>,
) -> StateModels {
    let JobModels { chat, embedding } = job.models.clone();
    StateModels {
        state,
        chat: bit_of(&chat),
        embedding: bit_of(&embedding),
        token: Some(token.to_string()),
        usage: ModelUsageContext {
            app_id: Some(job.workspace.clone()),
            run_id: Some(job.turn_id.clone()),
            api_base_url: flow_like::hub::hub_origin(&claims.callback_url, true),
        },
    }
}

fn deadline(claims: &ExecutorClaims) -> Duration {
    let remaining = claims.exp.saturating_sub(chrono::Utc::now().timestamp());
    Duration::from_secs((remaining.max(1) as u64).min(MAX_TOOL_SECS))
}

pub(crate) async fn run_job(
    claims: &ExecutorClaims,
    envelope: &ToolJobEnvelope,
    job: ToolJob,
) -> Result<ToolReply, Refusal> {
    let access = Arc::new(JobAccess::parse(&job)?);
    let state = tool_state(
        access
            .credential(&job.workspace)
            .map_err(|error| forbidden(error.to_string()))?,
    )
    .await?;
    let models = Arc::new(models(&job, claims, &envelope.executor_jwt, state));
    let env = HostEnv::new(&job.workspace, &job.scope, access, models)
        .with_sessions(Arc::new(MemorySessions::default()))
        .with_expensive_documents(job.allow_expensive_documents)
        .with_secrets(Arc::new(job.secrets.clone()))
        .with_connectors(flow_like_bot_tools_host::connector_source(&job.connectors));
    let started = Instant::now();
    let outcome = tokio::time::timeout(deadline(claims), run(&job.tool, job.args.clone(), &env))
        .await
        .map_err(|_| {
            (
                StatusCode::GATEWAY_TIMEOUT,
                format!("{} timed out", job.tool),
            )
        })?;
    Ok(ToolReply {
        output: outcome_output(&outcome),
        cpu_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

pub(crate) async fn handle(envelope: ToolJobEnvelope) -> Result<ToolReply, Refusal> {
    let claims = crate::jwt::verify_jwt_async(&envelope.executor_jwt)
        .await
        .map_err(|error| unauthorized(error.to_string()))?;
    let job = authorize(&claims, &envelope)?;
    run_job(&claims, &envelope, job).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bot_tools_host::{JobApp, JobScope};
    use flow_like_types::data_classification::DataClassification;
    use serde_json::json;

    const OWNER: &str = "user-1";
    const TURN: &str = "turn-1";
    const BOT: &str = "bot-1";
    const WORKSPACE: &str = "ws-bot";

    fn job(apps: Vec<JobApp>, credentials: &[&str]) -> ToolJob {
        ToolJob {
            tool: "file_read".into(),
            args: json!({ "file": "bot/a.md" }),
            owner: OWNER.into(),
            bot_id: BOT.into(),
            turn_id: TURN.into(),
            workspace: WORKSPACE.into(),
            scope: JobScope {
                storage: true,
                data_connections: false,
                classification: DataClassification::Internal,
                apps,
            },
            credentials: credentials
                .iter()
                .map(|app| (app.to_string(), json!({})))
                .collect(),
            models: JobModels::default(),
            allow_expensive_documents: false,
            secrets: Default::default(),
            connectors: Vec::new(),
        }
    }

    fn app(id: &str, files: bool, tables: bool) -> JobApp {
        JobApp {
            app_id: id.into(),
            alias: None,
            files,
            tables,
            column_attributes: None,
        }
    }

    fn claims(hash: Option<String>) -> ExecutorClaims {
        serde_json::from_value(json!({
            "sub": OWNER, "run_id": TURN, "app_id": WORKSPACE, "board_id": "",
            "callback_url": "https://api.example", "typ": "executor", "iss": "flow-like",
            "aud": "flow-like-executor", "iat": 0, "nbf": 0, "exp": 0, "jti": "j",
            "dispatch_hash": hash, "bot_id": BOT,
        }))
        .expect("claims")
    }

    fn signed(job: &ToolJob) -> (ExecutorClaims, ToolJobEnvelope) {
        let text = serde_json::to_string(job).unwrap();
        (
            claims(Some(job_hash(&text))),
            ToolJobEnvelope {
                executor_jwt: "jwt".into(),
                job: text,
            },
        )
    }

    fn refused(result: Result<ToolJob, Refusal>) -> (StatusCode, String) {
        result.expect_err("the job must be refused")
    }

    #[test]
    fn a_job_the_token_binds_is_authorized() {
        let (claims, envelope) = signed(&job(vec![app("a", true, true)], &[WORKSPACE, "a"]));
        let job = authorize(&claims, &envelope).expect("authorized");
        assert_eq!(job.tool, "file_read");
    }

    #[test]
    fn a_token_that_binds_another_job_or_none_is_refused() {
        let (claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
        let altered = ToolJobEnvelope {
            job: envelope.job.replace("file_read", "file_write"),
            ..envelope.clone()
        };
        assert_eq!(
            refused(authorize(&claims, &altered)).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            refused(authorize(&self::claims(None), &envelope)).0,
            StatusCode::UNAUTHORIZED
        );
        let board_hash = self::claims(Some("a-dispatch-hash".into()));
        assert_eq!(
            refused(authorize(&board_hash, &envelope)).0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn the_token_must_name_the_owner_turn_and_workspace() {
        for (field, value) in [
            ("sub", "user-2"),
            ("run_id", "turn-2"),
            ("app_id", "ws-other"),
        ] {
            let (mut claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
            match field {
                "sub" => claims.sub = value.into(),
                "run_id" => claims.run_id = value.into(),
                _ => claims.app_id = value.into(),
            }
            assert_eq!(
                refused(authorize(&claims, &envelope)).0,
                StatusCode::FORBIDDEN,
                "{field}"
            );
        }
    }

    #[test]
    fn a_board_run_token_cannot_run_tools() {
        let (mut claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
        claims.board_id = "board-1".into();
        assert_eq!(
            refused(authorize(&claims, &envelope)).0,
            StatusCode::FORBIDDEN
        );
        let (mut claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
        claims.event_id = Some("event-1".into());
        assert_eq!(
            refused(authorize(&claims, &envelope)).0,
            StatusCode::FORBIDDEN
        );
        let (mut claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
        claims.shadow = Some(true);
        assert_eq!(
            refused(authorize(&claims, &envelope)).0,
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn the_token_must_name_the_jobs_bot() {
        let (mut claims, envelope) = signed(&job(vec![], &[WORKSPACE]));
        claims.bot_id = None;
        assert_eq!(
            refused(authorize(&claims, &envelope)).0,
            StatusCode::FORBIDDEN
        );
        claims.bot_id = Some("bot-2".into());
        assert_eq!(
            refused(authorize(&claims, &envelope)).0,
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn every_reachable_app_needs_its_credential_and_no_other_may_ride_along() {
        let (claims, envelope) = signed(&job(vec![app("a", true, false)], &[WORKSPACE]));
        assert!(refused(authorize(&claims, &envelope)).1.contains("'a'"));
        let (claims, envelope) = signed(&job(vec![], &["a"]));
        assert!(refused(authorize(&claims, &envelope))
            .1
            .contains("workspace"));
        let (claims, envelope) = signed(&job(vec![], &[WORKSPACE, "stranger"]));
        assert!(refused(authorize(&claims, &envelope))
            .1
            .contains("stranger"));
        let (claims, envelope) = signed(&job(vec![app("a", false, false)], &[WORKSPACE, "a"]));
        assert!(refused(authorize(&claims, &envelope)).1.contains("'a'"));
        let (claims, envelope) = signed(&job(vec![app("a", false, false)], &[WORKSPACE]));
        assert!(authorize(&claims, &envelope).is_ok());
    }

    #[tokio::test]
    async fn a_token_that_does_not_verify_is_refused_before_anything_is_read() {
        let envelope = ToolJobEnvelope {
            executor_jwt: "not-a-token".into(),
            job: "{}".into(),
        };
        let (status, _) = handle(envelope).await.expect_err("refused");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
