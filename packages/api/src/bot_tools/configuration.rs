use std::sync::Arc;

use async_trait::async_trait;
use flow_like::{
    flow::copilot::{model_changes_need_catalogue, profile_models, validate_model_changes},
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_bot_tools::configuration::{ConfigurationBackend, snapshot, validate_changes};
use flow_like_types::{Result, anyhow};
use serde_json::{Value, json};

use crate::{
    audit::{AuditRecordInput, record_entry},
    routes::bots::{configuration, model::Bot, owned_bot},
    state::AppState,
};

/// The host fixes the owner, bot and profile before the model can call a configuration tool.
pub(crate) struct ApiConfiguration {
    state: AppState,
    owner_id: String,
    bot_id: String,
    profile: Arc<Profile>,
    flow_like_state: Arc<FlowLikeState>,
}

impl ApiConfiguration {
    pub(crate) fn new(
        state: &AppState,
        owner_id: &str,
        bot_id: &str,
        profile: Arc<Profile>,
        flow_like_state: Arc<FlowLikeState>,
    ) -> Self {
        Self {
            state: state.clone(),
            owner_id: owner_id.to_owned(),
            bot_id: bot_id.to_owned(),
            profile,
            flow_like_state,
        }
    }
}

#[async_trait]
impl ConfigurationBackend for ApiConfiguration {
    async fn inspect(&self) -> Result<Value> {
        let row = owned_bot(&self.state.db, &self.owner_id, &self.bot_id)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(snapshot(&serde_json::to_value(Bot::from(row))?))
    }

    async fn models(&self) -> Result<Value> {
        owned_bot(&self.state.db, &self.owner_id, &self.bot_id)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        profile_models(&self.profile, &self.flow_like_state, true).await
    }

    async fn update(&self, changes: Value) -> Result<Value> {
        validate_changes(&changes)?;
        if model_changes_need_catalogue(&changes) {
            let catalogue = self.models().await?;
            validate_model_changes(&changes, &catalogue)?;
        }
        let patched = configuration::apply(&self.state, &self.owner_id, &self.bot_id, changes)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        if !patched.changed.is_empty() {
            record_entry(
                &self.state,
                AuditRecordInput::system(&self.bot_id, "bot.update", "Bot", &self.bot_id)
                    .with_details(json!({
                        "fields": patched.changed,
                        "owner_id": self.owner_id,
                        "source": "approved_configuration_tool",
                    })),
            )
            .await;
        }
        Ok(snapshot(&serde_json::to_value(Bot::from(patched.bot))?))
    }
}
