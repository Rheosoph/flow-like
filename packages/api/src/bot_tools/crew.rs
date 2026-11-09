use std::sync::Arc;

use async_trait::async_trait;
use flow_like::{
    flow::copilot::{model_changes_need_catalogue, profile_models, validate_model_changes},
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_bot_tools::crew::CrewBackend;
use flow_like_types::{Result, anyhow};
use serde_json::{Value, json};

use crate::{
    audit::{AuditRecordInput, record_entry},
    routes::bots::{
        apps::{attach_owned, attached_apps, detach_owned},
        crud::{create_owned, owner_bots, update_owned},
        model::{Bot, CreateBotRequest, UpdateBotRequest},
        owned_bot,
    },
    state::AppState,
};

/// Flux's management of its owner's bots. The host fixes the owner and the Flux bot that acts
/// before the model can call a crew tool.
pub(crate) struct ApiCrew {
    state: AppState,
    owner_id: String,
    actor_bot_id: String,
    profile: Arc<Profile>,
    flow_like_state: Arc<FlowLikeState>,
}

impl ApiCrew {
    pub(crate) fn new(
        state: &AppState,
        owner_id: &str,
        actor_bot_id: &str,
        profile: Arc<Profile>,
        flow_like_state: Arc<FlowLikeState>,
    ) -> Self {
        Self {
            state: state.clone(),
            owner_id: owner_id.to_owned(),
            actor_bot_id: actor_bot_id.to_owned(),
            profile,
            flow_like_state,
        }
    }

    async fn check_models(&self, request: &Value) -> Result<()> {
        if model_changes_need_catalogue(request) {
            let catalogue = profile_models(&self.profile, &self.flow_like_state, true).await?;
            validate_model_changes(request, &catalogue)?;
        }
        Ok(())
    }

    async fn app_ids(&self, bot_id: &str) -> Result<Vec<String>> {
        Ok(attached_apps(&self.state.db, &self.owner_id, bot_id)
            .await?
            .into_iter()
            .map(|app| app.app_id)
            .collect())
    }

    async fn audit(&self, action: &str, bot_id: &str, mut details: Value) {
        if let Some(fields) = details.as_object_mut() {
            fields.insert("owner_id".into(), json!(self.owner_id));
            fields.insert("source".into(), json!("approved_crew_tool"));
        }
        record_entry(
            &self.state,
            AuditRecordInput::system(&self.actor_bot_id, action, "Bot", bot_id)
                .with_details(details),
        )
        .await;
    }

    async fn move_apps(&self, bot_id: &str, attach: &[String], detach: &[String]) -> Result<()> {
        for app_id in attach {
            if attach_owned(&self.state, &self.owner_id, bot_id, app_id)
                .await
                .map_err(|error| anyhow!("Attaching app '{app_id}' failed: {error}"))?
            {
                self.audit("bot.app.attach", bot_id, json!({ "app_id": app_id }))
                    .await;
            }
        }
        for app_id in detach {
            if detach_owned(&self.state, &self.owner_id, bot_id, app_id)
                .await
                .map_err(|error| anyhow!("Detaching app '{app_id}' failed: {error}"))?
            {
                self.audit("bot.app.detach", bot_id, json!({ "app_id": app_id }))
                    .await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl CrewBackend for ApiCrew {
    async fn bots(&self) -> Result<Vec<Value>> {
        let bots = owner_bots(&self.state, &self.owner_id)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        bots.into_iter()
            .map(|bot| Ok(serde_json::to_value(bot)?))
            .collect()
    }

    async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<String>)> {
        let row = owned_bot(&self.state.db, &self.owner_id, bot_id)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let apps = self.app_ids(bot_id).await?;
        Ok((serde_json::to_value(Bot::from(row))?, apps))
    }

    fn profile_id(&self) -> Option<String> {
        Some(self.profile.id.clone()).filter(|id| !id.is_empty())
    }

    async fn create(&self, request: Value, app_ids: &[String]) -> Result<Value> {
        self.check_models(&request).await?;
        let body: CreateBotRequest = serde_json::from_value(request)?;
        let bot = create_owned(&self.state, &self.owner_id, body)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        self.audit(
            "bot.create",
            &bot.id,
            json!({ "capabilities": bot.capabilities, "location": bot.location }),
        )
        .await;
        self.move_apps(&bot.id, app_ids, &[])
            .await
            .map_err(|error| anyhow!("Bot '{}' was created, but {error}", bot.id))?;
        Ok(serde_json::to_value(bot)?)
    }

    async fn update(
        &self,
        bot_id: &str,
        request: Option<Value>,
        attach: &[String],
        detach: &[String],
    ) -> Result<Value> {
        let row = match request {
            Some(request) => {
                self.check_models(&request).await?;
                let body: UpdateBotRequest = serde_json::from_value(request)?;
                let patched = update_owned(&self.state, &self.owner_id, bot_id, body)
                    .await
                    .map_err(|error| anyhow!("{error}"))?;
                if !patched.changed.is_empty() {
                    self.audit("bot.update", bot_id, json!({ "fields": patched.changed }))
                        .await;
                }
                patched.bot
            }
            None => owned_bot(&self.state.db, &self.owner_id, bot_id)
                .await
                .map_err(|error| anyhow!("{error}"))?,
        };
        self.move_apps(bot_id, attach, detach).await?;
        Ok(serde_json::to_value(Bot::from(row))?)
    }
}
