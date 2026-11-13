//! A live-call reply as its model writes it, for the devices that will speak it. It only shortens
//! the wait: the stored reply stays authoritative, and every exit that posts no reply ends the
//! stream as dropped. A draft the room replaced after devices heard part of it is dropped there,
//! and the rewrite follows as the reply's next draft. Where the messages go is the host's
//! [`ReplyOutbox`]: the server publishes them over the realtime connection, a host on the
//! caller's computer hands them to its app.

use super::{
    bot_run::SILENT_REPLY,
    stream::{DRAFT_REPLACED_TAG, frame_tag},
    tool_gate::ToolGate,
};
use async_trait::async_trait;
use flow_like_types::{
    bot_turn::reply_message_id,
    realtime::{REALTIME_KIND_REPLY, REALTIME_PROTOCOL_VERSION, RealtimeReply, RealtimeReplyState},
    tokio::{
        self,
        sync::Notify,
        task::JoinHandle,
        time::{sleep, timeout},
    },
};
use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
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
    /// How often the room replaced the reply; `text` belongs to the latest draft only.
    rewrites: u32,
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
            Some(DRAFT_REPLACED_TAG) => self.replace(),
            Some(_) => false,
            None => self.write(chunk),
        }
    }

    /// The text devices hold is not the reply any more: the rewrite starts over as a new draft.
    fn replace(&mut self) -> bool {
        if self.posted.is_some() {
            return false;
        }
        self.rewrites += 1;
        self.text.clear();
        self.working = false;
        true
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

    fn message(&self) -> Message {
        Message {
            draft: self.rewrites,
            text: self.text.clone(),
            state: self.state(),
        }
    }
}

/// One message of the stream, before it is addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    draft: u32,
    text: String,
    state: RealtimeReplyState,
}

impl Message {
    /// Ends a draft devices hold that the room has replaced since.
    fn replaced(draft: u32) -> Self {
        Self {
            draft,
            text: String::new(),
            state: RealtimeReplyState::Dropped,
        }
    }
}

/// Whether `text` is empty so far or could still become the marker a turn answers to stay
/// silent ([`super::bot_run::is_silent_reply`]); such text is not published.
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

/// Remembers what went out and decides what follows.
#[derive(Default)]
struct Publisher {
    /// The draft, text length and state of the last message that went out.
    sent: Option<(u32, usize, RealtimeReplyState)>,
}

impl Publisher {
    /// The messages due for the reply as it stands, in order, and whether the stream ends with
    /// them. A draft devices hold that the room replaced since is dropped first; its rewrite then
    /// starts like a new reply, held back while it is empty or may become the silent marker.
    fn due(&mut self, now: &Message, repeat: bool) -> (Vec<Message>, bool) {
        let mut due = Vec::new();
        if let Some((draft, ..)) = self.sent.filter(|&(draft, ..)| draft < now.draft) {
            due.push(Message::replaced(draft));
            self.sent = None;
        }
        let sent = self.sent.map(|(_, len, state)| (len, state));
        let over = match step(sent, &now.text, now.state, repeat) {
            Step::Wait => false,
            Step::Stop => true,
            Step::Publish => {
                self.sent = Some((now.draft, now.text.len(), now.state));
                due.push(now.clone());
                now.state.is_final()
            }
        };
        (due, over)
    }

    /// Whether devices hold text of the current draft that a repeat keeps alive.
    fn holds_draft(&self) -> bool {
        self.sent.is_some()
    }
}

#[derive(Default)]
struct Shared {
    draft: Mutex<Draft>,
    changed: Notify,
}

impl Shared {
    fn draft(&self) -> MutexGuard<'_, Draft> {
        self.draft.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshot(&self) -> Message {
        self.draft().message()
    }

    /// Waits for the draft to change, or until the last message is old enough to repeat, and
    /// then for the gap between two messages unless the turn is over. `true` when a repeat is due.
    async fn next_change(&self, published_at: Option<Instant>) -> bool {
        let repeat_in = published_at.map(|at| REPEAT_EVERY.saturating_sub(at.elapsed()));
        tokio::select! {
            () = self.changed.notified() => {}
            () = sleep(repeat_in.unwrap_or_default()), if repeat_in.is_some() => {}
        }
        let ending = self.draft().posted.is_some();
        if let (false, Some(at)) = (ending, published_at) {
            sleep(MIN_INTERVAL.saturating_sub(at.elapsed())).await;
        }
        published_at.is_some_and(|at| at.elapsed() >= REPEAT_EVERY)
    }
}

/// Takes what the turn's copilot streams: reply text and protocol frames.
#[derive(Clone)]
pub struct ReplySink(Arc<Shared>);

impl ReplySink {
    pub fn push(&self, chunk: &str) {
        if self.0.draft().take(chunk) {
            self.0.changed.notify_one();
        }
    }
}

impl std::fmt::Debug for ReplySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplySink(..)")
    }
}

