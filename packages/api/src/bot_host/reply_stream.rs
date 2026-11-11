//! A live-call reply as its model writes it, published to the devices of the conversation's
//! people over the realtime connection they already hold. It only shortens the wait: the stored
//! reply, its wakeup and the replay are unchanged, and every exit that posts no reply ends the
//! stream as dropped. So does a draft the room replaced after devices heard part of it.

use crate::{
    entity::bot_turn,
    routes::inbox::{active_members, members::Principal},
    state::AppState,
};
use flow_like::flow::copilot::{
    bot_run::SILENT_REPLY,
    stream::{DRAFT_REPLACED_TAG, frame_tag},
    tool_gate::ToolGate,
};
use flow_like_types::{
    bot_turn::reply_message_id,
    inbox::PrincipalKind,
    realtime::{REALTIME_KIND_REPLY, REALTIME_PROTOCOL_VERSION, RealtimeReply, RealtimeReplyState},
    tokio::{
        self,
        sync::Notify,
        task::JoinHandle,
        time::{sleep, timeout},
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Messages of one reply go out at most this often; the newest text wins.
const MIN_INTERVAL: Duration = Duration::from_millis(100);
/// Nothing changes while a tool runs; a repeat tells devices the reply is still coming.
const REPEAT_EVERY: Duration = Duration::from_secs(5);
/// How long a finished turn waits for its last message before it moves on.
const FINISH_WAIT: Duration = Duration::from_millis(2_500);
/// A reply is cut to one envelope when it is posted; text far beyond that is not collected.
const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct Draft {
    text: String,
    /// The bot stopped writing: a tool started, or the model finished and the turn settles.
    working: bool,
    /// Set once the turn knows whether the reply is posted.
    posted: Option<bool>,
}

impl Draft {
    /// Takes one chunk of the turn's stream; `true` when it changed what devices are told.
    fn take(&mut self, chunk: &str) -> bool {
        match frame_tag(chunk) {
            Some("tool_start") => !std::mem::replace(&mut self.working, true),
            // The text devices hold is not the reply any more; they speak the stored one.
            Some(DRAFT_REPLACED_TAG) => self.settle(false),
            Some(_) => false,
            None => self.write(chunk),
        }
    }

    /// Decides whether the reply is posted; `true` when nothing had decided it before.
    fn settle(&mut self, posted: bool) -> bool {
        let undecided = self.posted.is_none();
        self.posted.get_or_insert(posted);
        undecided
    }

    fn write(&mut self, text: &str) -> bool {
        if text.is_empty() || self.text.len() >= MAX_TEXT_BYTES {
            return false;
        }
        // What follows a tool is a new thought even when the model sends no blank line.
        if std::mem::take(&mut self.working)
            && !self.text.is_empty()
            && !self.text.ends_with(char::is_whitespace)
            && !text.starts_with(char::is_whitespace)
        {
            self.text.push_str("\n\n");
        }
        self.text.push_str(text);
        true
    }

    fn state(&self) -> RealtimeReplyState {
        match self.posted {
            Some(true) => RealtimeReplyState::Done,
            Some(false) => RealtimeReplyState::Dropped,
            None if self.working => RealtimeReplyState::Working,
            None => RealtimeReplyState::Writing,
        }
    }
}

/// Whether `text` is empty so far or could still become the marker a turn answers to stay
/// silent ([`flow_like::flow::copilot::bot_run::is_silent_reply`]); such text is not published.
fn may_be_silent(text: &str) -> bool {
    let core = text
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    SILENT_REPLY
        .get(..core.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(core))
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Wait,
    Publish,
    Stop,
}

/// What the publisher does with the draft as it stands. `sent` is the text length and state of
/// the last message that went out; `repeat` is the tick that repeats an unchanged state.
fn step(
    sent: Option<(usize, RealtimeReplyState)>,
    text: &str,
    state: RealtimeReplyState,
    repeat: bool,
) -> Step {
    match (sent, state) {
        (None, RealtimeReplyState::Dropped) => Step::Stop,
        (None, RealtimeReplyState::Done) => Step::Publish,
        (None, _) if may_be_silent(text) => Step::Wait,
        (Some(last), _) if !repeat && last == (text.len(), state) => Step::Wait,
        _ => Step::Publish,
    }
}

#[derive(Default)]
struct Shared {
    draft: parking_lot::Mutex<Draft>,
    changed: Notify,
}

impl Shared {
    fn snapshot(&self) -> (String, RealtimeReplyState) {
        let draft = self.draft.lock();
        (draft.text.clone(), draft.state())
    }

    /// Waits for the draft to change, or until the last message is old enough to repeat, and
    /// then for the gap between two messages unless the turn is over. `true` when a repeat is due.
    async fn next_change(&self, published_at: Option<Instant>) -> bool {
        let repeat_in = published_at.map(|at| REPEAT_EVERY.saturating_sub(at.elapsed()));
        tokio::select! {
            () = self.changed.notified() => {}
            () = sleep(repeat_in.unwrap_or_default()), if repeat_in.is_some() => {}
        }
        let ending = self.draft.lock().posted.is_some();
        if let (false, Some(at)) = (ending, published_at) {
            sleep(MIN_INTERVAL.saturating_sub(at.elapsed())).await;
        }
        published_at.is_some_and(|at| at.elapsed() >= REPEAT_EVERY)
    }
}

/// Takes what the turn's copilot streams: reply text and protocol frames.
#[derive(Clone)]
pub(crate) struct ReplySink(Arc<Shared>);

impl ReplySink {
    pub(crate) fn push(&self, chunk: &str) {
        if self.0.draft.lock().take(chunk) {
            self.0.changed.notify_one();
        }
    }
}

struct Route {
    turn_id: String,
    conversation_id: String,
    message_id: String,
    reply_to: String,
    sender_id: String,
}

impl Route {
    fn of(turn: &bot_turn::Model) -> Self {
        Self {
            turn_id: turn.id.clone(),
            conversation_id: turn.conversation_id.clone(),
            message_id: reply_message_id(&turn.bot_id, &turn.trigger_message_id),
            reply_to: turn.trigger_message_id.clone(),
            sender_id: turn.bot_id.clone(),
        }
    }

    fn message(&self, text: &str, state: RealtimeReplyState) -> Option<Vec<u8>> {
        RealtimeReply {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            kind: REALTIME_KIND_REPLY.into(),
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            reply_to: self.reply_to.clone(),
            sender_id: self.sender_id.clone(),
            offset: 0,
            text: String::new(),
            state,
        }
        .encode_fitting(text)
    }
}

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
    route: Route,
    recipients: Option<Vec<String>>,
    messages: u32,
}

impl Outbox {
    /// `false` when there is nobody to tell or the message cannot be built: the stream ends.
    async fn send(&mut self, text: &str, reply_state: RealtimeReplyState) -> bool {
        if self.recipients.is_none() {
            self.recipients = Some(people(&self.state, &self.route.conversation_id).await);
        }
        let to = self.recipients.as_deref().unwrap_or_default();
        let Some(payload) = self
            .route
            .message(text, reply_state)
            .filter(|_| !to.is_empty())
        else {
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
            turn_id = %self.route.turn_id,
            first_text_ms,
            messages = self.messages,
            people = self.recipients.as_ref().map_or(0, Vec::len),
            total_ms = opened.elapsed().as_millis() as u64,
            "voice call reply stream"
        );
    }
}

/// Whether the turn's participation decision, when it waits for one, lets it answer.
async fn answers(gate: Option<ToolGate>) -> bool {
    match gate {
        Some(gate) => gate.passes().await,
        None => true,
    }
}

async fn publish(mut outbox: Outbox, gate: Option<ToolGate>, shared: Arc<Shared>, opened: Instant) {
    if !answers(gate).await {
        return;
    }
    let mut sent = None;
    let mut published_at = None;
    let mut first_text_ms = None;
    loop {
        let repeat = shared.next_change(published_at).await;
        let (text, reply_state) = shared.snapshot();
        match step(sent, &text, reply_state, repeat) {
            Step::Wait => continue,
            Step::Stop => break,
            Step::Publish => {}
        }
        if !outbox.send(&text, reply_state).await {
            break;
        }
        first_text_ms.get_or_insert(opened.elapsed().as_millis() as u64);
        sent = Some((text.len(), reply_state));
        published_at = Some(Instant::now());
        if reply_state.is_final() {
            break;
        }
    }
    outbox.report(first_text_ms, opened);
}

/// One turn's reply stream. Ends as dropped unless [`ReplyStream::finish`] says it was posted
/// and no draft was replaced on the way.
pub(crate) struct ReplyStream {
    shared: Arc<Shared>,
    task: Option<JoinHandle<()>>,
}

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
        let shared = Arc::new(Shared::default());
        let outbox = Outbox {
            state: state.clone(),
            route: Route::of(turn),
            recipients: None,
            messages: 0,
        };
        let task = tokio::spawn(publish(outbox, gate, shared.clone(), Instant::now()));
        Some(Self {
            shared,
            task: Some(task),
        })
    }

    pub(crate) fn sink(&self) -> ReplySink {
        ReplySink(self.shared.clone())
    }

    /// The model stopped writing: devices speak what they hold while the turn settles.
    pub(crate) fn pause(&self) {
        self.shared.draft.lock().working = true;
        self.shared.changed.notify_one();
    }

    /// Sends the last message and waits for it, within [`FINISH_WAIT`].
    pub(crate) async fn finish(mut self, posted: bool) {
        self.end(posted);
        if let Some(task) = self.task.take() {
            let _ = timeout(FINISH_WAIT, task).await;
        }
    }

    fn end(&self, posted: bool) {
        self.shared.draft.lock().settle(posted);
        self.shared.changed.notify_one();
    }
}

