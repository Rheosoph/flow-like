//! A run's answer in the chat: text deltas, the plan and files of `chat_stream_partial` become
//! throttled edits of one reply; `chat_stream`, `chat_out` and the end of the run give the final
//! text, which is shown once. The stream payloads are read with tolerant structs of their own.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::host::StreamEvent;

/// Files of one answer that are sent as links.
pub const MAX_LINKS: usize = 10;
/// The longest answer text a run sends; what a flow streams beyond it is left out.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Retries of the final answer the provider asked to wait for.
const FINAL_ATTEMPTS: usize = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

/// The steps a flow reports while it works.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<(u32, String)>,
    pub current_step: u32,
    pub current_message: String,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.current_message.is_empty()
    }
}

/// A file of the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub name: Option<String>,
}

/// A run's answer so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// Markdown.
    pub text: String,
    /// While the run works; the final answer has none.
    pub plan: Option<Plan>,
    pub links: Vec<Link>,
}

impl Answer {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && self.links.is_empty()
            && self.plan.as_ref().is_none_or(Plan::is_empty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The provider asked to wait this long.
    RetryAfter(Duration),
    Failed,
}

/// Where one run's answer goes. The provider sends and edits messages in the chat.
#[async_trait]
pub trait ReplySink: Send {
    /// Shows `answer`: the first call sends a reply to the message, later calls edit it.
    /// `done` marks the final answer, which the sink splits into as many messages as it needs.
    async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError>;
    /// The pause before the next edit after `shown` calls of `show`.
    fn edit_interval(&self, shown: u32) -> Duration;
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Partial {
    chunk: Option<Chunk>,
    plan: Option<Reasoning>,
    attachments: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Chunk {
    choices: Vec<ChunkChoice>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ChunkChoice {
    delta: Option<Delta>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Delta {
    content: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Reasoning {
    plan: Vec<(u32, String)>,
    current_step: u32,
    current_message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Full {
    response: Option<Value>,
    attachments: Vec<Value>,
}

/// The text of a `Response`: the content of its last choice that has one.
fn response_text(response: &Value) -> Option<String> {
    response
        .get("choices")?
        .as_array()?
        .iter()
        .rev()
        .find_map(|choice| choice.get("message")?.get("content")?.as_str())
        .map(str::to_string)
}

/// An attachment as the catalog sends it: a URL, or an object with `url` and `name`. Only
/// web addresses become links.
fn link(attachment: &Value) -> Option<Link> {
    let (url, name) = match attachment {
        Value::String(url) => (url.as_str(), None),
        Value::Object(fields) => (
            fields.get("url")?.as_str()?,
            fields
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_string),
        ),
        _ => return None,
    };
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| Link {
        url: url.to_string(),
        name,
    })
}

/// Appends `more` within [`MAX_TEXT_BYTES`]. False when the text is full: it then ends in "…".
fn push_bounded(text: &mut String, more: &str) -> bool {
    const ELLIPSIS: &str = "…";
    if text.len() + more.len() <= MAX_TEXT_BYTES {
        text.push_str(more);
        return true;
    }
    let mut end = MAX_TEXT_BYTES
        .saturating_sub(text.len() + ELLIPSIS.len())
        .min(more.len());
    while !more.is_char_boundary(end) {
        end -= 1;
    }
    text.push_str(&more[..end]);
    text.push_str(ELLIPSIS);
    false
}

/// Follows one run's events and keeps its reply in the chat up to date.
pub struct ReplyStream {
    sink: Box<dyn ReplySink>,
    answer: Answer,
    shown: u32,
    next_show: Option<Instant>,
    changed: bool,
    asked: bool,
    /// The text reached [`MAX_TEXT_BYTES`].
    full: bool,
}

impl ReplyStream {
    pub fn new(sink: Box<dyn ReplySink>) -> Self {
        Self {
            sink,
            answer: Answer::default(),
            shown: 0,
            next_show: None,
            changed: false,
            asked: false,
            full: false,
        }
    }

    pub fn answer(&self) -> &Answer {
        &self.answer
    }

    /// The run asked a question nobody can answer here.
    pub fn asked(&self) -> bool {
        self.asked
    }

    fn add_links(&mut self, attachments: &[Value]) {
        for link in attachments.iter().filter_map(link) {
            if self.answer.links.len() < MAX_LINKS
                && !self.answer.links.iter().any(|known| known.url == link.url)
            {
                self.answer.links.push(link);
                self.changed = true;
            }
        }
    }

    /// Takes one event of the run into the answer. Returns false for an event that asks a
    /// question (`interaction_request`): the caller cancels the run.
    pub fn take(&mut self, event: &StreamEvent) -> bool {
        match event.kind.as_str() {
            "chat_stream_partial" => {
                let partial: Partial =
                    serde_json::from_value(event.payload.clone()).unwrap_or_default();
                if let Some(content) = partial
                    .chunk
                    .and_then(|chunk| chunk.choices.into_iter().next())
                    .and_then(|choice| choice.delta)
                    .and_then(|delta| delta.content)
                    .filter(|content| !content.is_empty() && !self.full)
                {
                    self.full = !push_bounded(&mut self.answer.text, &content);
                    self.changed = true;
                }
                if let Some(plan) = partial.plan {
                    self.answer.plan = Some(Plan {
                        steps: plan.plan,
                        current_step: plan.current_step,
                        current_message: plan.current_message,
                    });
                    self.changed = true;
                }
                self.add_links(&partial.attachments);
            }
            "chat_stream" | "chat_out" => {
                let full: Full = serde_json::from_value(event.payload.clone()).unwrap_or_default();
                if let Some(text) = full
                    .response
                    .as_ref()
                    .and_then(response_text)
                    .filter(|text| !text.trim().is_empty())
                {
                    let mut bounded = String::new();
                    self.full = !push_bounded(&mut bounded, &text);
                    self.answer.text = bounded;
                    self.changed = true;
                }
                self.add_links(&full.attachments);
            }
            "interaction_request" => {
                self.asked = true;
                return false;
            }
            _ => {}
        }
        true
    }

    /// When the next intermediate edit may be sent, if one is due.
    pub fn due(&self) -> Option<Instant> {
        (self.changed && !self.answer.is_empty())
            .then(|| self.next_show.unwrap_or_else(Instant::now))
    }

    /// Sends an intermediate edit when one is due and the throttle allows it.
    pub async fn flush(&mut self) {
        let Some(due) = self.due() else {
            return;
        };
        let now = Instant::now();
        if due > now {
            return;
        }
        self.changed = false;
        let result = self.sink.show(&self.answer, false).await;
        self.shown += 1;
        let pause = match result {
            Err(SendError::RetryAfter(wait)) => {
                self.changed = true;
                wait.min(MAX_RETRY_WAIT)
                    .max(self.sink.edit_interval(self.shown))
            }
            _ => self.sink.edit_interval(self.shown),
        };
        self.next_show = Some(Instant::now() + pause);
    }

    /// The run ended: shows the final answer, once.
    pub async fn finish(mut self) {
        let mut answer = std::mem::take(&mut self.answer);
        answer.plan = None;
        if answer.text.trim().is_empty() && answer.links.is_empty() {
            return;
        }
        for _ in 0..FINAL_ATTEMPTS {
            match self.sink.show(&answer, true).await {
                Err(SendError::RetryAfter(wait)) => {
                    tokio::time::sleep(wait.min(MAX_RETRY_WAIT)).await
                }
                _ => return,
            }
        }
    }

    /// Follows `events` until `done` fires (the run returned), takes what is still queued and
    /// shows the final answer. A question cancels `run`. Returns whether the run asked one.
    pub async fn follow(
        mut self,
        mut events: mpsc::Receiver<StreamEvent>,
        done: CancellationToken,
        run: CancellationToken,
    ) -> bool {
        loop {
            let due = self.due();
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => {
                        if !self.take(&event) {
                            run.cancel();
                        }
                    }
                    None => break,
                },
                () = done.cancelled() => break,
                () = async {
                    match due {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => self.flush().await,
            }
        }
        while let Ok(event) = events.try_recv() {
            if !self.take(&event) {
                run.cancel();
            }
        }
        let asked = self.asked;
        self.finish().await;
        asked
    }
}