/// Where one reply's messages go. A host sends them in its own way; the stream decides when.
#[async_trait]
pub trait ReplyOutbox: Send + 'static {
    /// `false` when there is nobody to tell or the message cannot be built: the stream ends.
    async fn send(&mut self, message: &Message) -> bool;

    /// The stream ended; `first_text_ms` is when its first message went out.
    fn report(&self, _first_text_ms: Option<u64>, _opened: Instant) {}
}

/// Names the reply a turn will post in every message of its stream.
#[derive(Debug, Clone)]
pub struct ReplyRoute {
    conversation_id: String,
    message_id: String,
    reply_to: String,
    sender_id: String,
}

impl ReplyRoute {
    /// The reply `bot_id` posts in `conversation_id` to `trigger_message_id`, under the id the
    /// stored reply will have.
    pub fn new(conversation_id: &str, bot_id: &str, trigger_message_id: &str) -> Self {
        Self {
            conversation_id: conversation_id.to_owned(),
            message_id: reply_message_id(bot_id, trigger_message_id),
            reply_to: trigger_message_id.to_owned(),
            sender_id: bot_id.to_owned(),
        }
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    /// The message as one realtime packet: the whole text while it fits, else its tail. `None`
    /// when the names alone are too large.
    pub fn encode(&self, message: &Message) -> Option<Vec<u8>> {
        RealtimeReply {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            kind: REALTIME_KIND_REPLY.into(),
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            reply_to: self.reply_to.clone(),
            sender_id: self.sender_id.clone(),
            offset: 0,
            text: String::new(),
            state: message.state,
            draft: message.draft,
        }
        .encode_fitting(&message.text)
    }

    /// [`Self::encode`] as the reply a device receives.
    pub fn reply(&self, message: &Message) -> Option<RealtimeReply> {
        serde_json::from_slice(&self.encode(message)?).ok()
    }
}

/// Whether the turn's participation decision, when it waits for one, lets it answer.
async fn answers(gate: Option<ToolGate>) -> bool {
    match gate {
        Some(gate) => gate.passes().await,
        None => true,
    }
}

async fn publish(
    mut outbox: impl ReplyOutbox,
    gate: Option<ToolGate>,
    shared: Arc<Shared>,
    opened: Instant,
) {
    if !answers(gate).await {
        return;
    }
    let mut publisher = Publisher::default();
    let mut published_at = None;
    let mut first_text_ms = None;
    'stream: loop {
        let repeat = shared
            .next_change(published_at.filter(|_| publisher.holds_draft()))
            .await;
        let (due, over) = publisher.due(&shared.snapshot(), repeat);
        for message in &due {
            if !outbox.send(message).await {
                break 'stream;
            }
            first_text_ms.get_or_insert(opened.elapsed().as_millis() as u64);
            published_at = Some(Instant::now());
        }
        if over {
            break;
        }
    }
    outbox.report(first_text_ms, opened);
}

/// One turn's reply stream. Ends as dropped unless [`ReplyStream::finish`] says it was posted.
pub struct ReplyStream {
    shared: Arc<Shared>,
    task: Option<JoinHandle<()>>,
}

impl ReplyStream {
    /// Starts publishing to `outbox`. With a `gate`, nothing goes out before the turn's
    /// participation decision lets it answer.
    pub fn start(outbox: impl ReplyOutbox, gate: Option<ToolGate>) -> Self {
        let shared = Arc::new(Shared::default());
        let task = tokio::spawn(publish(outbox, gate, shared.clone(), Instant::now()));
        Self {
            shared,
            task: Some(task),
        }
    }

    pub fn sink(&self) -> ReplySink {
        ReplySink(self.shared.clone())
    }

    /// The model stopped writing: devices speak what they hold while the turn settles.
    pub fn pause(&self) {
        self.shared.draft().working = true;
        self.shared.changed.notify_one();
    }

    /// Sends the last message and waits for it, within [`FINISH_WAIT`].
    pub async fn finish(mut self, posted: bool) {
        self.end(posted);
        if let Some(task) = self.task.take() {
            let _ = timeout(FINISH_WAIT, task).await;
        }
    }

