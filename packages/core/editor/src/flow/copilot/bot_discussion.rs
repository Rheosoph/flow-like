//! Discussions between bots in a room. When a topic needs more rounds than its discussion allows,
//! a bot proposes them with [`DISCUSSION_PROPOSE_TOOL`]: its reply carries the proposal, which the
//! room shows with a button, and a person who allows it sends the grant on their next message
//! ([`flow_like_types::bot_message::DiscussionGrant`]). When the bots agree, or have nothing left
//! to add, one of them ends the discussion with [`DISCUSSION_END_TOOL`]: its reply carries the
//! outcome, which the room shows as the discussion's result. Either reply hands the word to no
//! one ([`flow_like_types::bot_message::MessageBody::rests_discussion`]).

use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use flow_like_types::{
    bot_message::{DiscussionOutcome, DiscussionProposal, MAX_DISCUSSION_SUMMARY_CHARS},
    bot_turn::{MAX_LONG_DISCUSSION_ROUNDS, discussion_rounds},
};
use serde_json::{Value, json};

use super::platform::{Clearance, PlatformToolBridge};
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};

pub const DISCUSSION_PROPOSE_TOOL: &str = "discussion_propose";
pub const DISCUSSION_END_TOOL: &str = "discussion_end";
const MAX_REASON_CHARS: usize = 280;

#[derive(Default)]
struct Marks {
    proposal: Option<DiscussionProposal>,
    outcome: Option<DiscussionOutcome>,
}

/// What one room turn's reply says about its discussion: a longer discussion it proposes, or the
/// outcome it ends the discussion with. The host puts it on the reply.
#[derive(Clone, Default)]
pub struct DiscussionReply(Arc<Mutex<Marks>>);

impl DiscussionReply {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn proposal(&self) -> Option<DiscussionProposal> {
        self.lock().proposal.clone()
    }

    pub fn outcome(&self) -> Option<DiscussionOutcome> {
        self.lock().outcome.clone()
    }

    /// Puts the turn's proposal and outcome on its reply `body` as `discussion_proposal` and
    /// `discussion_outcome`.
    pub fn attach(&self, body: &mut Value) {
        let Some(object) = body.as_object_mut() else {
            return;
        };
        let marks = self.lock();
        if let Some(proposal) = &marks.proposal {
            object.insert("discussion_proposal".to_owned(), json!(proposal));
        }
        if let Some(outcome) = &marks.outcome {
            object.insert("discussion_outcome".to_owned(), json!(outcome));
        }
    }

    fn lock(&self) -> MutexGuard<'_, Marks> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for DiscussionReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let marks = self.lock();
        f.debug_struct("DiscussionReply")
            .field(
                "proposal",
                &marks.proposal.as_ref().map(|proposal| proposal.rounds),
            )
            .field(
                "outcome",
                &marks.outcome.as_ref().map(|outcome| outcome.agreed),
            )
            .finish()
    }
}

impl PartialEq for DiscussionReply {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for DiscussionReply {}

pub fn discussion_propose_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: DISCUSSION_PROPOSE_TOOL,
        description: "Ask the people in this room to allow a longer discussion with the other bots. Use it when the topic clearly needs more rounds than the discussion allows, or when a person asked for more rounds than it allows; a round is one message from each of two bots. Your reply then shows the proposal with a button: say in your text why the topic needs it. The discussion waits until a person allows it, then runs on with the new budget.",
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

pub fn discussion_end_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: DISCUSSION_END_TOOL,
        description: "End your discussion with the other bot in this room: when you two agree, or when neither of you has anything left to add. The room shows the outcome as the discussion's result and the other bot doesn't answer your reply. Still write the outcome for the room in your text, in a few sentences.",
        schema: discussion_end_schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: 10,
    }
}

fn discussion_end_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["agreed", "summary"],
        "properties": {
            "agreed": {
                "type": "boolean",
                "description": "Whether you two agree on the outcome."
            },
            "summary": {
                "type": "string",
                "maxLength": MAX_DISCUSSION_SUMMARY_CHARS,
                "description": "One sentence for the people in the room: what you agreed on, or what stays open."
            }
        }
    })
}