impl Drop for ReplyStream {
    fn drop(&mut self) {
        self.end(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::copilot::{
        PlanStepStatus,
        stream::{draft_replaced_frame, plan_step_frame, tool_end_frame, tool_start_frame},
    };

    #[test]
    fn a_draft_keeps_the_text_and_notes_when_a_tool_runs() {
        let mut draft = Draft::default();
        assert!(draft.take("Let me "));
        let thinking = plan_step_frame(
            "step-1".into(),
            "Thinking".into(),
            PlanStepStatus::InProgress,
            "think",
        );
        assert!(!draft.take(&thinking) && !draft.take(""));
        assert!(draft.take("check."));
        assert_eq!(
            (draft.text.as_str(), draft.state()),
            ("Let me check.", RealtimeReplyState::Writing)
        );

        assert!(draft.take(&tool_start_frame("call-1", "web_search", None)));
        assert!(!draft.take(&tool_start_frame("call-2", "web_fetch", None)));
        assert!(!draft.take(&tool_end_frame("call-1", "web_search", "done")));
        assert_eq!(draft.state(), RealtimeReplyState::Working);

        assert!(draft.take("It is five."));
        assert_eq!(
            (draft.text.as_str(), draft.state()),
            ("Let me check.\n\nIt is five.", RealtimeReplyState::Writing)
        );
    }

    #[test]
    fn text_after_a_tool_gets_a_break_only_when_it_has_none() {
        for (before, after, joined) in [
            ("One.", "Two.", "One.\n\nTwo."),
            ("One.\n", "Two.", "One.\nTwo."),
            ("One.", " Two.", "One. Two."),
            ("", "Two.", "Two."),
        ] {
            let mut draft = Draft::default();
            draft.take(before);
            draft.take(&tool_start_frame("call-1", "web_search", None));
            draft.take(after);
            assert_eq!(draft.text, joined);
        }
    }

    #[test]
    fn the_end_of_a_turn_decides_the_state_once() {
        let mut draft = Draft::default();
        draft.take("Done.");
        draft.posted.get_or_insert(true);
        draft.posted.get_or_insert(false);
        draft.working = true;
        assert_eq!(draft.state(), RealtimeReplyState::Done);
        assert_eq!(
            Draft {
                posted: Some(false),
                ..Draft::default()
            }
            .state(),
            RealtimeReplyState::Dropped
        );
    }

    #[test]
    fn a_replaced_draft_drops_the_stream_for_good() {
        let mut draft = Draft::default();
        draft.take("Sure, the ");
        assert!(draft.take(&draft_replaced_frame()));
        assert!(!draft.take(&draft_replaced_frame()));
        draft.take("It is five.");
        assert!(!draft.settle(true));
        assert_eq!(draft.state(), RealtimeReplyState::Dropped);
    }

    #[test]
    fn text_that_may_become_the_silent_marker_is_held() {
        for held in [
            "",
            " ",
            "N",
            "no",
            "NO_",
            "no_rep",
            "NO_REPLY",
            "**NO_REPLY**",
            "NO_REPLY.",
            "\"No",
        ] {
            assert!(may_be_silent(held), "{held:?}");
        }
        for released in [
            "No, I",
            "Not",
            "NO_REPLY and",
            "Nö",
            "Sure",
            "N.O",
            "NO_REPLYX",
        ] {
            assert!(!may_be_silent(released), "{released:?}");
        }
    }

    #[test]
    fn nothing_goes_out_before_there_is_text_worth_sending() {
        use RealtimeReplyState::{Done, Dropped, Working, Writing};
        assert_eq!(step(None, "", Writing, false), Step::Wait);
        assert_eq!(step(None, "", Working, false), Step::Wait);
        assert_eq!(step(None, "NO_REP", Writing, false), Step::Wait);
        assert_eq!(step(None, "No, wait", Writing, false), Step::Publish);
        assert_eq!(step(None, "NO_REPLY", Dropped, false), Step::Stop);
        assert_eq!(step(None, "No.", Done, false), Step::Publish);
    }

    #[test]
    fn a_message_goes_out_for_new_text_a_new_state_or_a_repeat() {
        use RealtimeReplyState::{Done, Dropped, Working, Writing};
        let sent = Some((5, Writing));
        assert_eq!(step(sent, "Hello", Writing, false), Step::Wait);
        assert_eq!(step(sent, "Hello", Writing, true), Step::Publish);
        assert_eq!(step(sent, "Hello there", Writing, false), Step::Publish);
        assert_eq!(step(sent, "Hello", Working, false), Step::Publish);
        assert_eq!(step(sent, "Hello", Done, false), Step::Publish);
        assert_eq!(step(sent, "Hello", Dropped, false), Step::Publish);
    }

    #[test]
    fn a_message_names_the_reply_the_turn_will_post() {
        let route = Route {
            turn_id: "bt_1".into(),
            conversation_id: "c-1".into(),
            message_id: reply_message_id("bot-1", "m-1"),
            reply_to: "m-1".into(),
            sender_id: "bot-1".into(),
        };
        let encoded = route.message("Hello", RealtimeReplyState::Writing).unwrap();
        let reply: RealtimeReply = serde_json::from_slice(&encoded).unwrap();
        assert!(reply.is_valid());
        assert_eq!(reply.message_id, reply_message_id("bot-1", "m-1"));
        assert_eq!(
            (reply.reply_to.as_str(), reply.sender_id.as_str()),
            ("m-1", "bot-1")
        );
        assert_eq!((reply.offset, reply.text.as_str()), (0, "Hello"));
    }
}
