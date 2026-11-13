//! The bot's secrets in its server turns: SERVER values decrypted for the tools that fill them in,
//! names and descriptions for the model. Every read sets the secret's `last_used_at`. A SEALED
//! secret opens only where the bot's key lives, so a server turn gets `secret_sealed`.

use async_trait::async_trait;
use flow_like_bot_tools::{SecretInfo, ToolError, error::code};
use flow_like_bot_tools_host::SecretSource;
use flow_like_types::{Result, anyhow};

use crate::error::ApiError;
use crate::routes::bots::secrets::{BOT_SECRET_SEALED, open_for_bot, secret_infos};
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

/// What a server turn's tool learns when secret `name` cannot be read.
pub(crate) fn read_failure(name: &str, error: ApiError) -> flow_like_types::Error {
    if error.public_code() != BOT_SECRET_SEALED {
        return anyhow!("{error}");
    }
    ToolError::new(
        code::SECRET_SEALED,
        format!(
            "The secret '{name}' is sealed to this bot's own key, which the server never holds, so a turn that runs on the server cannot use it. Tell the owner: '{name}' works only where the bot runs with its key (its desktop or device), or they can store it again without sealing."
        ),
    )
    .with_details(serde_json::json!({ "name": name }))
    .into()
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
        .map_err(|error| read_failure(name, error))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn a_server_turn_learns_a_sealed_secret_opens_only_where_the_bots_key_lives() {
        let sealed = ApiError::coded(StatusCode::CONFLICT, BOT_SECRET_SEALED, "sealed");
        let error = ToolError::from(read_failure("API_KEY", sealed));
        assert_eq!(error.code, code::SECRET_SEALED);
        assert!(error.message.contains("'API_KEY'"), "{}", error.message);
        assert!(
            error.message.contains("Tell the owner"),
            "{}",
            error.message
        );
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "name": "API_KEY" }))
        );

        let other = ToolError::from(read_failure("API_KEY", ApiError::internal("broken")));
        assert_eq!(other.code, code::FAILED);
    }
}
