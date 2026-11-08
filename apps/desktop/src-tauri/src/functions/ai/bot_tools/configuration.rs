//! Configuration of the bot bound to this turn, after the bridge has obtained owner approval.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::flow::copilot::{
    model_changes_need_catalogue, profile_models, validate_model_changes,
};
use flow_like_bot_tools::configuration::{ConfigurationBackend, snapshot, validate_changes};
use flow_like_types::{Result, anyhow};
use reqwest::Method;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::{
    ai::{native_bot_tools::BotToolRuntime, native_platform_tools::segment},
    local_bots::{LocalBots, on_store},
};

pub(crate) struct DesktopConfiguration {
    app: AppHandle,
    runtime: Arc<BotToolRuntime>,
    api: Arc<dyn TaskApi>,
}

impl DesktopConfiguration {
    pub(crate) fn new(
        app: &AppHandle,
        runtime: Arc<BotToolRuntime>,
        api: Arc<dyn TaskApi>,
    ) -> Self {
        Self {
            app: app.clone(),
            runtime,
            api,
        }
    }

    fn path(&self) -> String {
        format!("bots/{}", segment(&self.runtime.context.bot_id))
    }

    async fn current(&self) -> Result<Value> {
        if self.runtime.account {
            return self
                .api
                .request(Method::GET, &self.path(), None)
                .await
                .map_err(|error| anyhow!(error));
        }
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        let id = self.runtime.context.bot_id.clone();
        let bot = on_store(&bots, move |store| store.get_bot(&id))
            .await
            .map_err(|error| anyhow!(error.to_string()))?
            .ok_or_else(|| anyhow!("This bot no longer exists on this device"))?;
        Ok(serde_json::to_value(bot)?)
    }
}

#[async_trait]
impl ConfigurationBackend for DesktopConfiguration {
    async fn inspect(&self) -> Result<Value> {
        Ok(snapshot(&self.current().await?))
    }

    async fn models(&self) -> Result<Value> {
        let profile = self.runtime.profile.as_ref().ok_or_else(|| {
            anyhow!("This turn has no profile. Select a profile before mapping models.")
        })?;
        profile_models(
            profile,
            &self.runtime.state,
            self.runtime.context.decision_hosted_only,
        )
        .await
    }

    async fn update(&self, changes: Value) -> Result<Value> {
        validate_changes(&changes).map_err(|error| anyhow!(error.to_string()))?;
        if model_changes_need_catalogue(&changes) {
            validate_model_changes(&changes, &self.models().await?)?;
        }
        if self.runtime.account {
            let updated = self
                .api
                .request(
                    Method::PATCH,
                    &format!("{}/configuration", self.path()),
                    Some(&changes),
                )
                .await
                .map_err(|error| anyhow!(error))?;
            return Ok(snapshot(&updated));
        }
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        let id = self.runtime.context.bot_id.clone();
        let updated = on_store(&bots, move |store| store.patch_bot(&id, changes))
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(snapshot(&serde_json::to_value(updated)?))
    }
}
