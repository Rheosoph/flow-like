//! `skill_call` / `skill_run` on the executor: the package a bot's skill ships, loaded like a
//! board run loads it, with the workspace as its storage and the bot's turn as its run.
//! The package gets no OAuth tokens; its network is limited to the manifest's allowed hosts.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::models::llm::ModelUsageContext;
use flow_like::state::FlowLikeState;
use flow_like_bot_tools::{SkillHost, SkillToolSpec};
use flow_like_bot_tools_host::SkillCallJob;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::bot_skill::{BotSkill, CreateBotSkill};
use flow_like_types::{Result, anyhow, tokio::sync::OnceCell};
use flow_like_wasm::{HeadlessRun, WasmCapabilities, WasmNodeLogic};
use serde_json::Value;

pub(crate) struct ExecutorSkills {
    job: SkillCallJob,
    nodes: OnceCell<Vec<WasmNodeLogic>>,
    state: Arc<FlowLikeState>,
    token: String,
    workspace: String,
    workspace_store: Option<FlowLikeStore>,
    turn_id: String,
    user_id: String,
    usage: ModelUsageContext,
}

impl ExecutorSkills {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        job: SkillCallJob,
        state: Arc<FlowLikeState>,
        token: String,
        workspace: String,
        workspace_store: Option<FlowLikeStore>,
        turn_id: String,
        user_id: String,
        usage: ModelUsageContext,
    ) -> Self {
        Self {
            job,
            nodes: OnceCell::new(),
            state,
            token,
            workspace,
            workspace_store,
            turn_id,
            user_id,
            usage,
        }
    }

    async fn nodes(&self) -> Result<&Vec<WasmNodeLogic>> {
        self.nodes
            .get_or_try_init(|| async {
                crate::wasm_loader::load_skill_nodes(&self.job.package_id, &self.job.package)
                    .await
                    .map_err(|error| anyhow!("{error}"))
            })
            .await
    }
}

/// Whether a node reaches the network or writes: those tools ask the owner first.
pub(crate) fn gated(capabilities: WasmCapabilities) -> (bool, bool) {
    let network = capabilities.intersects(WasmCapabilities::NETWORK_ALL);
    let writes = capabilities.intersects(
        WasmCapabilities::STORAGE_WRITE
            | WasmCapabilities::STORAGE_DELETE
            | WasmCapabilities::DATABASE_WRITE
            | WasmCapabilities::HTTP_WRITE,
    );
    (network, writes)
}

#[async_trait]
impl SkillHost for ExecutorSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        Ok((self.job.skill.name == name).then(|| self.job.skill.clone()))
    }

    async fn propose(&self, _request: CreateBotSkill) -> Result<BotSkill> {
        Err(anyhow!("A skill is proposed in the API, not on the executor"))
    }

    async fn tool_specs(&self, skill: &BotSkill) -> Result<Vec<SkillToolSpec>> {
        let mut specs = Vec::new();
        for node in self.nodes().await? {
            let entry = node.tool_entry().await.map_err(|error| anyhow!("{error}"))?;
            if !skill.tools.contains(&entry.name) {
                continue;
            }
            let info = flow_like_wasm::skill_call::tool_info(&entry);
            let (network, writes) = gated(node.security().capabilities);
            specs.push(SkillToolSpec {
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
        for node in self.nodes().await? {
            let entry = node.tool_entry().await.map_err(|error| anyhow!("{error}"))?;
            if entry.name != tool || !skill.tools.iter().any(|known| known == tool) {
                continue;
            }
            return node
                .run_headless(
                    HeadlessRun {
                        app_state: self.state.clone(),
                        token: Some(self.token.clone()),
                        workspace: self.workspace.clone(),
                        workspace_store: self.workspace_store.clone(),
                        turn_id: self.turn_id.clone(),
                        user_id: self.user_id.clone(),
                        usage: Some(self.usage.clone()),
                    },
                    &arguments,
                )
                .await
                .map_err(|error| anyhow!(error));
        }
        Err(anyhow!("The package has no tool '{tool}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_and_writing_capabilities_gate_a_tool() {
        assert_eq!(gated(WasmCapabilities::STORAGE_READ | WasmCapabilities::CACHE_ALL), (false, false));
        assert_eq!(gated(WasmCapabilities::HTTP_GET), (true, false));
        assert_eq!(gated(WasmCapabilities::STORAGE_WRITE), (false, true));
        assert_eq!(gated(WasmCapabilities::DATABASE_WRITE | WasmCapabilities::DNS), (true, true));
    }
}
