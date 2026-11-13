//! Flux's management of its owner's bots: the account's through the hub, signed out this
//! device's. Each call is the one the Bots tab makes. The bridge has obtained the owner's
//! approval before a create, update or delete.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::flow::copilot::{
    model_changes_need_catalogue, profile_models, validate_model_changes,
};
use flow_like_bot_tools::crew::{CrewApp, CrewBackend};
use flow_like_local_bots::{CreateBotRequest, LocalBotStore};
use flow_like_types::{Result, anyhow, bot_restrictions::BotRestrictions};
use reqwest::Method;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::{
    ai::{native_bot_tools::BotToolRuntime, native_platform_tools::segment},
    local_bots::{LocalBots, delete_local_bot, on_store},
};

pub(crate) struct DesktopCrew {
    app: AppHandle,
    runtime: Arc<BotToolRuntime>,
    api: Arc<dyn TaskApi>,
}

impl DesktopCrew {
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

    async fn hub(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        self.api
            .request(method, path, body)
            .await
            .map_err(|error| anyhow!(error))
    }

    fn local_bots(&self) -> Result<tauri::State<'_, LocalBots>> {
        self.app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))
    }

    async fn local<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&LocalBotStore) -> flow_like_local_bots::Result<T> + Send + 'static,
    {
        on_store(&*self.local_bots()?, work)
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
        if let Some(refusal) =
            crate::functions::ai::native_bot_tools::attach_refusal(&self.app, attach).await
        {
            return Err(anyhow!(
                "{}: {refusal}",
                flow_like_types::bot_restrictions::APP_REFUSES_BOTS
            ));
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

fn local_app(app: flow_like_local_bots::BotApp) -> CrewApp {
    CrewApp {
        app_id: app.app_id,
        restrictions: app.restrictions,
    }
}

/// The hub's `BotAppList` as the apps Flux reads.
fn hub_apps(list: &Value) -> Result<Vec<CrewApp>> {
    Ok(serde_json::from_value(
        list.get("apps")
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
    )?)
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

    async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<CrewApp>)> {
        if self.runtime.account {
            let bot = self.hub(Method::GET, &bot_path(bot_id), None).await?;
            let apps = self
                .hub(Method::GET, &format!("{}/apps", bot_path(bot_id)), None)
                .await?;
            return Ok((bot, hub_apps(&apps)?));
        }
        let id = bot_id.to_string();
        let (bot, apps) = self
            .local(move |store| Ok((store.get_bot(&id)?, store.bot_apps(&id).unwrap_or_default())))
            .await?;
        let bot = bot.ok_or_else(|| anyhow!("You have no bot '{bot_id}' on this device"))?;
        Ok((
            serde_json::to_value(bot)?,
            apps.into_iter().map(local_app).collect(),
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
                serde_json::to_value(
                    self.local(move |store| store.patch_bot(&id, request))
                        .await?,
                )?
            }
            None => self.bot(bot_id).await?.0,
        };
        self.move_apps(bot_id, attach, detach).await?;
        Ok(bot)
    }

    async fn restrict(
        &self,
        bot_id: &str,
        app_id: &str,
        restrictions: &BotRestrictions,
    ) -> Result<()> {
        if self.runtime.account {
            let body = serde_json::to_value(restrictions)?;
            self.hub(
                Method::PUT,
                &format!("{}/restrictions", app_path(bot_id, app_id)),
                Some(&body),
            )
            .await?;
            return Ok(());
        }
        let (id, app, restrictions) =
            (bot_id.to_string(), app_id.to_string(), restrictions.clone());
        self.local(move |store| store.set_app_restrictions(&id, &app, restrictions))
            .await?;
        Ok(())
    }

    async fn delete(&self, bot_id: &str) -> Result<()> {
        if self.runtime.account {
            self.hub(Method::DELETE, &bot_path(bot_id), None).await?;
            return Ok(());
        }
        delete_local_bot(&self.app, &*self.local_bots()?, bot_id)
            .await
            .map_err(|error| anyhow!("Deleting bot '{bot_id}' on this device failed: {error}"))
    }

    async fn devices(&self) -> Result<Vec<Value>> {
        if !self.runtime.account {
            return Ok(Vec::new());
        }
        let devices = self.hub(Method::GET, "devices", None).await?;
        Ok(devices.as_array().cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::bot_restrictions::BotPreset;
    use serde_json::json;

    #[test]
    fn paths_keep_each_id_one_segment() {
        assert_eq!(bot_path("bot_1"), "bots/bot_1");
        assert_eq!(app_path("bot_1", "a/b"), "bots/bot_1/apps/a%2Fb");
    }

    #[test]
    fn hub_apps_carry_their_restrictions() {
        let restricted = BotPreset::ReadOnly.restrictions();
        let apps = hub_apps(&json!({"apps": [
            {"app_id": "a", "added_at": "2026-11-13T00:00:00Z", "owner_role": "Owner", "restrictions": restricted},
            {"app_id": "b", "added_at": "2026-11-13T00:00:00Z", "owner_role": null, "restrictions": {}},
        ]}))
        .unwrap();
        assert_eq!(apps[0].restrictions, restricted);
        assert!(apps[1].restrictions.is_unrestricted());
        assert!(hub_apps(&json!({})).unwrap().is_empty());
    }
}
