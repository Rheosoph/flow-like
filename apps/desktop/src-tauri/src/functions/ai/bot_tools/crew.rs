//! Flux's management of its owner's bots: the account's through the hub, signed out this
//! device's. The bridge has obtained the owner's approval before a create or update.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::flow::copilot::{
    model_changes_need_catalogue, profile_models, validate_model_changes,
};
use flow_like_bot_tools::crew::CrewBackend;
use flow_like_local_bots::{CreateBotRequest, LocalBotStore};
use flow_like_types::{Result, anyhow};
use reqwest::Method;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::{
    ai::{native_bot_tools::BotToolRuntime, native_platform_tools::segment},
    local_bots::{LocalBots, on_store},
};

pub(crate) struct DesktopCrew {
    app: AppHandle,
    runtime: Arc<BotToolRuntime>,
    api: Arc<dyn TaskApi>,
}

impl DesktopCrew {
    pub(crate) fn new(app: &AppHandle, runtime: Arc<BotToolRuntime>, api: Arc<dyn TaskApi>) -> Self {
        Self {
            app: app.clone(),
            runtime,
            api,
        }
    }

    async fn hub(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        self.api
            .request(method, path, body)
            .await
            .map_err(|error| anyhow!(error))
    }

    async fn local<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&LocalBotStore) -> flow_like_local_bots::Result<T> + Send + 'static,
    {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        on_store(&bots, work)
            .await
            .map_err(|error| anyhow!(error.to_string()))
    }

    async fn check_models(&self, request: &Value) -> Result<()> {
        if !model_changes_need_catalogue(request) {
            return Ok(());
        }
        let profile = self.runtime.profile.as_ref().ok_or_else(|| {
            anyhow!("This turn has no profile. Select a profile before mapping models.")
        })?;
        let catalogue = profile_models(
            profile,
            &self.runtime.state,
            self.runtime.context.decision_hosted_only,
        )
        .await?;
        validate_model_changes(request, &catalogue)
    }

    async fn move_apps(&self, bot_id: &str, attach: &[String], detach: &[String]) -> Result<()> {
        if self.runtime.account {
            for app_id in attach {
                self.hub(Method::PUT, &app_path(bot_id, app_id), None)
                    .await
                    .map_err(|error| anyhow!("Attaching app '{app_id}' failed: {error}"))?;
            }
            for app_id in detach {
                self.hub(Method::DELETE, &app_path(bot_id, app_id), None)
                    .await
                    .map_err(|error| anyhow!("Detaching app '{app_id}' failed: {error}"))?;
            }
            return Ok(());
        }
        let (id, attach, detach) = (bot_id.to_string(), attach.to_vec(), detach.to_vec());
        self.local(move |store| {
            for app_id in &attach {
                store.attach_app(&id, app_id)?;
            }
            for app_id in &detach {
                store.detach_app(&id, app_id)?;
            }
            Ok(())
        })
        .await
    }
}

fn bot_path(bot_id: &str) -> String {
    format!("bots/{}", segment(bot_id))
}

fn app_path(bot_id: &str, app_id: &str) -> String {
    format!("{}/apps/{}", bot_path(bot_id), segment(app_id))
}

#[async_trait]
impl CrewBackend for DesktopCrew {
    async fn bots(&self) -> Result<Vec<Value>> {
        if self.runtime.account {
            let list = self.hub(Method::GET, "bots", None).await?;
            return Ok(list["bots"].as_array().cloned().unwrap_or_default());
        }
        let bots = self.local(|store| store.list_bots()).await?;
        bots.into_iter()
            .map(|bot| Ok(serde_json::to_value(bot)?))
            .collect()
    }

    async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<String>)> {
        if self.runtime.account {
            let bot = self.hub(Method::GET, &bot_path(bot_id), None).await?;
            let apps = self
                .hub(Method::GET, &format!("{}/apps", bot_path(bot_id)), None)
                .await?;
            let ids = apps["apps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|app| app["app_id"].as_str().map(str::to_string))
                .collect();
            return Ok((bot, ids));
        }
        let id = bot_id.to_string();
        let (bot, apps) = self
            .local(move |store| Ok((store.get_bot(&id)?, store.bot_apps(&id).unwrap_or_default())))
            .await?;
        let bot = bot.ok_or_else(|| anyhow!("You have no bot '{bot_id}' on this device"))?;
        Ok((
            serde_json::to_value(bot)?,
            apps.into_iter().map(|app| app.app_id).collect(),
        ))
    }

    fn profile_id(&self) -> Option<String> {
        self.runtime
            .profile
            .as_ref()
            .map(|profile| profile.id.clone())
            .filter(|id| !id.is_empty())
    }

    async fn create(&self, request: Value, app_ids: &[String]) -> Result<Value> {
        self.check_models(&request).await?;
        let bot = if self.runtime.account {
            self.hub(Method::POST, "bots", Some(&request)).await?
        } else {
            let request: CreateBotRequest = serde_json::from_value(request)?;
            serde_json::to_value(self.local(move |store| store.create_bot(request)).await?)?
        };
        let bot_id = bot["id"]
            .as_str()
            .ok_or_else(|| anyhow!("The new bot came back without an id"))?
            .to_string();
        self.move_apps(&bot_id, app_ids, &[])
            .await
            .map_err(|error| anyhow!("Bot '{bot_id}' was created, but {error}"))?;
        Ok(bot)
    }

    async fn update(
        &self,
        bot_id: &str,
        request: Option<Value>,
        attach: &[String],
        detach: &[String],
    ) -> Result<Value> {
        if let Some(request) = &request {
            self.check_models(request).await?;
        }
        let bot = match request {
            Some(request) if self.runtime.account => {
                self.hub(Method::PATCH, &bot_path(bot_id), Some(&request))
                    .await?
            }
            Some(request) => {
                let id = bot_id.to_string();
                serde_json::to_value(self.local(move |store| store.patch_bot(&id, request)).await?)?
            }
            None => self.bot(bot_id).await?.0,
        };
        self.move_apps(bot_id, attach, detach).await?;
        Ok(bot)
    }
}
