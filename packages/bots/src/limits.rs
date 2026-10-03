//! Flood and cost limits of the bots of one service (§1.13, §5.5). Every function takes the time
//! as monotonic milliseconds, so tests drive it with a fake clock.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Messages of one chat or channel waiting behind its running one.
pub const CHAT_WAITING: usize = 4;
/// Bot runs of one service at once.
pub const SERVICE_RUNS: usize = 8;
pub const CHAT_RUNS_PER_MINUTE: usize = 10;
pub const BOT_RUNS_PER_MINUTE: usize = 60;
pub const RUN_TIME_LIMIT: Duration = Duration::from_secs(1_800);
/// What runs in progress get at a stop before they are cancelled.
pub const STOP_GRACE: Duration = Duration::from_secs(12);
/// Telegram messages older than this at a connect are skipped.
pub const STALE_AFTER_SECS: i64 = 900;
pub const IMAGES_PER_RUN: usize = 2;
pub const IMAGE_BYTES: u64 = 2 * 1024 * 1024;
pub const IMAGE_BYTES_PER_HOUR: u64 = 256 * 1024 * 1024;
/// Fresh starts of one provider connection within an hour.
pub const RESTARTS_PER_HOUR: usize = 20;
/// Pauses before the fresh starts of a connection that keeps ending.
pub const RESTART_PAUSES_SECS: [u64; 7] = [5, 10, 20, 40, 80, 160, 300];
/// A connection that served this long starts again with the shortest pause.
pub const SETTLED_AFTER: Duration = Duration::from_secs(600);

pub const MINUTE_MS: u64 = 60_000;
pub const HOUR_MS: u64 = 3_600_000;
const PRUNE_ABOVE: usize = 1_024;

/// Starts within a sliding window.
#[derive(Debug, Clone)]
pub struct Window {
    limit: usize,
    span_ms: u64,
    starts: VecDeque<u64>,
}

impl Window {
    pub fn new(limit: usize, span_ms: u64) -> Self {
        Self {
            limit,
            span_ms,
            starts: VecDeque::new(),
        }
    }

    fn expire(&mut self, now_ms: u64) {
        while self
            .starts
            .front()
            .is_some_and(|start| now_ms.saturating_sub(*start) >= self.span_ms)
        {
            self.starts.pop_front();
        }
    }

    pub fn admits(&mut self, now_ms: u64) -> bool {
        self.expire(now_ms);
        self.starts.len() < self.limit
    }

    pub fn record(&mut self, now_ms: u64) {
        self.starts.push_back(now_ms);
    }

    /// How long until the window admits another start.
    pub fn wait_ms(&mut self, now_ms: u64) -> u64 {
        if self.admits(now_ms) {
            return 0;
        }
        let oldest = self.starts[self.starts.len() - self.limit];
        (oldest + self.span_ms).saturating_sub(now_ms)
    }

    fn idle(&mut self, now_ms: u64) -> bool {
        self.expire(now_ms);
        self.starts.is_empty()
    }
}

/// Runs started per chat and per bot.
#[derive(Debug)]
pub struct Rates {
    per_chat: HashMap<String, Window>,
    per_bot: Window,
}

impl Default for Rates {
    fn default() -> Self {
        Self::new()
    }
}

impl Rates {
    pub fn new() -> Self {
        Self {
            per_chat: HashMap::new(),
            per_bot: Window::new(BOT_RUNS_PER_MINUTE, MINUTE_MS),
        }
    }

    /// Whether a run of `chat` may start at `now_ms`.
    pub fn admits(&mut self, chat: &str, now_ms: u64) -> bool {
        let chat_admits = self
            .per_chat
            .get_mut(chat)
            .is_none_or(|window| window.admits(now_ms));
        chat_admits && self.per_bot.admits(now_ms)
    }

    pub fn record(&mut self, chat: &str, now_ms: u64) {
        if self.per_chat.len() > PRUNE_ABOVE {
            self.per_chat.retain(|_, window| !window.idle(now_ms));
        }
        self.per_chat
            .entry(chat.to_string())
            .or_insert_with(|| Window::new(CHAT_RUNS_PER_MINUTE, MINUTE_MS))
            .record(now_ms);
        self.per_bot.record(now_ms);
    }

    /// Admits and records a start of `chat` in one step.
    pub fn admit(&mut self, chat: &str, now_ms: u64) -> bool {
        let admitted = self.admits(chat, now_ms);
        if admitted {
            self.record(chat, now_ms);
        }
        admitted
    }
}

/// Short messages a bot sends to a chat on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoticeKind {
    /// A message was dropped by the flood limits.
    Busy,
    /// An image did not reach the flow.
    Image,
    /// The service stopped before it answered.
    Restart,
}

impl NoticeKind {
    pub fn text(self) -> &'static str {
        match self {
            Self::Busy => "I'm getting too many messages. Try again in a minute.",
            Self::Image => "I can't take that image.",
            Self::Restart => "I'm restarting. Please send that again in a moment.",
        }
    }
}

/// One notice per chat, kind and minute.
#[derive(Debug, Default)]
pub struct Notices {
    last: HashMap<(String, NoticeKind), u64>,
}

impl Notices {
    pub fn allow(&mut self, chat: &str, kind: NoticeKind, now_ms: u64) -> bool {
        if self.last.len() > PRUNE_ABOVE {
            self.last
                .retain(|_, at| now_ms.saturating_sub(*at) < MINUTE_MS);
        }
        let key = (chat.to_string(), kind);
        match self.last.get(&key) {
            Some(at) if now_ms.saturating_sub(*at) < MINUTE_MS => false,
            _ => {
                self.last.insert(key, now_ms);
                true
            }
        }
    }
}

