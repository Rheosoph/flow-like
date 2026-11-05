//! Skill tools on the executor: approved instructions and package tools, with the workspace
//! as storage and the bot's turn as the run. The API persists validated proposals.
//! The package gets no OAuth tokens; its network is limited to the manifest's allowed hosts.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like::models::llm::ModelUsageContext;
use flow_like::state::FlowLikeState;
use flow_like_bot_tools::{SkillHost, SkillToolSpec};
use flow_like_bot_tools_host::SkillCallJob;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::bot_skill::{BotSkill, CreateBotSkill, SkillSource, SkillStatus};
use flow_like_types::{Result, anyhow, tokio::sync::OnceCell};
use flow_like_wasm::{HeadlessRun, WasmCapabilities, WasmNodeLogic};
use serde_json::Value;

pub(crate) struct ExecutorSkills {
    job: Option<SkillCallJob>,
    bot_id: String,
    proposal: Mutex<Option<CreateBotSkill>>,
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
        job: Option<SkillCallJob>,
        bot_id: String,
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
            bot_id,
            proposal: Mutex::new(None),
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
                let job = self
                    .job
                    .as_ref()
                    .ok_or_else(|| anyhow!("This job has no skill"))?;
                let package = job
                    .package
                    .as_ref()
                    .ok_or_else(|| anyhow!("This skill ships no package tools"))?;
                crate::wasm_loader::load_skill_nodes(&job.package_id, package)
                    .await
                    .map_err(|error| anyhow!("{error}"))
            })
            .await
    }

    pub(crate) fn take_proposal(&self) -> Option<CreateBotSkill> {
        self.proposal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
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
        Ok(self
            .job
            .as_ref()
            .filter(|job| job.skill.name == name)
            .map(|job| job.skill.clone()))
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        // The tool validated the text. The API commits this proposal before releasing its
        // success output, so storage limits and ownership are checked against current state.
        *self.proposal.lock().unwrap_or_else(|error| error.into_inner()) = Some(request.clone());
        Ok(BotSkill {
            id: String::new(),
            bot_id: self.bot_id.clone(),
            name: request.name,
            description: request.description,
            body: request.body,
            source: SkillSource::BotProposed,
            status: SkillStatus::Pending,
            tools: Vec::new(),
            updated_at: String::new(),
        })
    }

    async fn tool_specs(&self, skill: &BotSkill) -> Result<Vec<SkillToolSpec>> {
        if skill.source.package_id().is_none() {
            return Ok(Vec::new());
        }
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
