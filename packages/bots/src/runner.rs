//! The bots of one service: one provider connection per token, kept while one of its events
//! may connect; the intake of a message (filter → age → rate → the chat's queue → a run permit
//! → the watermark → the run → the reply); bounded restarts; the stop.
//!
//! Log lines name the event and a run number, never a token, a message, a chat or user id, a
//! flow's error text or an error text of a provider's library.

use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{Notify, Semaphore, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::{BotSpec, BotToken, Provider};
use crate::host::{BotHost, Gate, RunEnd};
use crate::limits::{
    ChatQueues, HourBudget, ImageBudget, MINUTE_MS, NoticeKind, Notices, Offer, RUN_TIME_LIMIT,
    Rates, Restarts, SERVICE_RUNS, SETTLED_AFTER, STALE_AFTER_SECS, STOP_GRACE,
};
use crate::reply::{ReplySink, ReplyStream};
use crate::state::{BotEntry, BotsState, LastRun, Outcome, Revisions, Status, public_name};

/// Events of one run waiting for the reply.
const RUN_EVENTS: usize = 256;
/// What a cancelled run gets to end, at a stop or after its time limit.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// How long the provider's connections get to close at a stop.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Counters that changed without a run are written at most this often.
const FLUSH_EVERY: Duration = Duration::from_secs(10);

/// A bot event of the service with its token.
#[derive(Clone, Debug)]
pub struct Bot {
    pub spec: BotSpec,
    pub token: BotToken,
}

/// A new message a provider received, in the terms of the filter (§5.5). The provider checks
/// rules 0 to 2 itself (a new message, not from a bot or a webhook, with text, a caption or an
/// image) and calls [`Intake::ignore`] for one that fails them.
pub struct Message {
    /// Telegram: the update id, the watermark's unit. Discord: `None`.
    pub update_id: Option<i64>,
    /// Telegram: when the message was sent, Unix seconds. `None` skips the age check.
    pub sent_at: Option<i64>,
    /// The chat (Telegram) or channel (Discord) id as the allow and deny lists write it.
    pub chat: String,
    /// A private chat or a direct message.
    pub private: bool,
    /// The bot is mentioned in it, or it replies to one of the bot's messages.
    pub addressed: bool,
    /// Telegram: the text, or the caption, which the prefix is matched against. Discord: empty.
    pub text: String,
    /// Its leading command is addressed to another bot (`/x@otherbot`).
    pub foreign_command: bool,
    /// A waiting flow node took it (`Intake::observed`).
    pub observed: bool,
    /// The provider's own message, handed back to `payload`, `reply` and `notice`.
    pub native: Box<dyn Any + Send + Sync>,
}

impl Message {
    /// A message of `chat` in a group, with nothing else known yet.
    pub fn new(chat: impl Into<String>, native: impl Any + Send + Sync) -> Self {
        Self {
            update_id: None,
            sent_at: None,
            chat: chat.into(),
            private: false,
            addressed: false,
            text: String::new(),
            foreign_command: false,
            observed: false,
            native: Box::new(native),
        }
    }

    pub fn native<T: 'static>(&self) -> Option<&T> {
        self.native.downcast_ref()
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Message")
            .field("update_id", &self.update_id)
            .field("sent_at", &self.sent_at)
            .field("private", &self.private)
            .field("addressed", &self.addressed)
            .finish_non_exhaustive()
    }
}

/// What the provider refused for good: no further attempt until the service restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Token,
    Intents,
}

/// How [`Connection::serve`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The stop token fired.
    Stopped,
    Refused(Refusal),
    /// Anything else. The runner starts the connection again within the restart bound.
    Ended,
}

/// A connection's state as the provider reports it after [`Intake::connected`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connected,
    Reconnecting,
    /// Another program takes this bot's messages.
    Conflict,
    /// Telegram sends this bot's messages to a webhook.
    WebhookSet,
}

/// What this service remembers of a bot when it connects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resume {
    /// Telegram: updates at or below it were handled; the poller starts after it.
    pub watermark: Option<i64>,
    /// Telegram: this service removed the bot's webhook before and never does again.
    pub webhook_cleared: bool,
}

