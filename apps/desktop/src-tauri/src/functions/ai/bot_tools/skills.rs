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

/// What a package skill's tool runs with on this device: the workspace as its storage and the
/// bot's model access. The package must be installed on this device; nothing is installed here.
pub(crate) struct PackageRunner {
    pub app: AppHandle,
    pub state: Arc<flow_like::state::FlowLikeState>,
    pub token: Option<String>,
    pub workspace: Option<String>,
    pub workspace_store: Option<flow_like::flow_like_storage::files::store::FlowLikeStore>,
    pub turn_id: String,
    pub user_id: String,
    pub usage: flow_like::models::llm::ModelUsageContext,
}

struct PackageSkills {
    base: Arc<dyn SkillHost>,
    runner: PackageRunner,
}

impl PackageSkills {
    async fn nodes(&self, skill: &BotSkill) -> Result<Vec<flow_like_wasm::WasmNodeLogic>> {
        let package_id = skill
            .source
            .package_id()
            .ok_or_else(|| anyhow!("Skill '{}' ships no package tools", skill.name))?;
        let client = crate::state::TauriRegistryState::get_client(&self.runner.app).await?;
        let engine = crate::state::TauriWasmEngineState::construct(&self.runner.app)?;
        let nodes = client.load_nodes(package_id, engine).await.map_err(|error| {
            anyhow!("The package '{package_id}' is not installed on this device: {error}")
        })?;
        Ok(nodes)
    }
}

#[async_trait]
impl SkillHost for PackageSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        self.base.get(name).await
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        self.base.propose(request).await
    }

    async fn tool_specs(&self, skill: &BotSkill) -> Result<Vec<flow_like_bot_tools::SkillToolSpec>> {
        if skill.source.package_id().is_none() {
            return Ok(Vec::new());
        }
        let mut specs = Vec::new();
        for node in self.nodes(skill).await? {
            let entry = node.tool_entry().await.map_err(|error| anyhow!("{error}"))?;
            if !skill.tools.contains(&entry.name) {
                continue;
            }
            let info = flow_like_wasm::skill_call::tool_info(&entry);
            let (network, writes) = gating(node.security().capabilities);
            specs.push(flow_like_bot_tools::SkillToolSpec {
                name: info.name,
                description: info.description,
                input_schema: info.input_schema,
                network,
                writes,
            });
        }
        Ok(specs)
    }

    async fn call(&self, skill: &BotSkill, tool: &str, arguments: Value) -> Result<Value> {
        let workspace = self
            .runner
            .workspace
            .clone()
            .ok_or_else(|| anyhow!("The bot has no workspace for the tool to use"))?;
        for node in self.nodes(skill).await? {
            let entry = node.tool_entry().await.map_err(|error| anyhow!("{error}"))?;
            if entry.name != tool || !skill.tools.iter().any(|known| known == tool) {
                continue;
            }
            return node
                .run_headless(
                    flow_like_wasm::HeadlessRun {
                        app_state: self.runner.state.clone(),
                        token: self.runner.token.clone(),
                        workspace,
                        workspace_store: self.runner.workspace_store.clone(),
                        turn_id: self.runner.turn_id.clone(),
                        user_id: self.runner.user_id.clone(),
                        usage: Some(self.runner.usage.clone()),
                    },
                    &arguments,
                )
                .await
                .map_err(|error| anyhow!(error));
        }
        Err(anyhow!("The package has no tool '{tool}'"))
    }
}

/// Whether a node reaches the network or writes: those tools ask the owner first.
pub(crate) fn gating(capabilities: flow_like_wasm::WasmCapabilities) -> (bool, bool) {
    use flow_like_wasm::WasmCapabilities as Caps;
    (
        capabilities.intersects(Caps::NETWORK_ALL),
        capabilities.intersects(
            Caps::STORAGE_WRITE | Caps::STORAGE_DELETE | Caps::DATABASE_WRITE | Caps::HTTP_WRITE,
        ),
    )
}

/// The skill host for `bot_id`: the local store's when this device holds the bot, else the hub's,
/// with package tools run in this process.
pub(crate) async fn skills_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
    runner: PackageRunner,
) -> Arc<dyn SkillHost> {
    let base: Arc<dyn SkillHost> = if super::held_locally(app, bot_id).await {
        Arc::new(LocalSkills {
            app: app.clone(),
            bot_id: bot_id.to_string(),
        })
    } else {
        Arc::new(AccountSkills {
            api,
            bot_id: bot_id.to_string(),
        })
    };
    Arc::new(PackageSkills { base, runner })
}

#[cfg(test)]
mod tests {
    use super::gating;
    use flow_like_wasm::WasmCapabilities as Caps;

    #[test]
    fn a_tool_asks_when_it_reaches_the_network_or_writes() {
        assert_eq!(gating(Caps::STORAGE_READ), (false, false));
        assert_eq!(gating(Caps::HTTP_GET), (true, false));
        assert_eq!(gating(Caps::STORAGE_WRITE), (false, true));
    }
}
