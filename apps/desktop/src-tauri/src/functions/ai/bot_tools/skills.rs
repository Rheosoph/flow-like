//! The desktop's `SkillHost`: a local bot's skills live in the local store, an account bot's in
//! the hub. Both show the bot only what the owner approved.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::SkillHost;
use flow_like_types::{
    Result, anyhow,
    bot_skill::{BotSkill, CreateBotSkill, SkillStatus},
};
use reqwest::Method;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::local_bots::{LocalBots, on_store};

struct LocalSkills {
    app: AppHandle,
    bot_id: String,
}

impl LocalSkills {
    async fn on_store<T: Send + 'static>(
        &self,
        work: impl FnOnce(&flow_like_local_bots::LocalBotStore) -> flow_like_local_bots::Result<T>
        + Send
        + 'static,
    ) -> Result<T> {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        on_store(&bots, work)
            .await
            .map_err(|error| anyhow!(error.to_string()))
    }
}

#[async_trait]
impl SkillHost for LocalSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        let (bot_id, name) = (self.bot_id.clone(), name.to_string());
        self.on_store(move |store| store.skill_by_name(&bot_id, &name))
            .await
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        let bot_id = self.bot_id.clone();
        self.on_store(move |store| {
            store.create_skill(&bot_id, request, flow_like_local_bots::SkillOrigin::proposed_by_bot())
        })
        .await
    }
}

struct AccountSkills {
    api: Arc<dyn TaskApi>,
    bot_id: String,
}

impl AccountSkills {
    fn base(&self) -> String {
        format!(
            "bots/{}/skills",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
    }
}

#[async_trait]
impl SkillHost for AccountSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        let listed = self
            .api
            .request(Method::GET, &self.base(), None)
            .await
            .map_err(|error| anyhow!(error))?;
        let skills = listed.get("skills").cloned().unwrap_or(Value::Array(Vec::new()));
        let skills: Vec<Value> = serde_json::from_value(skills)
            .map_err(|error| anyhow!("The hub sent unusable skills: {error}"))?;
        let Some(found) = skills.into_iter().find(|skill| skill["name"] == name) else {
            return Ok(None);
        };
        let status = SkillStatus::parse(found["status"].as_str().unwrap_or_default())
            .unwrap_or(SkillStatus::Disabled);
        let source = flow_like_types::bot_skill::SkillSource::parse(
            found["source"].as_str().unwrap_or_default(),
        )
        .unwrap_or(flow_like_types::bot_skill::SkillSource::Import);
        let text = |key: &str| found[key].as_str().unwrap_or_default().to_string();
        Ok(Some(BotSkill {
            id: text("id"),
            bot_id: self.bot_id.clone(),
            name: text("name"),
            description: text("description"),
            body: text("body"),
            source,
            status,
            tools: serde_json::from_value(found["tools"].clone()).unwrap_or_default(),
            updated_at: text("updated_at"),
        }))
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        let body = json!({
            "name": request.name,
            "description": request.description,
            "body": request.body,
        });
        let created = self
            .api
            .request(Method::POST, &format!("{}/propose", self.base()), Some(&body))
            .await
            .map_err(|error| anyhow!(error))?;
        let name = created["name"].as_str().unwrap_or(&request.name).to_string();
        self.get(&name)
            .await?
            .ok_or_else(|| anyhow!("The hub saved the skill but did not list it"))
    }
}

/// The skill host for `bot_id`: the local store's when this device holds the bot, else the hub's.
pub(crate) async fn skills_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
) -> Arc<dyn SkillHost> {
    if super::held_locally(app, bot_id).await {
        Arc::new(LocalSkills {
            app: app.clone(),
            bot_id: bot_id.to_string(),
        })
    } else {
        Arc::new(AccountSkills {
            api,
            bot_id: bot_id.to_string(),
        })
    }
}
