use std::time::Duration;

use async_trait::async_trait;
use flow_like_bot_tools_host::{ToolJobEnvelope, ToolReply, job_hash};

use crate::{
    execution::{
        DispatchError, ExecutionJwtParams, TokenType, bind_bot_identity, bind_tool_job,
        sign_unattended_execution_jwt,
    },
    middleware::jwt::BotUser,
    state::AppState,
};

const TOKEN_TTL_SECS: i64 = 330;

/// Keeps private transport details in server logs while giving the bot a useful diagnosis.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub(crate) struct ToolExecutorError {
    detail: String,
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

impl From<DispatchError> for ToolExecutorError {
    fn from(error: DispatchError) -> Self {
        let (code, message) = match &error {
            DispatchError::ToolHttp {
                status: 404 | 501, ..
            } => (
                "bot_tool_executor_unsupported",
                "The configured executor does not support bot tools. The server needs an executor build with bot tools enabled and the correct tool endpoint. This request did not run; changing the script or its packages will not fix this.",
            ),
            DispatchError::ToolHttp {
                status: 401 | 403, ..
            } => (
                "bot_tool_executor_unauthorized",
                "The executor rejected the server's authorization. This request did not run. The server's executor credentials or signing configuration need to be checked.",
            ),
            DispatchError::ToolHttp { status: 429, .. } => (
                "bot_tool_executor_busy",
                "The executor has no capacity for this request. It did not run; try again later.",
            ),
            DispatchError::ToolHttp {
                status: 408 | 504, ..
            }
            | DispatchError::ToolTimeout => (
                "bot_tool_executor_timeout",
                "The executor did not finish before its deadline. Execution and saved files are unconfirmed; check the workspace before retrying.",
            ),
            DispatchError::Configuration(_) => (
                "bot_tool_executor_configuration",
                "The server could not configure or authorize the tool job. It did not run. Check the server's executor configuration.",
            ),
            DispatchError::Rejected(_) => (
                "bot_tool_executor_rejected",
                "The execution service rejected the tool job before it started. Check the server logs for the rejection reason; changing the script's packages is not a supported diagnosis.",
            ),
            DispatchError::Serialization(_) => (
                "bot_tool_executor_invalid_reply",
                "The server could not encode the job or read the executor's reply. Execution and saved files are unconfirmed. Check that the API and executor use compatible versions.",
            ),
            DispatchError::ExecutorFailed(_) => (
                "bot_tool_executor_crashed",
                "The execution service reported that the executor stopped unexpectedly. Saved files are unconfirmed. Check the executor logs before retrying.",
            ),
            _ => (
                "bot_tool_executor_failed",
                "Communication with the executor failed. Execution and saved files are unconfirmed. The server logs contain the failure reason; this does not establish a problem with the script or its packages.",
            ),
        };
        Self {
            detail: error.to_string(),
            code,
            message,
        }
    }
}

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
    ) -> Result<ToolReply, ToolExecutorError>;
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
    ) -> Result<ToolReply, ToolExecutorError> {
        let executor_jwt =
            mint(owner, bot, workspace, &job).map_err(DispatchError::Configuration)?;
        let envelope = ToolJobEnvelope { executor_jwt, job };
        self.state
            .dispatcher
            .invoke_tool(owner, &envelope, timeout)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_http_failures_explain_the_cause_without_exposing_private_details() {
        for (status, code) in [
            (404, "bot_tool_executor_unsupported"),
            (501, "bot_tool_executor_unsupported"),
            (401, "bot_tool_executor_unauthorized"),
            (403, "bot_tool_executor_unauthorized"),
            (429, "bot_tool_executor_busy"),
            (504, "bot_tool_executor_timeout"),
        ] {
            let error = ToolExecutorError::from(DispatchError::ToolHttp {
                status,
                message: "private upstream address and token".into(),
            });
            assert_eq!(error.code, code);
            assert!(!error.message.contains("private upstream"));
            assert!(error.to_string().contains("private upstream"));
        }
    }

    #[test]
    fn lost_results_never_claim_that_nothing_was_saved() {
        for failure in [
            DispatchError::ToolTimeout,
            DispatchError::Network("connection lost".into()),
            DispatchError::Serialization("bad response".into()),
            DispatchError::ExecutorFailed("worker died".into()),
        ] {
            let error = ToolExecutorError::from(failure);
            assert!(error.message.contains("unconfirmed"));
            assert!(!error.message.contains("Nothing was changed"));
        }
    }
}
