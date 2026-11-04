//! The bot's secrets in its server turns: SERVER values decrypted for the tools that fill them in,
//! names and descriptions for the model. Every read sets the secret's `last_used_at`.

use async_trait::async_trait;
use flow_like_bot_tools::SecretInfo;
use flow_like_bot_tools_host::SecretSource;
use flow_like_types::{Result, anyhow};

use crate::routes::bots::secrets::{open_for_bot, secret_infos};
use crate::state::AppState;

pub(crate) struct ServerSecrets {
    state: AppState,
    bot_id: String,
}

impl ServerSecrets {
    pub(crate) fn new(state: &AppState, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_string(),
        }
    }
}

#[async_trait]
impl SecretSource for ServerSecrets {
    async fn secret(&self, name: &str) -> Result<Option<String>> {
        let value = open_for_bot(
            &self.state.db,
            &self.state.encryption_key,
            &self.bot_id,
            name,
        )
        .await
        .map_err(|error| anyhow!("{error}"))?;
        if value.is_some() {
            tracing::info!(bot_id = %self.bot_id, name, "[bot_tools] a server turn read a bot secret");
        }
        Ok(value)
    }

    async fn list(&self) -> Result<Vec<SecretInfo>> {
        Ok(secret_infos(&self.state.db, &self.bot_id)
            .await?
            .into_iter()
            .map(|info| SecretInfo {
                name: info.name,
                description: info.description,
            })
            .collect())
    }
}