/// What became of a message handed to [`Intake::message`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    /// A run starts now.
    Run,
    /// It waits behind the running message of its chat.
    Queued,
    /// At or below the watermark: handled before.
    Seen,
    /// No open event's filter passes.
    Ignored,
    /// Sent before the age cut of this connect.
    Stale,
    /// Over a rate limit.
    Flood,
    /// Its chat's queue is full.
    Busy,
    /// The connection stops: it is not answered.
    Stopping,
}

/// One provider connection for one token, serving the service's events that use the token.
/// [`serve`](Connection::serve) may be called again after it returned [`End::Ended`]; each call
/// is a fresh start. `payload`, `reply` and `notice` keep working after `serve` returned, until
/// the connection is dropped.
#[async_trait]
pub trait Connection: Send + Sync + 'static {
    /// Connects and hands every update to `intake` until `stop` fires (then returns
    /// [`End::Stopped`] after it confirmed what it read) or the connection ends.
    async fn serve(&self, intake: Intake, stop: CancellationToken) -> End;
    /// The run's `Chat` payload for an admitted message. `handle` takes the token's place in
    /// `local_session.bot_token`; images go through `images`.
    async fn payload(&self, message: &Message, handle: &str, images: &mut ImageBudget) -> Value;
    /// Where the answer to `message` goes.
    fn reply(&self, message: &Message) -> Box<dyn ReplySink>;
    /// One short text in the chat of `message`. Best effort.
    async fn notice(&self, message: &Message, text: &str);
}

/// Opens the provider connections of a service.
pub trait Connector: Send + Sync + 'static {
    /// A connection for `token`, serving `specs`: the service's events that use it, in the
    /// service's order. `None` when this build cannot connect `provider`.
    fn connect(
        &self,
        provider: Provider,
        token: &BotToken,
        specs: &[BotSpec],
    ) -> Option<Arc<dyn Connection>>;
}

/// The providers this build has.
pub struct Providers;

impl Connector for Providers {
    fn connect(
        &self,
        provider: Provider,
        token: &BotToken,
        specs: &[BotSpec],
    ) -> Option<Arc<dyn Connection>> {
        match provider {
            #[cfg(feature = "telegram")]
            Provider::Telegram => Some(crate::telegram::connect(token, specs)),
            #[cfg(feature = "discord")]
            Provider::Discord => Some(crate::discord::connect(token, specs)),
            #[allow(unreachable_patterns)]
            _ => {
                let _ = (token, specs);
                None
            }
        }
    }
}

/// Runs `bots` until `stop` fires and returns only then. Connects nothing before `ready`
/// resolved and the bot's gate is [`Gate::Open`].
pub async fn run(
    bots: Vec<Bot>,
    revisions: Revisions,
    host: Arc<dyn BotHost>,
    ready: impl Future<Output = ()> + Send,
    stop: CancellationToken,
) {
    run_with(bots, revisions, host, Arc::new(Providers), ready, stop).await
}

/// [`run`] with the provider connections of `connector`.
pub async fn run_with(
    bots: Vec<Bot>,
    revisions: Revisions,
    host: Arc<dyn BotHost>,
    connector: Arc<dyn Connector>,
    ready: impl Future<Output = ()> + Send,
    stop: CancellationToken,
) {
    let specs: Vec<&BotSpec> = bots.iter().map(|bot| &bot.spec).collect();
    let doc = BotsState::prepare(host.load(), specs, revisions, host.now());
    let groups = groups(&bots, &host, connector.as_ref());
    let service = Arc::new(Service {
        host,
        doc: Mutex::new(doc),
        save_lock: tokio::sync::Mutex::new(()),
        dirty: AtomicBool::new(true),
        permits: Arc::new(Semaphore::new(SERVICE_RUNS)),
        stop,
        runs: TaskTracker::new(),
        active: Mutex::new(BTreeMap::new()),
        next_run: AtomicU64::new(1),
        origin: Instant::now(),
        unanswered: Mutex::new(Vec::new()),
        ready: AtomicBool::new(false),
        groups,
    });
    service.save().await;
    tokio::select! {
        () = ready => {}
        () = service.stop.cancelled() => {
            service.save().await;
            return;
        }
    }
    service.ready.store(true, Ordering::SeqCst);
    if service.decide() {
        service.save().await;
    }
    let mut tasks = JoinSet::new();
    for group in &service.groups {
        tasks.spawn(run_group(service.clone(), group.clone()));
    }
    tasks.spawn(flush(service.clone()));
    service.stop.cancelled().await;
    service.stop_runs().await;
    if tokio::time::timeout(CLOSE_GRACE, async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tracing::warn!("Bot connections did not close in time");
        tasks.abort_all();
    }
    service.update_all(|entry| entry.running = 0);
    service.save().await;
}