    fn end(&self, posted: bool) {
        self.shared.draft().settle(posted);
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
    use crate::flow::copilot::{
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

    /// A turn's chunks as the publisher sees them; `take` leaves the publisher asleep.
    #[derive(Default)]
    struct Run {
        draft: Draft,
        publisher: Publisher,
        sent: Vec<Message>,
        over: bool,
    }

    impl Run {
        fn take(&mut self, chunk: &str) -> &mut Self {
            self.draft.take(chunk);
            self
        }

        fn push(&mut self, chunk: &str) -> &mut Self {
            self.take(chunk).tick()
        }

        fn end(&mut self, posted: bool) -> &mut Self {
            self.draft.settle(posted);
            self.tick()
        }

        fn tick(&mut self) -> &mut Self {
            if !self.over {
                let (due, over) = self.publisher.due(&self.draft.message(), false);
                self.sent.extend(due);
                self.over = over;
            }
            self
        }

        fn sent(&self) -> Vec<(u32, &str, RealtimeReplyState)> {
            self.sent
                .iter()
                .map(|message| (message.draft, message.text.as_str(), message.state))
                .collect()
        }
    }

    #[test]
    fn a_replaced_draft_is_dropped_and_its_rewrite_streamed() {
        use RealtimeReplyState::{Done, Dropped, Writing};
        let replaced = draft_replaced_frame();
        let mut run = Run::default();
        run.push("Sure, the answer ")
            .push(&replaced)
            .push("Anna already ")
            .push("said it.")
            .end(true);
        assert_eq!(
            run.sent(),
            [
                (0, "Sure, the answer ", Writing),
                (0, "", Dropped),
                (1, "Anna already ", Writing),
                (1, "Anna already said it.", Writing),
                (1, "Anna already said it.", Done),
            ]
        );
        assert!(run.over);

        let mut late = Run::default();
        late.push("Sure, the answer ")
            .take(&replaced)
            .take("Anna said it.")
            .tick();
        assert_eq!(
            late.sent(),
            [
                (0, "Sure, the answer ", Writing),
                (0, "", Dropped),
                (1, "Anna said it.", Writing),
            ]
        );
    }

    #[test]
    fn three_replacements_in_a_row_each_drop_the_draft_devices_hold() {
        use RealtimeReplyState::{Done, Dropped, Writing};
        let replaced = draft_replaced_frame();
        let mut run = Run::default();
        run.push("One ")
            .push(&replaced)
            .push("Two ")
            .push(&replaced)
            .push("Three ")
            .push(&replaced)
            .push("Four.")
            .end(true);
        assert_eq!(
            run.sent(),
            [
                (0, "One ", Writing),
                (0, "", Dropped),
                (1, "Two ", Writing),
                (1, "", Dropped),
                (2, "Three ", Writing),
                (2, "", Dropped),
                (3, "Four.", Writing),
                (3, "Four.", Done),
            ]
        );
    }

    #[test]
    fn a_rewrite_the_turn_does_not_post_ends_dropped() {
        use RealtimeReplyState::{Dropped, Writing};
        let replaced = draft_replaced_frame();
        let mut silent = Run::default();
        silent.push("Sure, the ").push(&replaced).push("NO_REPLY");
        assert!(!silent.publisher.holds_draft());
        silent.end(false);
        assert_eq!(
            silent.sent(),
            [(0, "Sure, the ", Writing), (0, "", Dropped)]
        );
        assert!(silent.over);

        let mut failed = Run::default();
        failed
            .push("Sure, the ")
            .push(&replaced)
            .push("Anna said")
            .end(false);
        assert_eq!(failed.sent().last(), Some(&(1, "Anna said", Dropped)));
        assert!(failed.over);
    }

    #[test]
    fn a_replacement_before_any_text_went_out_sends_nothing_extra() {
        use RealtimeReplyState::{Done, Writing};
        let replaced = draft_replaced_frame();
        let mut held = Run::default();
        held.push("NO_").push(&replaced).push("Sure.").end(true);
        assert_eq!(held.sent(), [(1, "Sure.", Writing), (1, "Sure.", Done)]);

        let mut gated = Run::default();
        gated
            .take("Sure, the ")
            .take(&replaced)
            .take("Anna said it.")
            .tick();
        assert_eq!(gated.sent(), [(1, "Anna said it.", Writing)]);

        let mut quiet = Run::default();
        quiet.take("Sure, the ").take(&replaced).end(false);
        assert!(quiet.sent.is_empty() && quiet.over);
    }

    #[test]
    fn a_replacement_after_the_end_changes_nothing() {
        let mut draft = Draft::default();
        draft.take("It is five.");
        draft.settle(true);
        assert!(!draft.take(&draft_replaced_frame()));
        assert_eq!(
            draft.message(),
            Message {
                draft: 0,
                text: "It is five.".into(),
                state: RealtimeReplyState::Done,
            }
        );
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
        let route = ReplyRoute::new("c-1", "bot-1", "m-1");
        let encoded = route
            .encode(&Message {
                draft: 0,
                text: "Hello".into(),
                state: RealtimeReplyState::Writing,
            })
            .unwrap();
        let reply: RealtimeReply = serde_json::from_slice(&encoded).unwrap();
        assert!(reply.is_valid());
        assert_eq!(reply.message_id, reply_message_id("bot-1", "m-1"));
        assert_eq!(
            (reply.reply_to.as_str(), reply.sender_id.as_str()),
            ("m-1", "bot-1")
        );
        assert_eq!((reply.offset, reply.text.as_str()), (0, "Hello"));

        let ended = route.reply(&Message::replaced(2)).unwrap();
        assert_eq!(ended.message_id, reply.message_id);
        assert_eq!(
            (ended.draft, ended.text.as_str(), ended.state),
            (2, "", RealtimeReplyState::Dropped)
        );
    }
}
