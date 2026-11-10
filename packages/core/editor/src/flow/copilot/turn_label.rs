//! What a bot turn has seen and what it cost beyond its own clock.
//!
//! [`TurnLabel`] is the most sensitive classification any tool result of the turn reported, the
//! bot's own and its sub-tasks' alike. Outbound tools check it ([`LabelBridge`]): a turn that
//! handled data above the bot's own classification may not send anything out. [`TurnCompute`]
//! carries the compute time of parallel sub-tasks to the host that settles the turn.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use flow_like_types::data_classification::{DataClassification, DataSensitivity};
use serde_json::{Value, json};

use super::bot_run::BotRunContext;
use super::bot_talk::{ASK_BOT_TOOL, BotTalk};
use super::platform::PlatformToolBridge;

/// Tools that send what they were given out of the platform or to another principal: mail,
/// web and connector requests, images, a person or another bot, a shell, code, a browser, the
/// computer and skills that run their own tools.
const OUTBOUND_TOOLS: [&str; 20] = [
    "email_send",
    "email_reply",
    "connector_call",
    "connector_request",
    "image_generate",
    "web_request",
    "message_person",
    ASK_BOT_TOOL,
    "shell_run",
    "code_run",
    "browser_open",
    "browser_act",
    "computer_act",
    "computer_launch",
    "skill_call",
    "skill_run",
    "internet_search",
    "open_url",
    "http_request",
    "research_agent",
];

#[derive(Default)]
struct ComputeCells {
    child_ms: AtomicU64,
    waited_ms: AtomicU64,
}

/// Compute time of sub-tasks beyond the turn's own wall clock. A host settles the turn for
/// its elapsed time plus [`Self::extra_millis`], so parallel children do not run at a discount.
#[derive(Clone, Default)]
pub struct TurnCompute(Arc<ComputeCells>);

impl std::fmt::Debug for TurnCompute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnCompute")
            .field("extra_ms", &self.extra_millis())
            .finish()
    }
}

impl PartialEq for TurnCompute {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for TurnCompute {}

impl TurnCompute {
    pub(super) fn add_child(&self, millis: u64) {
        self.0.child_ms.fetch_add(millis, Ordering::Relaxed);
    }

    pub(super) fn add_waited(&self, millis: u64) {
        self.0.waited_ms.fetch_add(millis, Ordering::Relaxed);
    }

    /// Summed time of all sub-tasks minus the time the turn spent waiting for them.
    pub fn extra_millis(&self) -> i64 {
        let child = self.0.child_ms.load(Ordering::Relaxed);
        let waited = self.0.waited_ms.load(Ordering::Relaxed);
        child.saturating_sub(waited).min(i64::MAX as u64) as i64
    }
}

pub(super) struct TurnLabel {
    level: Mutex<DataSensitivity>,
    own: Option<DataClassification>,
    talk: Option<Arc<BotTalk>>,
}

impl TurnLabel {
    pub(super) fn new(bot: &BotRunContext, talk: Option<Arc<BotTalk>>) -> Arc<Self> {
        Arc::new(Self {
            level: Mutex::new(DataSensitivity::default()),
            own: bot.classification.classification,
            talk,
        })
    }

    pub(super) fn get(&self) -> DataSensitivity {
        *self
            .level
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn raise(&self, seen: DataSensitivity) {
        if seen.is_empty() {
            return;
        }
        {
            let mut level = self
                .level
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *level = level.combine(seen);
        }
        if let Some(talk) = &self.talk {
            talk.raise_taint(seen);
        }
    }

    pub(super) fn observe(&self, output: &str) {
        let Ok(result) = serde_json::from_str::<Value>(output) else {
            return;
        };
        let level = result
            .get("classification")
            .and_then(Value::as_str)
            .and_then(DataClassification::parse);
        if let Some(level) = level {
            self.raise(DataSensitivity {
                classification: Some(level),
                pii: false,
            });
        }
    }

    /// The refusal for an outbound call once the turn handled data above the bot's own level.
    pub(super) fn refusal(&self, tool_name: &str) -> Option<String> {
        if !OUTBOUND_TOOLS.contains(&tool_name) {
            return None;
        }
        let seen = self.get().classification?;
        if seen <= self.own.unwrap_or(DataClassification::Public) {
            return None;
        }
        Some(
            json!({
                "status": "error",
                "tool": tool_name,
                "code": "turn_label_outbound_refused",
                "retryable": false,
                "message": format!(
                    "This turn handled {} data, above this bot's own level, so {tool_name} may not send anything out. Nothing was run.",
                    seen.as_str()
                ),
            })
            .to_string(),
        )
    }
}

/// Meters every result into the turn's label and refuses outbound calls the label forbids.
pub(super) struct LabelBridge {
    pub(super) label: Arc<TurnLabel>,
    pub(super) inner: Arc<dyn PlatformToolBridge>,
}

#[async_trait]
impl PlatformToolBridge for LabelBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if let Some(refusal) = self.label.refusal(tool_name) {
            return refusal;
        }
        let output = self.inner.call(tool_name, arguments).await;
        self.label.observe(&output);
        output
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::tool_spec::find_global_tool_spec;

    fn label(own: Option<DataClassification>) -> Arc<TurnLabel> {
        Arc::new(TurnLabel {
            level: Mutex::new(DataSensitivity::default()),
            own,
            talk: None,
        })
    }

    #[test]
    fn every_outbound_tool_is_a_tool_that_exists() {
        let bot_tools: Vec<String> = flow_like_bot_tools::specs()
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect();
        for name in OUTBOUND_TOOLS {
            assert!(
                bot_tools.iter().any(|known| known == name)
                    || find_global_tool_spec(name).is_some()
                    || name == ASK_BOT_TOOL,
                "{name} is listed as outbound but no tool has that name"
            );
        }
    }

    #[test]
    fn a_label_above_the_bots_level_refuses_every_outbound_tool_and_only_those() {
        let label = label(Some(DataClassification::Internal));
        label.raise(DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        });
        for name in OUTBOUND_TOOLS {
            assert!(label.refusal(name).is_some(), "{name} was not refused");
        }
        assert!(label.refusal("file_read").is_none());
    }

    #[test]
    fn an_answer_marked_confidential_raises_the_label() {
        let label = label(Some(DataClassification::Internal));
        label.observe(r#"{"status":"ok","tool":"ask_bot","classification":"confidential"}"#);
        assert!(label.refusal("email_send").is_some());
    }
}