/// One group per token, in the order of the service's events.
fn groups(bots: &[Bot], host: &Arc<dyn BotHost>, connector: &dyn Connector) -> Vec<Arc<Group>> {
    let mut tokens: Vec<(Provider, &BotToken, Vec<&BotSpec>)> = Vec::new();
    for bot in bots {
        match tokens
            .iter_mut()
            .find(|(provider, token, _)| *provider == bot.spec.provider && **token == bot.token)
        {
            Some((_, _, specs)) => specs.push(&bot.spec),
            None => tokens.push((bot.spec.provider, &bot.token, vec![&bot.spec])),
        }
    }
    tokens
        .into_iter()
        .filter_map(|(provider, token, specs)| {
            let specs: Vec<BotSpec> = specs.into_iter().cloned().collect();
            let Some(connection) = connector.connect(provider, token, &specs) else {
                tracing::error!(
                    provider = provider.as_str(),
                    "This build of the agent cannot connect {} bots",
                    provider.name()
                );
                return None;
            };
            Some(Arc::new(Group {
                events: specs.iter().map(|spec| spec.event_id.clone()).collect(),
                gates: specs.iter().map(|spec| host.gate(&spec.event_id)).collect(),
                specs,
                connection,
                link: Mutex::new(Status::Connecting),
                refused: Mutex::new(None),
                cut: Mutex::new(None),
                rates: Mutex::new(Rates::new()),
                queues: Mutex::new(ChatQueues::new()),
                notices: Mutex::new(Notices::default()),
                notified: Mutex::new(HashSet::new()),
                images: Arc::new(Mutex::new(HourBudget::new())),
                drops: Mutex::new(HashMap::new()),
            }))
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Pending {
    event: usize,
    message: Arc<Message>,
}

struct Active {
    cancel: CancellationToken,
    group: Arc<Group>,
    message: Arc<Message>,
}

/// The events of the service that share one token, and their connection.
struct Group {
    events: Vec<String>,
    specs: Vec<BotSpec>,
    connection: Arc<dyn Connection>,
    gates: Vec<watch::Receiver<Gate>>,
    link: Mutex<Status>,
    refused: Mutex<Option<Refusal>>,
    /// Telegram: messages sent before it are skipped. Fixed at each connect.
    cut: Mutex<Option<i64>>,
    rates: Mutex<Rates>,
    queues: Mutex<ChatQueues<Pending>>,
    notices: Mutex<Notices>,
    /// Chats told at this stop that their message is not answered.
    notified: Mutex<HashSet<String>>,
    images: Arc<Mutex<HourBudget>>,
    /// Messages not answered per reason since the last log line, and when it was written.
    drops: Mutex<HashMap<&'static str, (u64, Option<u64>)>>,
}

impl Group {
    fn gate(&self, index: usize) -> Gate {
        *self.gates[index].borrow()
    }

    fn open(&self) -> Vec<usize> {
        (0..self.events.len())
            .filter(|index| self.gate(*index) == Gate::Open)
            .collect()
    }

    fn undecided(&self) -> bool {
        (0..self.events.len()).any(|index| self.gate(index) == Gate::Undecided)
    }
}

struct Service {
    host: Arc<dyn BotHost>,
    doc: Mutex<BotsState>,
    save_lock: tokio::sync::Mutex<()>,
    dirty: AtomicBool,
    permits: Arc<Semaphore>,
    stop: CancellationToken,
    runs: TaskTracker,
    active: Mutex<BTreeMap<u64, Active>>,
    next_run: AtomicU64,
    origin: Instant,
    unanswered: Mutex<Vec<(Arc<Group>, Arc<Message>)>>,
    ready: AtomicBool,
    groups: Vec<Arc<Group>>,
}

impl Service {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn update(&self, event_id: &str, change: impl FnOnce(&mut BotEntry)) {
        if let Some(entry) = lock(&self.doc).bots.get_mut(event_id) {
            change(entry);
        }
        self.dirty.store(true, Ordering::SeqCst);
    }

    fn update_group(&self, group: &Group, mut change: impl FnMut(&mut BotEntry)) {
        let mut doc = lock(&self.doc);
        for event in &group.events {
            if let Some(entry) = doc.bots.get_mut(event) {
                change(entry);
            }
        }
        self.dirty.store(true, Ordering::SeqCst);
    }

    fn update_all(&self, mut change: impl FnMut(&mut BotEntry)) {
        lock(&self.doc).bots.values_mut().for_each(&mut change);
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Writes the document. Later writes never carry an older document than earlier ones.
    async fn save(&self) -> bool {
        let _turn = self.save_lock.lock().await;
        self.dirty.store(false, Ordering::SeqCst);
        let document = lock(&self.doc).clone();
        match self.host.save(&document) {
            Ok(()) => true,
            Err(error) => {
                self.dirty.store(true, Ordering::SeqCst);
                tracing::warn!("Bot state could not be written: {error}");
                false
            }
        }
    }

    fn save_soon(self: &Arc<Self>) {
        let service = self.clone();
        tokio::spawn(async move {
            service.save().await;
        });
    }

    /// The watermark of the group's bot while it can name its updates.
    fn watermark(&self, group: &Group) -> Option<i64> {
        let now = self.host.now();
        let doc = lock(&self.doc);
        group
            .events
            .iter()
            .filter_map(|event| doc.bots.get(event)?.live_watermark(now))
            .max()
    }

    fn advance(&self, group: &Group, update_id: Option<i64>) {
        let Some(id) = update_id else {
            return;
        };
        let now = self.host.now();
        self.update_group(group, |entry| {
            if entry.live_watermark(now).is_none_or(|mark| mark < id) {
                entry.watermark = Some(id);
                entry.watermark_at = Some(now);
            }
        });
    }

    /// `decided` once no gate of the service is undecided after `ready`. True when it changed.
    fn decide(&self) -> bool {
        let decided =
            self.ready.load(Ordering::SeqCst) && !self.groups.iter().any(|group| group.undecided());
        let mut doc = lock(&self.doc);
        if !decided || doc.decided {
            return false;
        }
        doc.decided = true;
        self.dirty.store(true, Ordering::SeqCst);
        true
    }

    /// Counts a message that is not answered and logs the reason at most once a minute.
    fn dropped(&self, group: &Group, event_id: &str, reason: &'static str) {
        let now_ms = self.now_ms();
        let mut drops = lock(&group.drops);
        let (count, logged) = drops.entry(reason).or_insert((0, None));
        *count += 1;
        if logged.is_none_or(|at| now_ms.saturating_sub(at) >= MINUTE_MS) {
            tracing::info!(
                event_id,
                reason,
                count = *count,
                "Bot messages not answered"
            );
            *count = 0;
            *logged = Some(now_ms);
        }
    }

    /// Each event's state from its gate and the connection's state, and `decided`. True when
    /// anything changed.
    fn refresh(&self, group: &Group) -> bool {
        let link = *lock(&group.link);
        let now = self.host.now();
        let decided = self.decide();
        let mut doc = lock(&self.doc);
        let mut changed = false;
        for (index, event) in group.events.iter().enumerate() {
            let (state, hold) = match group.gate(index) {
                Gate::Open => (link, None),
                Gate::Held(code) => (Status::Waiting, Some(code)),
                Gate::Undecided => (Status::Waiting, None),
            };
            let Some(entry) = doc.bots.get_mut(event) else {
                continue;
            };
            if entry.state == state && entry.hold.as_deref() == hold {
                continue;
            }
            tracing::info!(
                event_id = event.as_str(),
                state = state.as_str(),
                hold = hold.unwrap_or(""),
                "Bot state changed"
            );
            if state == Status::Connected && entry.state != Status::Connected {
                entry.connected_at = Some(now);
            } else if state != Status::Connected {
                entry.connected_at = None;
            }
            if state == Status::Reconnecting && entry.state != Status::Reconnecting {
                entry.reconnects += 1;
            }
            entry.state = state;
            entry.hold = hold.map(str::to_string);
            changed = true;
        }
        drop(doc);
        if changed {
            self.dirty.store(true, Ordering::SeqCst);
        }
        changed || decided
    }

    fn set_link(self: &Arc<Self>, group: &Group, status: Status) {
        *lock(&group.link) = status;
        if self.refresh(group) {
            self.save_soon();
        }
    }

    fn notice(&self, group: &Arc<Group>, message: &Arc<Message>, kind: NoticeKind) {
        if !lock(&group.notices).allow(&message.chat, kind, self.now_ms()) {
            return;
        }
        let (group, message) = (group.clone(), message.clone());
        tokio::spawn(async move {
            group.connection.notice(&message, kind.text()).await;
        });
    }

    /// A message that a stop leaves unanswered: counted, and its chat is told.
    fn unanswered(&self, group: &Arc<Group>, event: usize, message: Arc<Message>) {
        self.update(&group.events[event], |entry| entry.dropped_busy += 1);
        self.dropped(group, &group.events[event], "stopping");
        lock(&self.unanswered).push((group.clone(), message));
    }

    async fn tell_unanswered(&self) {
        let pending: Vec<(Arc<Group>, Arc<Message>)> = std::mem::take(&mut *lock(&self.unanswered));
        let mut notices = JoinSet::new();
        for (group, message) in pending {
            if !lock(&group.notified).insert(message.chat.clone()) {
                continue;
            }
            notices.spawn(async move {
                group
                    .connection
                    .notice(&message, NoticeKind::Restart.text())
                    .await;
            });
        }
        let _ = tokio::time::timeout(CANCEL_GRACE, async {
            while notices.join_next().await.is_some() {}
        })
        .await;
    }

    /// Stop: the waiting messages are not answered; runs in progress get [`STOP_GRACE`], then
    /// they are cancelled; every chat that is left without an answer is told.
    async fn stop_runs(&self) {
        for group in &self.groups {
            let waiting = lock(&group.queues).drain_waiting();
            for (_, pending) in waiting {
                self.unanswered(group, pending.event, pending.message);
            }
        }
        self.runs.close();
        let ((), ()) = tokio::join!(self.tell_unanswered(), async {
            let _ = tokio::time::timeout(STOP_GRACE, self.runs.wait()).await;
        });
        let cut_off: Vec<(Arc<Group>, Arc<Message>)> = lock(&self.active)
            .values()
            .map(|active| {
                active.cancel.cancel();
                (active.group.clone(), active.message.clone())
            })
            .collect();
        lock(&self.unanswered).extend(cut_off);
        let ((), ()) = tokio::join!(self.tell_unanswered(), async {
            let _ = tokio::time::timeout(CANCEL_GRACE, self.runs.wait()).await;
        });
    }

    async fn run_one(self: &Arc<Self>, group: &Arc<Group>, pending: Pending) {
        let event_id = group.events[pending.event].clone();
        let permit = tokio::select! {
            permit = self.permits.clone().acquire_owned() => permit.ok(),
            () = self.stop.cancelled() => None,
        };
        let Some(_permit) = permit.filter(|_| !self.stop.is_cancelled()) else {
            self.unanswered(group, pending.event, pending.message);
            return;
        };
        let started_at = self.host.now();
        self.advance(group, pending.message.update_id);
        self.update(&event_id, |entry| entry.running += 1);
        if !self.save().await {
            tracing::warn!(
                event_id = event_id.as_str(),
                "Bot message not answered: its watermark could not be written"
            );
            self.update(&event_id, |entry| {
                entry.running = entry.running.saturating_sub(1);
                entry.failed += 1;
                entry.last = Some(LastRun {
                    at: started_at,
                    finished_at: started_at,
                    outcome: Outcome::Failed,
                });
            });
            return;
        }
        self.update(&event_id, |entry| {
            entry.runs += 1;
            entry.count_run_today(started_at);
        });
        let run = self.next_run.fetch_add(1, Ordering::SeqCst);
        tracing::info!(event_id = event_id.as_str(), run, "Bot run started");
        let clock = Instant::now();
        let cancel = CancellationToken::new();
        lock(&self.active).insert(
            run,
            Active {
                cancel: cancel.clone(),
                group: group.clone(),
                message: pending.message.clone(),
            },
        );
        let mut images = ImageBudget::new(group.images.clone(), self.now_ms());
        let handle = self.host.handle(&event_id);
        let payload = tokio::select! {
            payload = group.connection.payload(&pending.message, &handle, &mut images) => Some(payload),
            () = cancel.cancelled() => None,
        };
        if images.left_out() > 0 {
            self.update(&event_id, |entry| {
                entry.images_left_out += images.left_out()
            });
            self.notice(group, &pending.message, NoticeKind::Image);
        }
        let end = match payload {
            Some(payload) => {
                let sink = group.connection.reply(&pending.message);
                self.drive(&event_id, payload, sink, cancel).await
            }
            None => RunEnd::Cancelled,
        };
        lock(&self.active).remove(&run);
        let outcome = Outcome::from(end);
        let finished_at = self.host.now();
        self.update(&event_id, |entry| {
            entry.running = entry.running.saturating_sub(1);
            if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
                entry.failed += 1;
            }
            entry.last = Some(LastRun {
                at: started_at,
                finished_at,
                outcome,
            });
        });
        tracing::info!(
            event_id = event_id.as_str(),
            run,
            outcome = ?outcome,
            seconds = clock.elapsed().as_secs(),
            "Bot run finished"
        );
        self.save().await;
    }

    /// One run with its reply and its time limit. A question cancels it and counts as failed.
    async fn drive(
        &self,
        event_id: &str,
        payload: Value,
        sink: Box<dyn ReplySink>,
        cancel: CancellationToken,
    ) -> RunEnd {
        let (sender, receiver) = mpsc::channel(RUN_EVENTS);
        let done = CancellationToken::new();
        let run = async {
            let run = self.host.run(event_id, payload, sender, cancel.clone());
            tokio::pin!(run);
            let ended = tokio::select! {
                end = &mut run => (end, false),
                () = tokio::time::sleep(RUN_TIME_LIMIT) => {
                    cancel.cancel();
                    let end = tokio::time::timeout(CANCEL_GRACE, &mut run)
                        .await
                        .unwrap_or(RunEnd::Cancelled);
                    (end, true)
                }
            };
            done.cancel();
            ended
        };
        let reply = ReplyStream::new(sink).follow(receiver, done.clone(), cancel.clone());
        let ((end, timed_out), asked) = tokio::join!(run, reply);
        if asked {
            tracing::info!(
                event_id,
                "Bot run asked a question nobody can answer here; it was cancelled"
            );
            RunEnd::Failed
        } else if timed_out {
            RunEnd::TimedOut
        } else {
            end
        }
    }
}

async fn chat_worker(service: Arc<Service>, group: Arc<Group>, chat: String, first: Pending) {
    let mut next = Some(first);
    while let Some(pending) = next {
        service.run_one(&group, pending).await;
        next = lock(&group.queues).next(&chat);
    }
}

async fn flush(service: Arc<Service>) {
    loop {
        tokio::select! {
            () = tokio::time::sleep(FLUSH_EVERY) => {
                if service.dirty.load(Ordering::SeqCst) {
                    service.save().await;
                }
            }
            () = service.stop.cancelled() => return,
        }
    }
}

/// Wakes on any change of a group's gates.
struct GateWatch {
    notify: Arc<Notify>,
    _tasks: JoinSet<()>,
}

impl GateWatch {
    fn new(gates: &[watch::Receiver<Gate>]) -> Self {
        let notify = Arc::new(Notify::new());
        let mut tasks = JoinSet::new();
        for gate in gates {
            let mut gate = gate.clone();
            let notify = notify.clone();
            tasks.spawn(async move {
                while gate.changed().await.is_ok() {
                    notify.notify_one();
                }
            });
        }
        Self {
            notify,
            _tasks: tasks,
        }
    }

    async fn changed(&self) {
        self.notify.notified().await
    }
}

/// The connection of one token: up while one of its events may connect, started again
/// within the restart bound when it ends, never again after a refusal.
async fn run_group(service: Arc<Service>, group: Arc<Group>) {
    let gates = GateWatch::new(&group.gates);
    let mut restarts = Restarts::new();
    loop {
        if service.refresh(&group) {
            service.save().await;
        }
        if group.open().is_empty() || lock(&group.refused).is_some() {
            tokio::select! {
                () = gates.changed() => continue,
                () = service.stop.cancelled() => return,
            }
        }
        let wait = restarts.wait_ms(service.now_ms());
        if wait > 0 {
            if *lock(&group.link) != Status::Reconnecting {
                tracing::warn!(
                    event_id = group.events[0].as_str(),
                    seconds = wait / 1_000,
                    "Bot connection started too often this hour; it waits"
                );
                service.set_link(&group, Status::Reconnecting);
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(wait)) => {}
                () = gates.changed() => {}
                () = service.stop.cancelled() => return,
            }
            continue;
        }
        restarts.started(service.now_ms());
        *lock(&group.link) = Status::Connecting;
        service.refresh(&group);
        service.save().await;
        let stop = service.stop.child_token();
        let intake = Intake {
            service: service.clone(),
            group: group.clone(),
            stop: stop.clone(),
        };
        let started = Instant::now();
        let serve = group.connection.serve(intake, stop.clone());
        tokio::pin!(serve);
        let end = loop {
            tokio::select! {
                end = &mut serve => break end,
                () = gates.changed() => {
                    if service.refresh(&group) {
                        service.save_soon();
                    }
                    if group.open().is_empty() {
                        tracing::info!(
                            event_id = group.events[0].as_str(),
                            "Bot disconnects: none of its events may run here now"
                        );
                        stop.cancel();
                    }
                }
            }
        };
        let asked_to_stop = stop.is_cancelled();
        stop.cancel();
        match end {
            End::Stopped if asked_to_stop => {
                if service.stop.is_cancelled() {
                    return;
                }
                service.set_link(&group, Status::Connecting);
            }
            End::Refused(refusal) => {
                *lock(&group.refused) = Some(refusal);
                let status = match refusal {
                    Refusal::Token => Status::TokenRefused,
                    Refusal::Intents => Status::IntentsRefused,
                };
                tracing::warn!(
                    event_id = group.events[0].as_str(),
                    state = status.as_str(),
                    "Bot stops connecting until the service restarts"
                );
                service.set_link(&group, status);
            }
            End::Stopped | End::Ended => {
                if service.stop.is_cancelled() {
                    return;
                }
                if started.elapsed() >= SETTLED_AFTER {
                    restarts.settled();
                }
                let pause = Duration::from_millis(restarts.pause_ms(service.now_ms()));
                tracing::warn!(
                    event_id = group.events[0].as_str(),
                    seconds = pause.as_secs(),
                    "Bot connection ended; it starts again after a pause"
                );
                service.set_link(&group, Status::Reconnecting);
                tokio::select! {
                    () = tokio::time::sleep(pause) => {}
                    () = service.stop.cancelled() => return,
                }
            }
        }
    }
}

/// What a provider connection reports to the runner while it serves.
#[derive(Clone)]
pub struct Intake {
    service: Arc<Service>,
    group: Arc<Group>,
    stop: CancellationToken,
}

impl Intake {
    /// The provider accepted the token: the bot's public id and name. Fixes the age cut of
    /// this connect. Another bot than the one this service knew starts without its watermark
    /// and webhook mark.
    pub fn connected(&self, bot_id: u64, bot_name: Option<&str>) -> Resume {
        let now = self.service.host.now();
        let name = bot_name.and_then(public_name);
        let mut resume = Resume::default();
        self.service.update_group(&self.group, |entry| {
            entry.adopt_bot(bot_id);
            entry.bot_name = name.clone();
            resume.watermark = resume.watermark.max(entry.live_watermark(now));
            resume.webhook_cleared |= entry.webhook_cleared;
        });
        *lock(&self.group.cut) = Some(now - STALE_AFTER_SECS);
        tracing::info!(event_id = self.group.events[0].as_str(), "Bot connected");
        self.service.set_link(&self.group, Status::Connected);
        resume
    }

    /// The connection's state changed after it connected.
    pub fn state(&self, state: LinkState) {
        let status = match state {
            LinkState::Connected => Status::Connected,
            LinkState::Reconnecting => Status::Reconnecting,
            LinkState::Conflict => Status::Conflict,
            LinkState::WebhookSet => Status::WebhookSet,
        };
        if *lock(&self.group.link) != status {
            self.service.set_link(&self.group, status);
        }
    }

    /// Telegram: the bot's webhook was removed at its first connect on this service.
    pub fn webhook_cleared(&self) {
        self.service
            .update_group(&self.group, |entry| entry.webhook_cleared = true);
        self.service.save_soon();
    }

    /// The highest update id handled, while it can still name this bot's updates.
    pub fn watermark(&self) -> Option<i64> {
        self.service.watermark(&self.group)
    }

    /// The connection is asked to stop.
    pub fn stopping(&self) -> bool {
        self.stop.is_cancelled()
    }

    /// Telegram: an update reached the poller. True when a waiting flow node took it.
    pub fn observed(&self, update: &Value) -> bool {
        self.service.host.observed(&self.group.events[0], update)
    }

    /// An update that is no message: the watermark moves past it, nothing is counted.
    pub fn pass(&self, update_id: Option<i64>) {
        self.service.advance(&self.group, update_id);
    }

    /// A message that fails rules 0 to 2 of the filter: counted as ignored.
    pub fn ignore(&self, update_id: Option<i64>) {
        if update_id.is_some_and(|id| self.watermark().is_some_and(|mark| id <= mark)) {
            return;
        }
        self.service.advance(&self.group, update_id);
        let now = self.service.host.now();
        let event = self.attributed(None);
        self.service.update(&self.group.events[event], |entry| {
            entry.messages += 1;
            entry.ignored += 1;
            entry.last_message_at = Some(now);
        });
    }

    fn attributed(&self, chosen: Option<usize>) -> usize {
        chosen
            .or_else(|| self.group.open().first().copied())
            .unwrap_or(0)
    }

    /// A new message: the first open event whose filter passes takes it, and it starts at
    /// most one run.
    pub async fn message(&self, message: Message) -> Taken {
        let service = &self.service;
        let group = &self.group;
        if self.stop.is_cancelled() || service.stop.is_cancelled() {
            let event = self.attributed(None);
            service.update(&group.events[event], |entry| entry.messages += 1);
            service.unanswered(group, event, Arc::new(message));
            return Taken::Stopping;
        }
        if message
            .update_id
            .is_some_and(|id| self.watermark().is_some_and(|mark| id <= mark))
        {
            return Taken::Seen;
        }
        service.advance(group, message.update_id);
        let now = service.host.now();
        let chosen = group
            .open()
            .into_iter()
            .find(|index| group.specs[*index].admits(&message));
        let event = self.attributed(chosen);
        let event_id = group.events[event].as_str();
        service.update(event_id, |entry| {
            entry.messages += 1;
            entry.last_message_at = Some(now);
        });
        let Some(event) = chosen else {
            service.update(event_id, |entry| entry.ignored += 1);
            return Taken::Ignored;
        };
        if let (Some(sent), Some(cut)) = (message.sent_at, *lock(&group.cut))
            && sent < cut
        {
            service.update(event_id, |entry| entry.skipped_stale += 1);
            service.dropped(group, event_id, "too_old");
            return Taken::Stale;
        }
        if message.observed && lock(&group.queues).busy(&message.chat) {
            service.update(event_id, |entry| entry.ignored += 1);
            return Taken::Ignored;
        }
        let now_ms = service.now_ms();
        let message = Arc::new(message);
        if !lock(&group.rates).admits(&message.chat, now_ms) {
            service.update(event_id, |entry| entry.dropped_flood += 1);
            service.dropped(group, event_id, "too_many");
            service.notice(group, &message, NoticeKind::Busy);
            return Taken::Flood;
        }
        let offer = lock(&group.queues).offer(
            &message.chat,
            Pending {
                event,
                message: message.clone(),
            },
        );
        match offer {
            Offer::Start(pending) => {
                lock(&group.rates).record(&message.chat, now_ms);
                service.runs.spawn(chat_worker(
                    service.clone(),
                    group.clone(),
                    message.chat.clone(),
                    pending,
                ));
                Taken::Run
            }
            Offer::Queued => {
                lock(&group.rates).record(&message.chat, now_ms);
                Taken::Queued
            }
            Offer::Full(_) => {
                service.update(event_id, |entry| entry.dropped_busy += 1);
                service.dropped(group, event_id, "busy");
                service.notice(group, &message, NoticeKind::Busy);
                Taken::Busy
            }
        }
    }
}
