//! The tool bridge of a headless turn that runs the bot-tools in this process: a call that asks
//! the owner clears the turn's [`ApprovalGate`] (a standing grant, or it joins the turn's card),
//! a call that changes anything ends the turn's retries, and a stopped turn reads as cancelled. A
//! parked turn is not cancelled: calls already running finish, so their outcomes stay known.

use crate::{gate::ApprovalGate, hub::TurnHub};
use async_trait::async_trait;
use flow_like::flow::copilot::{
    BotToolHost, HostTools, find_bot_tool_spec,
    platform::{Clearance, PlatformToolBridge},
    tool_outcome_output,
    tool_spec::{MEMORY_STORE_TOOL, ToolEffect, resolve_tool_approval},
};
use flow_like_bot_tools::ToolEnv;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const MEMORY_WRITE_TOOLS: [&str; 3] = [MEMORY_STORE_TOOL, "memory_store", "memory_save"];

/// Builds the tool environment of one call; the flag says the owner allowed documents above the
/// cost limits for it.
pub type EnvFactory = Arc<dyn Fn(bool) -> Arc<dyn ToolEnv> + Send + Sync>;

pub struct ToolEnvBridge {
    env: EnvFactory,
    host_tools: HostTools,
    gate: Arc<ApprovalGate>,
    mutating: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    hub_apps: Option<HubApps>,
}

/// Where a turn's hosted app tools run: the hub, as the bot.
struct HubApps {
    hub: Arc<dyn TurnHub>,
    bot_id: String,
    turn_id: String,
}

fn refusal(tool: &str, code: &str, message: &str) -> String {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
        .to_string()
}

fn is_hosted_app_tool(tool: &str) -> bool {
    BotToolHost::Hosted.runs(tool) && !BotToolHost::CoreOnly.runs(tool)
}

fn changes_something(tool: &str) -> bool {
    MEMORY_WRITE_TOOLS.contains(&tool)
        || find_bot_tool_spec(tool)
            .is_some_and(|spec| spec.approval.effect() != ToolEffect::ReadOnly)
}

impl ToolEnvBridge {
    pub fn new(
        env: EnvFactory,
        host_tools: HostTools,
        gate: Arc<ApprovalGate>,
        mutating: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    ) -> Self {
        Self {
            env,
            host_tools,
            gate,
            mutating,
            stopped,
            hub_apps: None,
        }
    }

    pub fn with_hub_apps(mut self, hub: Arc<dyn TurnHub>, bot_id: &str, turn_id: &str) -> Self {
        self.hub_apps = Some(HubApps {
            hub,
            bot_id: bot_id.to_owned(),
            turn_id: turn_id.to_owned(),
        });
        self
    }

    async fn run_on_hub(&self, apps: &HubApps, tool: &str, args: Value) -> String {
        match apps
            .hub
            .app_tool(&apps.bot_id, &apps.turn_id, tool, args)
            .await
        {
            Ok(output) => output,
            Err(failure) => refusal(
                tool,
                "bot_tool_hub_failed",
                &format!("The hub could not run '{tool}' for this turn: {failure}"),
            ),
        }
    }

    async fn run_allowed(&self, tool: &str, args: Value) -> String {
        let expensive = self.gate.expensive_documents_allowed(tool, &args).await;
        let env = (self.env)(expensive);
        let output =
            tool_outcome_output(&flow_like_bot_tools::run(tool, args.clone(), env.as_ref()).await);
        match serde_json::from_str::<Value>(&output) {
            Ok(value) if !expensive => self.gate.park_for_cost(tool, &args, value).to_string(),
            _ => output,
        }
    }
}

#[async_trait]
impl PlatformToolBridge for ToolEnvBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        let hub_app = self
            .hub_apps
            .as_ref()
            .filter(|_| is_hosted_app_tool(tool_name));
        if hub_app.is_none() && !self.host_tools.runs(tool_name) {
            if changes_something(tool_name) {
                self.mutating.store(true, Ordering::SeqCst);
            }
            return refusal(
                tool_name,
                "bot_tool_unavailable_headless",
                "This host does not run this tool while the bot answers on its own.",
            );
        }
        let Some(spec) = find_bot_tool_spec(tool_name) else {
            return refusal(
                tool_name,
                "bot_tool_unknown",
                "This tool has no spec on this host, so its approval cannot be checked and it is not run.",
            );
        };
        let approval = resolve_tool_approval(&spec, &arguments);
        if approval.kind != "none"
            && let Err(refused) = self
                .gate
                .clear(tool_name, &arguments, None, &approval.description)
                .await
        {
            return refused.to_string();
        }
        if let Some(apps) = hub_app {
            if changes_something(tool_name) {
                self.mutating.store(true, Ordering::SeqCst);
            }
            return self.run_on_hub(apps, tool_name, arguments).await;
        }
        if flow_like_bot_tools::spec(tool_name).is_some_and(|spec| spec.mutating) {
            self.mutating.store(true, Ordering::SeqCst);
        }
        self.run_allowed(tool_name, arguments).await
    }

    /// The gate's answer under the rule [`Self::call`] applies; a call this host refuses before
    /// any approval needs no question.
    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        let runs_here = (self.hub_apps.is_some() && is_hosted_app_tool(tool_name))
            || self.host_tools.runs(tool_name);
        let Some(spec) = find_bot_tool_spec(tool_name).filter(|_| runs_here) else {
            return Clearance::Clear;
        };
        let approval = resolve_tool_approval(&spec, arguments);
        if approval.kind == "none" {
            return Clearance::Clear;
        }
        self.gate
            .clearance(tool_name, arguments, None, &approval.description)
            .await
    }

    async fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    async fn bot_secret(&self, name: &str) -> Option<String> {
        (self.env)(false).secret(name).await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::copilot::linked_tool_names;

    #[test]
    fn every_linked_tool_has_a_spec_so_the_approval_check_never_skips() {
        for name in linked_tool_names() {
            assert!(
                find_bot_tool_spec(name).is_some(),
                "{name} is linked but has no core spec"
            );
        }
    }
}