/// Answers [`DISCUSSION_PROPOSE_TOOL`] and [`DISCUSSION_END_TOOL`] in core, keeping what they say
/// in `reply`; every other call goes to `inner`.
pub struct DiscussionBridge {
    reply: DiscussionReply,
    /// Rounds the turn's discussion has now.
    rounds: u32,
    /// Another bot's message woke the turn, so it is in a discussion it may end.
    can_end: bool,
    inner: Arc<dyn PlatformToolBridge>,
}

impl DiscussionBridge {
    /// `discussion_hops` is the bot turns the turn's discussion may reach; `can_end` says another
    /// bot's message woke the turn.
    pub fn new(
        reply: DiscussionReply,
        discussion_hops: u32,
        can_end: bool,
        inner: Arc<dyn PlatformToolBridge>,
    ) -> Self {
        Self {
            reply,
            rounds: discussion_rounds(discussion_hops),
            can_end,
            inner,
        }
    }

    fn answers(&self, tool_name: &str) -> bool {
        tool_name == DISCUSSION_PROPOSE_TOOL || (self.can_end && tool_name == DISCUSSION_END_TOOL)
    }

    fn propose(&self, arguments: &Value) -> String {
        let refuse = |message: String| refusal(DISCUSSION_PROPOSE_TOOL, message);
        let rounds = arguments
            .get("rounds")
            .and_then(Value::as_u64)
            .and_then(|rounds| u32::try_from(rounds).ok());
        let Some(rounds) = rounds else {
            return refuse(
                "Pass `rounds`, the rounds the whole discussion needs, as a number".into(),
            );
        };
        if rounds > MAX_LONG_DISCUSSION_ROUNDS {
            return refuse(format!(
                "A discussion has at most {MAX_LONG_DISCUSSION_ROUNDS} rounds; propose {MAX_LONG_DISCUSSION_ROUNDS} or fewer"
            ));
        }
        if rounds <= self.rounds {
            return refuse(format!(
                "This discussion already allows {} rounds; propose more, or go on without asking",
                self.rounds
            ));
        }
        let reason = one_line(arguments.get("reason"), MAX_REASON_CHARS);
        if reason.is_empty() {
            return refuse("Pass `reason`: one sentence on why the topic needs more rounds".into());
        }
        let mut marks = self.reply.lock();
        if marks.outcome.is_some() {
            return refuse(format!(
                "You already ended the discussion with `{DISCUSSION_END_TOOL}` in this reply"
            ));
        }
        marks.proposal = Some(DiscussionProposal { rounds, reason });
        json!({
            "status": "ok",
            "rounds": rounds,
            "note": "Your reply shows the proposal with a button to allow it. Say in your text why the topic needs it: the discussion waits until a person in the room allows it."
        })
        .to_string()
    }

    fn end(&self, arguments: &Value) -> String {
        let refuse = |message: String| refusal(DISCUSSION_END_TOOL, message);
        let Some(agreed) = arguments.get("agreed").and_then(Value::as_bool) else {
            return refuse("Pass `agreed`: true when you two agree, else false".into());
        };
        let summary = one_line(arguments.get("summary"), MAX_DISCUSSION_SUMMARY_CHARS);
        if summary.is_empty() {
            return refuse(
                "Pass `summary`: one sentence on what you agreed on, or what stays open".into(),
            );
        }
        let mut marks = self.reply.lock();
        if marks.proposal.is_some() {
            return refuse(format!(
                "You proposed a longer discussion with `{DISCUSSION_PROPOSE_TOOL}` in this reply; end it in a later message"
            ));
        }
        marks.outcome = Some(DiscussionOutcome { agreed, summary });
        json!({
            "status": "ok",
            "agreed": agreed,
            "note": "Your reply ends the discussion and shows the outcome to the room. Write the outcome in your text in a few sentences, and don't address the other bot."
        })
        .to_string()
    }
}

/// `value` as one line of at most `max` characters, empty when it is no text.
fn one_line(value: Option<&Value>, max: usize) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn refusal(tool: &str, message: String) -> String {
    json!({
        "status": "error",
        "tool": tool,
        "retryable": true,
        "message": message,
    })
    .to_string()
}

