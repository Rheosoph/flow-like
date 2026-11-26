//! Longer discussions between bots in a room. When a topic needs more rounds than its discussion
//! allows, a bot proposes them with [`DISCUSSION_PROPOSE_TOOL`]: its reply carries the proposal,
//! which the room shows with a button, and a person who allows it sends the grant on their next
//! message ([`flow_like_types::bot_message::DiscussionGrant`]). Until then the bot hands the word
//! to no one.

use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use flow_like_types::{
    bot_message::DiscussionProposal,
    bot_turn::{MAX_LONG_DISCUSSION_ROUNDS, discussion_rounds},
};
use serde_json::{Value, json};

use super::platform::{Clearance, PlatformToolBridge};
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};

pub const DISCUSSION_PROPOSE_TOOL: &str = "discussion_propose";
const MAX_REASON_CHARS: usize = 280;

/// The longer discussion one turn's reply proposes; the host puts it on the reply.
#[derive(Clone, Default)]
pub struct DiscussionProposals(Arc<Mutex<Option<DiscussionProposal>>>);

impl DiscussionProposals {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn proposal(&self) -> Option<DiscussionProposal> {
        self.lock().clone()
    }

    /// Puts the turn's proposal on its reply `body` as `discussion_proposal`.
    pub fn attach(&self, body: &mut Value) {
        if let (Some(proposal), Some(object)) = (self.proposal(), body.as_object_mut()) {
            object.insert("discussion_proposal".to_owned(), json!(proposal));
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<DiscussionProposal>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for DiscussionProposals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DiscussionProposals")
            .field(&self.proposal().map(|proposal| proposal.rounds))
            .finish()
    }
}

impl PartialEq for DiscussionProposals {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for DiscussionProposals {}

pub fn discussion_propose_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: DISCUSSION_PROPOSE_TOOL,
        description: "Ask the people in this room to allow a longer discussion with the other bots. Use it only when the topic clearly needs more rounds than the discussion allows; a round is one message from each of two bots. Your reply then shows the proposal with a button: say in your text why the topic needs it, and don't @mention another bot, since the discussion waits until a person allows it. It runs on with the new budget from their answer.",
        schema: discussion_propose_schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: 10,
    }
}

fn discussion_propose_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["rounds", "reason"],
        "properties": {
            "rounds": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LONG_DISCUSSION_ROUNDS,
                "description": format!("The rounds the whole discussion needs, at most {MAX_LONG_DISCUSSION_ROUNDS}; more than it has now.")
            },
            "reason": {
                "type": "string",
                "maxLength": MAX_REASON_CHARS,
                "description": "One sentence for the people in the room: why the topic needs that many rounds."
            }
        }
    })
}

/// Answers [`DISCUSSION_PROPOSE_TOOL`] in core, keeping the proposal in `proposals`; every other
/// call goes to `inner`.
pub struct DiscussionBridge {
    proposals: DiscussionProposals,
    /// Rounds the turn's discussion has now.
    rounds: u32,
    inner: Arc<dyn PlatformToolBridge>,
}

impl DiscussionBridge {
    /// `discussion_hops` is the bot turns the turn's discussion may reach.
    pub fn new(
        proposals: DiscussionProposals,
        discussion_hops: u32,
        inner: Arc<dyn PlatformToolBridge>,
    ) -> Self {
        Self {
            proposals,
            rounds: discussion_rounds(discussion_hops),
            inner,
        }
    }

    fn propose(&self, arguments: &Value) -> String {
        let rounds = arguments
            .get("rounds")
            .and_then(Value::as_u64)
            .and_then(|rounds| u32::try_from(rounds).ok());
        let Some(rounds) = rounds else {
            return refusal("Pass `rounds`, the rounds the whole discussion needs, as a number");
        };
        if rounds > MAX_LONG_DISCUSSION_ROUNDS {
            return refusal(format!(
                "A discussion has at most {MAX_LONG_DISCUSSION_ROUNDS} rounds; propose {MAX_LONG_DISCUSSION_ROUNDS} or fewer"
            ));
        }
        if rounds <= self.rounds {
            return refusal(format!(
                "This discussion already allows {} rounds; propose more, or go on without asking",
                self.rounds
            ));
        }
        let reason: String = arguments
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_REASON_CHARS)
            .collect();
        if reason.is_empty() {
            return refusal("Pass `reason`: one sentence on why the topic needs more rounds");
        }
        *self.proposals.lock() = Some(DiscussionProposal { rounds, reason });
        json!({
            "status": "ok",
            "rounds": rounds,
            "note": "Your reply shows the proposal with a button to allow it. Say in your text why the topic needs it, and don't @mention another bot: the discussion waits until a person in the room allows it."
        })
        .to_string()
    }
}

