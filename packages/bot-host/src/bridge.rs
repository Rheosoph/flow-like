//! The tool bridge of a headless turn that runs the bot-tools in this process: a call that asks
//! the owner clears the turn's [`ApprovalGate`] (a standing grant, or the turn parks), a call that
//! changes anything ends the turn's retries, and a stopped turn reads as cancelled. A parked turn
//! is not cancelled: calls already running finish, so their outcomes stay known.

use crate::gate::ApprovalGate;
use async_trait::async_trait;
use flow_like::flow::copilot::{
    HostTools, find_bot_tool_spec,
    platform::PlatformToolBridge,
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
}

fn refusal(tool: &str, code: &str, message: &str) -> String {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
        .to_string()
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
        if !self.host_tools.runs(tool_name) {
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
        if flow_like_bot_tools::spec(tool_name).is_some_and(|spec| spec.mutating) {
            self.mutating.store(true, Ordering::SeqCst);
        }
        self.run_allowed(tool_name, arguments).await
    }

    async fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
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
