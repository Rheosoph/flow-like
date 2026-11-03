use async_trait::async_trait;
use flow_like::credentials::{SharedCredentials, StoreType};
use flow_like_bot_tools_host::AppAccess;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Result;

use crate::{
    credentials::{CredentialsAccess, RuntimeCredentialsTrait},
    state::AppState,
};

/// Credentials scoped to the bot's owner: read and write on the workspace, read on every other
/// app. The same grants go to the executor for tool jobs.
pub(crate) async fn shared_credentials(
    state: &AppState,
    owner: &str,
    workspace: &str,
    app_id: &str,
) -> Result<SharedCredentials> {
    let mode = if app_id == workspace {
        CredentialsAccess::EditAppContent
    } else {
        CredentialsAccess::ReadAppContent
    };
    let credentials = state.scoped_credentials(owner, app_id, mode).await?;
    Ok(credentials.into_shared_credentials())
}

pub(crate) struct ApiAccess {
    state: AppState,
    owner: String,
    workspace: String,
}

impl ApiAccess {
    pub(crate) fn new(state: AppState, owner: &str, workspace: &str) -> Self {
        Self {
            state,
            owner: owner.to_string(),
            workspace: workspace.to_string(),
        }
    }

    async fn credentials(&self, app_id: &str) -> Result<SharedCredentials> {
        shared_credentials(&self.state, &self.owner, &self.workspace, app_id).await
    }
}

#[async_trait]
impl AppAccess for ApiAccess {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        self.credentials(app_id)
            .await?
            .to_store_type(StoreType::Content)
            .await
    }

    async fn connection(&self, app_id: &str) -> Result<Connection> {
        let builder = self.credentials(app_id).await?.to_db(app_id).await?;
        Ok(builder.execute().await?)
    }
}