fn refusal(message: impl Into<String>) -> String {
    json!({
        "status": "error",
        "tool": DISCUSSION_PROPOSE_TOOL,
        "retryable": true,
        "message": message.into(),
    })
    .to_string()
}

#[async_trait]
impl PlatformToolBridge for DiscussionBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if tool_name == DISCUSSION_PROPOSE_TOOL {
            return self.propose(&arguments);
        }
        self.inner.call(tool_name, arguments).await
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        if tool_name == DISCUSSION_PROPOSE_TOOL {
            return Clearance::Clear;
        }
        self.inner.clearance(tool_name, arguments).await
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        tool_name == DISCUSSION_PROPOSE_TOOL || self.inner.supports_tool(tool_name)
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

    async fn bot_secret(&self, name: &str) -> Option<String> {
        self.inner.bot_secret(name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::bot_turn::{MAX_DISCUSSION_HOPS, MAX_DISCUSSION_ROUNDS, discussion_hops};

    struct Inner;

    #[async_trait]
    impl PlatformToolBridge for Inner {
        async fn call(&self, tool_name: &str, _arguments: Value) -> String {
            format!("inner:{tool_name}")
        }
    }

    fn bridge(hops: u32) -> (DiscussionProposals, DiscussionBridge) {
        let proposals = DiscussionProposals::new();
        let bridge = DiscussionBridge::new(proposals.clone(), hops, Arc::new(Inner));
        (proposals, bridge)
    }

    fn status(result: &str) -> String {
        serde_json::from_str::<Value>(result).unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn a_proposal_asks_for_more_rounds_than_the_discussion_has_and_lands_on_the_reply() {
        let (proposals, bridge) = bridge(MAX_DISCUSSION_HOPS);
        assert!(bridge.supports_tool(DISCUSSION_PROPOSE_TOOL));
        for (arguments, why) in [
            (json!({"reason": "x"}), "rounds missing"),
            (
                json!({"rounds": MAX_DISCUSSION_ROUNDS, "reason": "x"}),
                "not more",
            ),
            (
                json!({"rounds": MAX_LONG_DISCUSSION_ROUNDS + 1, "reason": "x"}),
                "too many",
            ),
            (json!({"rounds": 12, "reason": "  "}), "no reason"),
        ] {
            let result = bridge.call(DISCUSSION_PROPOSE_TOOL, arguments).await;
            assert_eq!(status(&result), "error", "{why}");
        }
        assert!(proposals.proposal().is_none());

        let result = bridge
            .call(
                DISCUSSION_PROPOSE_TOOL,
                json!({"rounds": 12, "reason": "Pricing has\nthree markets to compare."}),
            )
            .await;
        assert_eq!(status(&result), "ok");
        let mut body = json!({"text": "This needs longer."});
        proposals.attach(&mut body);
        assert_eq!(
            body["discussion_proposal"],
            json!({"rounds": 12, "reason": "Pricing has three markets to compare."})
        );
        assert_eq!(
            bridge.call("web_search", json!({})).await,
            "inner:web_search"
        );
        assert_eq!(
            bridge.clearance(DISCUSSION_PROPOSE_TOOL, &json!({})).await,
            Clearance::Clear
        );
    }

    #[tokio::test]
    async fn a_granted_discussion_counts_its_own_rounds() {
        let (proposals, bridge) = bridge(discussion_hops(12));
        let refused = bridge
            .call(
                DISCUSSION_PROPOSE_TOOL,
                json!({"rounds": 12, "reason": "x"}),
            )
            .await;
        assert!(refused.contains("already allows 12 rounds"));
        let mut body = json!({"text": "no proposal"});
        proposals.attach(&mut body);
        assert!(body.get("discussion_proposal").is_none());
    }
}
