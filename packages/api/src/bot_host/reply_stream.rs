//! A live-call reply as its model writes it, published to the devices of the conversation's
//! people over the realtime connection they already hold. What goes out and when is decided in
//! [`flow_like::flow::copilot::reply_stream`], shared with the hosts on the caller's computer;
//! this module finds the people and publishes.

use crate::{
    entity::bot_turn,
    routes::inbox::{active_members, members::Principal},
    state::AppState,
};
use async_trait::async_trait;
use flow_like::flow::copilot::{
    reply_stream::{self, Message, ReplyOutbox, ReplyRoute, ReplySink},
    tool_gate::ToolGate,
};
use flow_like_types::inbox::PrincipalKind;
use std::time::Instant;

/// The people who receive the stored reply: the conversation's active user members.
async fn people(state: &AppState, conversation_id: &str) -> Vec<String> {
    match active_members(&state.db, &[conversation_id.to_owned()]).await {
        Ok(members) => members
            .iter()
            .map(Principal::from)
            .filter(|principal| principal.kind == PrincipalKind::User)
            .map(|principal| principal.id)
            .collect(),
        Err(error) => {
            tracing::debug!(%error, conversation_id, "Reply stream has no members to tell; the stored reply still arrives");
            Vec::new()
        }
    }
}

/// Sends one reply's messages to the people of its conversation, found on the first send.
struct Outbox {
    state: AppState,
    turn_id: String,
    route: ReplyRoute,
    recipients: Option<Vec<String>>,
    messages: u32,
}

#[async_trait]
impl ReplyOutbox for Outbox {
    async fn send(&mut self, message: &Message) -> bool {
        if self.recipients.is_none() {
            self.recipients = Some(people(&self.state, self.route.conversation_id()).await);
        }
        let to = self.recipients.as_deref().unwrap_or_default();
        let Some(payload) = self.route.encode(message).filter(|_| !to.is_empty()) else {
            return false;
        };
        futures::future::join_all(to.iter().map(|sub| {
            self.state
                .channels
                .publish_realtime_reply(sub, payload.clone())
        }))
        .await;
        self.messages += 1;
        true
    }

    fn report(&self, first_text_ms: Option<u64>, opened: Instant) {
        if self.messages == 0 {
            return;
        }
        tracing::info!(
            target: "bot_call_latency",
            turn_id = %self.turn_id,
            first_text_ms,
            messages = self.messages,
            people = self.recipients.as_ref().map_or(0, Vec::len),
            total_ms = opened.elapsed().as_millis() as u64,
            "voice call reply stream"
        );
    }
}

/// One turn's reply stream. Ends as dropped unless [`ReplyStream::finish`] says it was posted.
pub(crate) struct ReplyStream(reply_stream::ReplyStream);

impl ReplyStream {
    /// `None` on a deployment whose realtime transport carries no replies. With a `gate`,
    /// nothing is published before the turn's participation decision lets it answer.
    pub(crate) fn open(
        state: &AppState,
        turn: &bot_turn::Model,
        gate: Option<ToolGate>,
    ) -> Option<Self> {
        if !state.channels.carries_realtime_replies() {
            return None;
        }
        let outbox = Outbox {
            state: state.clone(),
            turn_id: turn.id.clone(),
            route: ReplyRoute::new(
                &turn.conversation_id,
                &turn.bot_id,
                &turn.trigger_message_id,
            ),
            recipients: None,
            messages: 0,
        };
        Some(Self(reply_stream::ReplyStream::start(outbox, gate)))
    }

    pub(crate) fn sink(&self) -> ReplySink {
        self.0.sink()
    }

    /// The model stopped writing: devices speak what they hold while the turn settles.
    pub(crate) fn pause(&self) {
        self.0.pause();
    }

    /// Sends the last message and waits for it, within the shared stream's finish wait.
    pub(crate) async fn finish(self, posted: bool) {
        self.0.finish(posted).await;
    }
}
