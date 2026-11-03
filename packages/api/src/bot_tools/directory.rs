use async_trait::async_trait;
use flow_like::flow::copilot::BotRunContext;
use flow_like_bot_tools_host::JobScope;
use serde_json::Value;

use super::{access::shared_credentials, host::AppDirectory, scope::job_scope};
use crate::{middleware::jwt::BotUser, state::AppState};

/// The bot's rights and credentials, read from the database and the credential service.
pub(crate) struct ApiDirectory {
    pub state: AppState,
    pub principal: BotUser,
    pub bot: BotRunContext,
}

#[async_trait]
impl AppDirectory for ApiDirectory {
    async fn scope(&self) -> Result<JobScope, String> {
        job_scope(&self.state, &self.principal, &self.bot)
            .await
            .map_err(|error| format!("Cannot resolve what the bot may use: {error}"))
    }

    async fn credential(&self, app_id: &str) -> Result<Value, String> {
        let workspace = self.bot.workspace_app_id.as_deref().unwrap_or_default();
        let shared = shared_credentials(&self.state, &self.principal, workspace, app_id)
            .await
            .map_err(|error| format!("Cannot scope credentials for the tool: {error}"))?;
        serde_json::to_value(shared)
            .map_err(|error| format!("Cannot encode the tool credentials: {error}"))
    }
}
