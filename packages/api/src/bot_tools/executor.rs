use std::time::Duration;

use async_trait::async_trait;
use flow_like_bot_tools_host::{ToolJobEnvelope, ToolReply, job_hash};

use crate::{
    execution::{
        ExecutionJwtParams, TokenType, bind_bot_identity, bind_tool_job,
        sign_unattended_execution_jwt,
    },
    middleware::jwt::BotUser,
    state::AppState,
};

const TOKEN_TTL_SECS: i64 = 330;

/// Where a bot turn's heavy tool calls run.
#[async_trait]
pub(crate) trait ToolExecutor: Send + Sync {
    /// Runs `job` (JSON text) for `bot` (and its `owner`, the payer) in `workspace`.
    async fn run(
        &self,
        owner: &str,
        bot: &BotUser,
        workspace: &str,
        job: String,
        timeout: Duration,
    ) -> Result<ToolReply, String>;
}

/// The deployment's executor, reached through the dispatcher.
pub(crate) struct DispatcherExecutor {
    state: AppState,
}

impl DispatcherExecutor {
    /// `None` when the deployment has no executor that takes tool jobs or cannot sign for one.
    pub(crate) fn of(state: &AppState) -> Option<Self> {
        available(state).then(|| Self {
            state: state.clone(),
        })
    }
}

pub(crate) fn available(state: &AppState) -> bool {
    state.dispatcher.tool_target().is_some() && crate::execution::is_jwt_configured()
}

fn callback_url() -> String {
    std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

/// The unattended executor token of one job: sub = owner (the payer), bot = the bot, app =
/// workspace, run = turn, bound to the job text by its digest.
pub(crate) fn mint(
    owner: &str,
    bot: &BotUser,
    workspace: &str,
    job: &str,
) -> Result<String, String> {
    let token = sign_unattended_execution_jwt(ExecutionJwtParams {
        user_id: owner.to_string(),
        technical_user_id: None,
        run_id: bot.turn_id.clone(),
        app_id: workspace.to_string(),
        board_id: String::new(),
        event_id: None,
        app_chain: None,
        correlation: None,
        callback_url: callback_url(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(TOKEN_TTL_SECS),
        shadow: None,
    })
    .map_err(|error| format!("Cannot sign the tool job: {error}"))?;
    let token = bind_bot_identity(&token, &bot.bot_id, bot.conv_id.as_deref())
        .map_err(|error| format!("Cannot bind the bot identity: {error}"))?;
    bind_tool_job(&token, job_hash(job))
        .map_err(|error| format!("Cannot bind the tool job: {error}"))
}

#[async_trait]
impl ToolExecutor for DispatcherExecutor {
    async fn run(
        &self,
        owner: &str,
        bot: &BotUser,
        workspace: &str,
        job: String,
        timeout: Duration,
    ) -> Result<ToolReply, String> {
        let executor_jwt = mint(owner, bot, workspace, &job)?;
        let envelope = ToolJobEnvelope { executor_jwt, job };
        self.state
            .dispatcher
            .invoke_tool(owner, &envelope, timeout)
            .await
            .map_err(|error| error.to_string())
    }
}
