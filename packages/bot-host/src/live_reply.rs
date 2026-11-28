//! A turn that a host on the caller's computer answers is heard (live call) or shown (text) while
//! it is written.
//! The host hands the reply, in the shape of the server's realtime reply, to an outlet its app
//! provides ([`crate::host::TurnEvents::replies`]); nothing of it leaves this computer. What goes
//! out and when is the server's stream, shared through
//! [`flow_like::flow::copilot::reply_stream`]. A host without an outlet streams nothing.

use crate::host::{RunOutcome, TurnEvents, TurnJob};
use async_trait::async_trait;
use flow_like::flow::copilot::reply_stream::{
    Message, ReplyOutbox, ReplyRoute, ReplyStream, interval_for,
};
use flow_like_types::realtime::RealtimeReply;
use std::{sync::Arc, time::Instant};

/// Hands a reply to the windows of this computer while it is written.
pub trait ReplyOutlet: Send + Sync + 'static {
    fn reply(&self, reply: RealtimeReply);
}

struct Outlet {
    outlet: Arc<dyn ReplyOutlet>,
    route: ReplyRoute,
    turn_id: String,
    messages: u32,
    live_call: bool,
}

#[async_trait]
impl ReplyOutbox for Outlet {
    async fn send(&mut self, message: &Message) -> bool {
        let Some(reply) = self.route.reply(message) else {
            return false;
        };
        self.outlet.reply(reply);
        self.messages += 1;
        true
    }

    fn report(&self, first_text_ms: Option<u64>, opened: Instant) {
        if self.messages == 0 {
            return;
        }
        let total_ms = opened.elapsed().as_millis() as u64;
        if self.live_call {
            tracing::info!(
                target: "bot_call_latency",
                turn_id = %self.turn_id,
                first_text_ms,
                messages = self.messages,
                total_ms,
                "live reply stream on this computer"
            );
        } else {
            tracing::info!(
                turn_id = %self.turn_id,
                first_text_ms,
                messages = self.messages,
                total_ms,
                "live reply stream on this computer"
            );
        }
    }
}

/// Opens the stream of a turn that answers in its own conversation when the host
/// has an outlet, and hands the job its sink; any other job gets none. Call it right before the
/// run starts, after `job.tool_gate` is set: nothing goes out before that gate opens.
pub fn open(
    events: &dyn TurnEvents,
    job: &mut TurnJob,
    trigger_message_id: &str,
) -> Option<ReplyStream> {
    job.reply_stream = None;
    if job.answered_round.is_some() {
        return None;
    }
    let outlet = Outlet {
        outlet: events.replies()?,
        route: ReplyRoute::new(&job.conversation_id, &job.bot_id, trigger_message_id),
        turn_id: job.run_id.clone(),
        messages: 0,
        live_call: job.live_call,
    };
    let stream = ReplyStream::start(outlet, job.tool_gate.clone(), interval_for(job.live_call));
    job.reply_stream = Some(stream.sink());
    Some(stream)
}

/// The model finished: devices speak what they hold while the reply is stored.
pub fn pause(stream: Option<&ReplyStream>, outcome: &RunOutcome) {
    if let (Some(stream), Ok(Ok(_))) = (stream, outcome) {
        stream.pause();
    }
}

/// Ends the stream once the turn knows whether its reply was stored; a stream dropped without
/// this ends as dropped.
pub async fn finish(stream: Option<ReplyStream>, posted: bool) {
    if let Some(stream) = stream {
        stream.finish(posted).await;
    }
}
