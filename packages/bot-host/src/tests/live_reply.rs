//! A live-call turn the host answers on the caller's computer feeds the app's reply outlet with
//! the reply as it is written; other turns, and hosts without an outlet, feed nothing.

use super::*;
use crate::live_reply::ReplyOutlet;
use flow_like_types::realtime::{RealtimeReply, RealtimeReplyState};

#[derive(Default)]
struct Heard(Mutex<Vec<RealtimeReply>>);

impl Heard {
    fn replies(&self) -> Vec<RealtimeReply> {
        self.0.lock().unwrap().clone()
    }

    fn states(&self) -> Vec<RealtimeReplyState> {
        self.replies().iter().map(|reply| reply.state).collect()
    }
}

impl ReplyOutlet for Heard {
    fn reply(&self, reply: RealtimeReply) {
        self.0.lock().unwrap().push(reply);
    }
}

/// The app on this computer: it hears live-call replies.
struct Listening(Arc<Heard>);

impl TurnEvents for Listening {
    fn turn(&self, _: &TurnInfo) {}

    fn replies(&self) -> Option<Arc<dyn ReplyOutlet>> {
        Some(self.0.clone())
    }
}

/// Writes its pieces into the job's reply stream as a model would, then ends with `answer`.
struct Writer {
    pieces: &'static [&'static str],
    answer: Option<&'static str>,
    streamed: Mutex<Vec<bool>>,
}

impl Writer {
    fn new(pieces: &'static [&'static str], answer: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            pieces,
            answer,
            streamed: Mutex::default(),
        })
    }

    fn streamed(&self) -> Vec<bool> {
        self.streamed.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnRunner for Writer {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        self.streamed
            .lock()
            .unwrap()
            .push(job.reply_stream.is_some());
        for piece in self.pieces {
            if let Some(sink) = &job.reply_stream {
                sink.push(piece);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        match self.answer {
            Some(text) => Ok(text.to_owned()),
            None => Err(RunError::SessionChanged),
        }
    }

    fn cancel(&self, _: &str) {}
}

fn start(entry: InboxEntry, runner: Arc<Writer>, events: Arc<dyn TurnEvents>) -> AccountHost {
    let hub = Arc::new(DeviceHub::default());
    *hub.entries.lock().unwrap() = vec![entry];
    hub.turns.lock().unwrap().push(turn("bt_1"));
    let host = AccountHost::new(hub, runner, events, Arc::new(Semaphore::new(2)), config());
    tokio::spawn(host.clone().run());
    host
}

async fn until(what: &str, check: impl Fn() -> bool) {
    for _ in 0..800 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}");
}

fn ended(host: &AccountHost) -> Option<TurnStatus> {
    host.turns()
        .first()
        .map(|turn| turn.status)
        .filter(|status| {
            matches!(
                status,
                TurnStatus::Done | TurnStatus::Cancelled | TurnStatus::Failed
            )
        })
}

#[tokio::test]
async fn a_live_call_turn_is_heard_while_it_is_written_and_ends_done() {
    let heard = Arc::new(Heard::default());
    let runner = Writer::new(&["Here is ", "the plan."], Some("Here is the plan."));
    let host = start(
        spoken(&["bot"]),
        runner.clone(),
        Arc::new(Listening(heard.clone())),
    );
    until("the posted reply", || ended(&host).is_some()).await;
    assert_eq!(ended(&host), Some(TurnStatus::Done));
    assert_eq!(runner.streamed(), [true]);

    let replies = heard.replies();
    let first = replies.first().expect("the call heard the reply");
    assert_eq!(
        (first.state, first.text.as_str()),
        (RealtimeReplyState::Writing, "Here is ")
    );
    let last = replies.last().unwrap();
    assert_eq!(
        (last.state, last.offset, last.text.as_str()),
        (RealtimeReplyState::Done, 0, "Here is the plan.")
    );
    for reply in &replies {
        assert!(reply.is_valid());
        assert_eq!(reply.message_id, reply_message_id("bot", "m1"));
        assert_eq!(
            (
                reply.conversation_id.as_str(),
                reply.reply_to.as_str(),
                reply.sender_id.as_str(),
                reply.draft
            ),
            ("conv", "m1", "bot", 0)
        );
    }
    assert!(
        heard.states()[..replies.len() - 1]
            .iter()
            .all(|state| matches!(
                state,
                RealtimeReplyState::Writing | RealtimeReplyState::Working
            )),
        "{:?}",
        heard.states()
    );
}

#[tokio::test]
async fn a_live_call_turn_that_posts_nothing_ends_dropped() {
    let heard = Arc::new(Heard::default());
    let runner = Writer::new(&["Sure, the answer "], None);
    let host = start(
        spoken(&["bot"]),
        runner.clone(),
        Arc::new(Listening(heard.clone())),
    );
    until("the dropped stream", || {
        heard.states().last() == Some(&RealtimeReplyState::Dropped)
    })
    .await;
    assert_eq!(ended(&host), Some(TurnStatus::Cancelled));
    assert_eq!(
        heard.states(),
        [RealtimeReplyState::Writing, RealtimeReplyState::Dropped]
    );
    assert_eq!(heard.replies()[0].text, "Sure, the answer ");
}

#[tokio::test]
async fn a_turn_that_is_not_a_live_call_feeds_nothing() {
    let heard = Arc::new(Heard::default());
    let runner = Writer::new(&["Hello"], Some("Hello"));
    let host = start(
        plain("hi"),
        runner.clone(),
        Arc::new(Listening(heard.clone())),
    );
    until("the reply", || ended(&host).is_some()).await;
    assert_eq!(ended(&host), Some(TurnStatus::Done));
    assert_eq!(runner.streamed(), [false]);
    assert!(heard.replies().is_empty());
}

#[tokio::test]
async fn a_host_without_an_outlet_streams_no_live_call() {
    let runner = Writer::new(&["Here is ", "the plan."], Some("Here is the plan."));
    let host = start(spoken(&["bot"]), runner.clone(), Arc::new(Quiet));
    until("the reply", || ended(&host).is_some()).await;
    assert_eq!(ended(&host), Some(TurnStatus::Done));
    assert_eq!(runner.streamed(), [false]);
}