/// What became of a message offered to its chat's queue.
#[derive(Debug, PartialEq, Eq)]
pub enum Offer<T> {
    /// Nothing runs in the chat: the caller starts it now.
    Start(T),
    /// It waits behind the chat's running message.
    Queued,
    /// The chat's queue is full.
    Full(T),
}

/// One running message per chat and at most [`CHAT_WAITING`] waiting behind it.
#[derive(Debug)]
pub struct ChatQueues<T> {
    chats: HashMap<String, VecDeque<T>>,
}

impl<T> Default for ChatQueues<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ChatQueues<T> {
    pub fn new() -> Self {
        Self {
            chats: HashMap::new(),
        }
    }

    pub fn offer(&mut self, chat: &str, item: T) -> Offer<T> {
        match self.chats.get_mut(chat) {
            None => {
                self.chats.insert(chat.to_string(), VecDeque::new());
                Offer::Start(item)
            }
            Some(waiting) if waiting.len() < CHAT_WAITING => {
                waiting.push_back(item);
                Offer::Queued
            }
            Some(_) => Offer::Full(item),
        }
    }

    /// The chat's running message finished: the next waiting one, or the chat is idle again.
    pub fn next(&mut self, chat: &str) -> Option<T> {
        let next = self.chats.get_mut(chat).and_then(VecDeque::pop_front);
        if next.is_none() {
            self.chats.remove(chat);
        }
        next
    }

    /// A message of the chat is running.
    pub fn busy(&self, chat: &str) -> bool {
        self.chats.contains_key(chat)
    }

    pub fn waiting(&self, chat: &str) -> usize {
        self.chats.get(chat).map_or(0, VecDeque::len)
    }

    /// Takes every waiting message; the running ones stay.
    pub fn drain_waiting(&mut self) -> Vec<(String, T)> {
        self.chats
            .iter_mut()
            .flat_map(|(chat, waiting)| waiting.drain(..).map(|item| (chat.clone(), item)))
            .collect()
    }
}

/// Image bytes one bot takes in an hour.
#[derive(Debug, Default)]
pub struct HourBudget {
    started_ms: Option<u64>,
    used: u64,
}

impl HourBudget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes `bytes` from the hour that holds `now_ms`, when they fit.
    pub fn take(&mut self, bytes: u64, now_ms: u64) -> bool {
        if self
            .started_ms
            .is_none_or(|started| now_ms.saturating_sub(started) >= HOUR_MS)
        {
            self.started_ms = Some(now_ms);
            self.used = 0;
        }
        let fits = self.used.saturating_add(bytes) <= IMAGE_BYTES_PER_HOUR;
        if fits {
            self.used += bytes;
        }
        fits
    }
}

/// The images of one run: at most [`IMAGES_PER_RUN`] of [`IMAGE_BYTES`] each, within the bot's
/// hourly budget. What does not fit is left out and counted.
#[derive(Debug)]
pub struct ImageBudget {
    hour: Arc<Mutex<HourBudget>>,
    now_ms: u64,
    taken: usize,
    left_out: u64,
}

impl ImageBudget {
    pub fn new(hour: Arc<Mutex<HourBudget>>, now_ms: u64) -> Self {
        Self {
            hour,
            now_ms,
            taken: 0,
            left_out: 0,
        }
    }

    /// A budget with an hour of its own, for a payload built outside a runner.
    pub fn fresh() -> Self {
        Self::new(Arc::new(Mutex::new(HourBudget::new())), 0)
    }

    /// Whether one more image of up to `bytes` could be taken, without taking it. A provider
    /// asks before it downloads.
    pub fn allows(&self, bytes: u64) -> bool {
        self.taken < IMAGES_PER_RUN && bytes <= IMAGE_BYTES
    }

    /// Takes an image of `bytes`. False leaves it out and counts it.
    pub fn take(&mut self, bytes: u64) -> bool {
        let fits = self.allows(bytes)
            && self
                .hour
                .lock()
                .map(|mut hour| hour.take(bytes, self.now_ms))
                .unwrap_or(false);
        if fits {
            self.taken += 1;
        } else {
            self.left_out += 1;
        }
        fits
    }

    /// Counts an image the provider left out on its own (unreadable, too large to download).
    pub fn leave_out(&mut self) {
        self.left_out += 1;
    }

    pub fn taken(&self) -> usize {
        self.taken
    }

    pub fn left_out(&self) -> u64 {
        self.left_out
    }
}

/// Fresh starts of a connection that keeps ending: growing pauses, and at most
/// [`RESTARTS_PER_HOUR`] starts within any hour.
#[derive(Debug)]
pub struct Restarts {
    starts: Window,
    pauses: usize,
}

impl Default for Restarts {
    fn default() -> Self {
        Self::new()
    }
}

impl Restarts {
    pub fn new() -> Self {
        Self {
            starts: Window::new(RESTARTS_PER_HOUR, HOUR_MS),
            pauses: 0,
        }
    }

    /// The connection starts at `now_ms`.
    pub fn started(&mut self, now_ms: u64) {
        self.starts.record(now_ms);
    }

    /// How long until one more start fits within the hour's bound.
    pub fn wait_ms(&mut self, now_ms: u64) -> u64 {
        self.starts.wait_ms(now_ms)
    }

    /// The connection served long enough: the next pause is the shortest again.
    pub fn settled(&mut self) {
        self.pauses = 0;
    }

    /// The pause before the next start of a connection that ended at `now_ms`.
    pub fn pause_ms(&mut self, now_ms: u64) -> u64 {
        let pause = RESTART_PAUSES_SECS[self.pauses.min(RESTART_PAUSES_SECS.len() - 1)] * 1_000;
        self.pauses += 1;
        pause + self.starts.wait_ms(now_ms + pause)
    }
}