#[async_trait]
impl PlatformToolBridge for DiscussionBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        match tool_name {
            DISCUSSION_PROPOSE_TOOL => self.propose(&arguments),
            DISCUSSION_END_TOOL if self.can_end => self.end(&arguments),
            _ => self.inner.call(tool_name, arguments).await,
        }
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        if self.answers(tool_name) {
            return Clearance::Clear;
        }
        self.inner.clearance(tool_name, arguments).await
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        self.answers(tool_name) || self.inner.supports_tool(tool_name)
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

    fn bridge(hops: u32, can_end: bool) -> (DiscussionReply, DiscussionBridge) {
        let reply = DiscussionReply::new();
        let bridge = DiscussionBridge::new(reply.clone(), hops, can_end, Arc::new(Inner));
        (reply, bridge)
    }

    fn status(result: &str) -> String {
        serde_json::from_str::<Value>(result).unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn a_proposal_asks_for_more_rounds_than_the_discussion_has_and_lands_on_the_reply() {
        let (reply, bridge) = bridge(MAX_DISCUSSION_HOPS, false);
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
        assert!(reply.proposal().is_none());

        let result = bridge
            .call(
                DISCUSSION_PROPOSE_TOOL,
                json!({"rounds": 12, "reason": "Pricing has\nthree markets to compare."}),
            )
            .await;
        assert_eq!(status(&result), "ok");
        let mut body = json!({"text": "This needs longer."});
        reply.attach(&mut body);
        assert_eq!(
            body["discussion_proposal"],
            json!({"rounds": 12, "reason": "Pricing has three markets to compare."})
        );
        assert!(body.get("discussion_outcome").is_none());
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
        let (reply, bridge) = bridge(discussion_hops(12), false);
        let refused = bridge
            .call(
                DISCUSSION_PROPOSE_TOOL,
                json!({"rounds": 12, "reason": "x"}),
            )
            .await;
        assert!(refused.contains("already allows 12 rounds"));
        let mut body = json!({"text": "no proposal"});
        reply.attach(&mut body);
        assert!(body.get("discussion_proposal").is_none());
    }

    #[tokio::test]
    async fn only_a_turn_in_a_discussion_ends_it_and_the_outcome_lands_on_the_reply() {
        let (_, opener) = bridge(MAX_DISCUSSION_HOPS, false);
        assert_eq!(
            opener
                .call(DISCUSSION_END_TOOL, json!({"agreed": true, "summary": "x"}))
                .await,
            "inner:discussion_end"
        );

        let (reply, bridge) = bridge(MAX_DISCUSSION_HOPS, true);
        assert!(bridge.supports_tool(DISCUSSION_END_TOOL));
        for (arguments, why) in [
            (json!({"summary": "x"}), "agreed missing"),
            (json!({"agreed": true, "summary": " \n "}), "no summary"),
        ] {
            let result = bridge.call(DISCUSSION_END_TOOL, arguments).await;
            assert_eq!(status(&result), "error", "{why}");
        }
        let result = bridge
            .call(
                DISCUSSION_END_TOOL,
                json!({"agreed": true, "summary": format!("Supplier B,\nfrom March. {}", "x".repeat(400))}),
            )
            .await;
        assert_eq!(status(&result), "ok");
        let outcome = reply.outcome().unwrap();
        assert!(outcome.agreed);
        assert!(outcome.summary.starts_with("Supplier B, from March."));
        assert_eq!(
            outcome.summary.chars().count(),
            MAX_DISCUSSION_SUMMARY_CHARS
        );
        let mut body = json!({"text": "We agree."});
        reply.attach(&mut body);
        assert_eq!(body["discussion_outcome"]["agreed"], json!(true));

        let refused = bridge
            .call(
                DISCUSSION_PROPOSE_TOOL,
                json!({"rounds": 12, "reason": "More"}),
            )
            .await;
        assert_eq!(
            status(&refused),
            "error",
            "an ended discussion proposes nothing"
        );
        assert!(reply.proposal().is_none());
    }
}
