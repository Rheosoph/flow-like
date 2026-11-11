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
use super::bot_talk::{ASK_BOT_TOOL, BotTalk, effective_level};
use super::platform::PlatformToolBridge;

/// Tools that send what they were given out of the platform or to another principal: mail,
/// web and connector requests, images, a person or another bot, a shell, code, a browser, the
/// computer and skills that run their own tools.
const OUTBOUND_TOOLS: [&str; 21] = [
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
    "archive_lookup",
    "http_request",
    "research_agent",
];

/// Tools that send something out only when a call sets `url`: a download names its address to
/// that host, and a linked attachment makes every reader's app load it.
const OUTBOUND_WITH_URL: [&str; 2] = ["file_save", "file_attach"];

fn is_outbound(tool_name: &str, args: &Value) -> bool {
    OUTBOUND_TOOLS.contains(&tool_name)
        || (OUTBOUND_WITH_URL.contains(&tool_name)
            && args
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.trim().is_empty()))
}

/// Mail and webhook content comes from outside the platform. The classification scale has no
/// level above `public` for it, so it marks the turn as having read outside content without
/// raising the level.
pub const EXTERNAL_CONTENT: DataSensitivity = DataSensitivity {
    classification: Some(DataClassification::Public),
    pii: false,
};

/// What a turn has read before its first tool call, as its host knows it.
#[derive(Debug, Clone, Default)]
pub struct TurnReads {
    /// The labels of the bots whose messages are in the history.
    pub bot_senders: Vec<DataSensitivity>,
    /// The taint recorded on messages the bot or other bots wrote earlier.
    pub recorded: DataSensitivity,
    /// A mail or webhook trigger is in the history.
    pub external_trigger: bool,
    /// A person's context snapshot is in the turn.
    pub personal_context: bool,
}

impl TurnReads {
    /// The label a turn starts with: the strictest of what it reads.
    pub fn seed(&self) -> DataSensitivity {
        let external = if self.external_trigger {
            EXTERNAL_CONTENT
        } else {
            DataSensitivity::default()
        };
        let personal = DataSensitivity {
            classification: None,
            pii: self.personal_context,
        };
        DataSensitivity::combined(self.bot_senders.iter().copied())
            .combine(self.recorded)
            .combine(external)
            .combine(personal)
    }
}

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
    own: DataClassification,
    talk: Option<Arc<BotTalk>>,
}

impl TurnLabel {
    pub(super) fn new(bot: &BotRunContext, talk: Option<Arc<BotTalk>>) -> Arc<Self> {
        let label = Arc::new(Self {
            level: Mutex::new(DataSensitivity::default()),
            own: effective_level(&bot.classification),
            talk,
        });
        label.raise(bot.seed_taint);
        label
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
    pub(super) fn refusal(&self, tool_name: &str, args: &Value) -> Option<String> {
        if !is_outbound(tool_name, args) {
            return None;
        }
        let seen = self.get().classification?;
        if seen <= self.own {
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
        if let Some(refusal) = self.label.refusal(tool_name, &arguments) {
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

    fn label(own: DataClassification) -> Arc<TurnLabel> {
        Arc::new(TurnLabel {
            level: Mutex::new(DataSensitivity::default()),
            own,
            talk: None,
        })
    }

    #[test]
    fn an_unclassified_bot_counts_as_internal_like_the_answers_it_reads() {
        let label = TurnLabel::new(&BotRunContext::general(), None);
        label.observe(r#"{"status":"ok","tool":"ask_bot","classification":"internal"}"#);
        assert!(label.refusal("internet_search", &json!({})).is_none());
        label.raise(DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        });
        assert!(label.refusal("internet_search", &json!({})).is_some());
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
        let label = label(DataClassification::Internal);
        label.raise(DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        });
        for name in OUTBOUND_TOOLS {
            assert!(
                label.refusal(name, &json!({})).is_some(),
                "{name} was not refused"
            );
        }
        assert!(label.refusal("file_read", &json!({})).is_none());
    }

    #[test]
    fn a_download_or_a_linked_attachment_is_outbound_and_a_local_file_is_not() {
        let label = label(DataClassification::Internal);
        label.raise(DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        });
        for name in OUTBOUND_WITH_URL {
            assert!(
                label
                    .refusal(name, &json!({ "url": "https://h/a.pdf" }))
                    .is_some()
            );
            assert!(label.refusal(name, &json!({ "file": "bot/a.pdf" })).is_none());
            assert!(label.refusal(name, &json!({ "url": " " })).is_none());
        }
    }

    #[test]
    fn a_turn_starts_at_the_strictest_of_what_it_reads() {
        let mut bot = BotRunContext::general();
        bot.seed_taint = TurnReads {
            bot_senders: vec![
                DataSensitivity {
                    classification: Some(DataClassification::Internal),
                    pii: false,
                },
                DataSensitivity {
                    classification: Some(DataClassification::Confidential),
                    pii: false,
                },
            ],
            recorded: DataSensitivity {
                classification: Some(DataClassification::Internal),
                pii: true,
            },
            external_trigger: true,
            personal_context: false,
        }
        .seed();
        let label = TurnLabel::new(&bot, None);
        assert_eq!(
            label.get(),
            DataSensitivity {
                classification: Some(DataClassification::Confidential),
                pii: true,
            }
        );
        assert!(label.refusal("email_send", &json!({})).is_some());
        assert!(TurnLabel::new(&BotRunContext::general(), None).get().is_empty());
    }

    #[test]
    fn mail_and_personal_context_seed_without_raising_the_level() {
        let external = TurnReads {
            external_trigger: true,
            ..Default::default()
        }
        .seed();
        assert_eq!(external, EXTERNAL_CONTENT);
        let personal = TurnReads {
            personal_context: true,
            ..Default::default()
        }
        .seed();
        assert!(personal.pii && personal.classification.is_none());
        assert!(TurnReads::default().seed().is_empty());
    }

    #[test]
    fn an_answer_marked_confidential_raises_the_label() {
        let label = label(DataClassification::Internal);
        label.observe(r#"{"status":"ok","tool":"ask_bot","classification":"confidential"}"#);
        assert!(label.refusal("email_send", &json!({})).is_some());
    }
}
