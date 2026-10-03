//! Answers in a Telegram chat: one reply that is edited while the run works, then the final
//! answer split into messages of at most 4,096 characters.

use std::time::Duration;

use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId, ParseMode, ReplyParameters, ThreadId};
use teloxide::{ApiError, RequestError};

use crate::config::Provider;
use crate::render::{escape_html, links_markdown, plan_telegram, split, telegram_html, utf16_len};
use crate::reply::{Answer, ReplySink, SendError};

const PRIVATE_EDIT_INTERVAL: Duration = Duration::from_secs(1);
const GROUP_EDIT_INTERVAL: Duration = Duration::from_secs(3);

fn limit() -> usize {
    Provider::Telegram.message_limit()
}

/// A message body: Telegram HTML, and the plain text sent when Telegram refuses the HTML.
pub(crate) struct Body {
    html: String,
    plain: String,
}

impl Body {
    pub(crate) fn markdown(markdown: &str) -> Self {
        Self {
            html: telegram_html(markdown),
            plain: markdown.trim().to_string(),
        }
    }

    pub(crate) fn plain(text: &str) -> Self {
        Self {
            html: escape_html(text),
            plain: text.to_string(),
        }
    }

    fn is_empty(&self) -> bool {
        self.plain.trim().is_empty() && self.html.trim().is_empty()
    }
}

/// Where a message goes: the chat, its forum topic, and the message it answers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target {
    pub chat: ChatId,
    pub thread: Option<ThreadId>,
    pub reply_to: Option<MessageId>,
}

pub(crate) fn send_error(error: &RequestError) -> SendError {
    match error {
        RequestError::RetryAfter(wait) => SendError::RetryAfter(wait.duration()),
        _ => SendError::Failed,
    }
}

/// Sends `body` to `target`.
pub(crate) async fn send(
    bot: &Bot,
    target: Target,
    body: &Body,
) -> Result<MessageId, RequestError> {
    let attempt = |html: bool| {
        let text = if html { &body.html } else { &body.plain };
        let mut request = bot.send_message(target.chat, text.clone());
        if html {
            request = request.parse_mode(ParseMode::Html);
        }
        if let Some(thread) = target.thread {
            request = request.message_thread_id(thread);
        }
        if let Some(message) = target.reply_to {
            request = request
                .reply_parameters(ReplyParameters::new(message).allow_sending_without_reply());
        }
        request
    };
    let sent = match attempt(true).await {
        Err(RequestError::Api(ApiError::CantParseEntities(_))) => attempt(false).await,
        other => other,
    };
    sent.map(|message| message.id)
}

async fn edit(bot: &Bot, chat: ChatId, id: MessageId, body: &Body) -> Result<(), RequestError> {
    let attempt = |html: bool| {
        let text = if html { &body.html } else { &body.plain };
        let mut request = bot.edit_message_text(chat, id, text.clone());
        if html {
            request = request.parse_mode(ParseMode::Html);
        }
        request
    };
    let edited = match attempt(true).await {
        Err(RequestError::Api(ApiError::CantParseEntities(_))) => attempt(false).await,
        other => other,
    };
    match edited {
        Ok(_) | Err(RequestError::Api(ApiError::MessageNotModified)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// The start of `text` in at most `units` UTF-16 code units (what Telegram counts), ending in
/// "…" when it was longer.
fn fit(text: &str, units: usize) -> String {
    if utf16_len(text) <= units {
        return text.to_string();
    }
    let mut start = split(text, units.saturating_sub(1))
        .into_iter()
        .next()
        .unwrap_or_default();
    start.push('…');
    start
}

/// The answer while the run works: the plan box above the text so far, in one message.
fn progress(answer: &Answer) -> Body {
    let plan = answer.plan.as_ref().map(plan_telegram).unwrap_or_default();
    let room = limit().saturating_sub(utf16_len(&plan) + 2);
    let text = fit(answer.text.trim(), room);
    let html = match (plan.is_empty(), text.is_empty()) {
        (true, _) => telegram_html(&text),
        (false, true) => plan,
        (false, false) => format!("{plan}\n\n{}", telegram_html(&text)),
    };
    Body { html, plain: text }
}

/// The final answer's text with its files as links.
fn final_markdown(answer: &Answer) -> String {
    let mut markdown = answer.text.trim().to_string();
    let links = links_markdown(&answer.links);
    if !links.is_empty() {
        if !markdown.is_empty() {
            markdown.push_str("\n\n");
        }
        markdown.push_str(&links);
    }
    markdown
}

/// One run's answer in its chat.
pub(crate) struct Reply {
    bot: Bot,
    target: Target,
    private: bool,
    message: Option<MessageId>,
    final_parts: usize,
}

impl Reply {
    pub(crate) fn new(bot: Bot, target: Target, private: bool) -> Self {
        Self {
            bot,
            target,
            private,
            message: None,
            final_parts: 0,
        }
    }

    async fn put(&mut self, body: &Body) -> Result<(), RequestError> {
        match self.message {
            Some(id) => edit(&self.bot, self.target.chat, id, body).await,
            None => {
                let id = send(&self.bot, self.target, body).await?;
                self.message = Some(id);
                Ok(())
            }
        }
    }

    async fn put_final(&mut self, answer: &Answer) -> Result<(), RequestError> {
        let parts: Vec<Body> = split(&final_markdown(answer), limit())
            .iter()
            .map(|part| Body::markdown(part))
            .filter(|body| !body.is_empty())
            .collect();
        let follow_up = Target {
            reply_to: None,
            ..self.target
        };
        for (index, body) in parts.iter().enumerate().skip(self.final_parts) {
            if index == 0 {
                self.put(body).await?;
            } else {
                send(&self.bot, follow_up, body).await?;
            }
            self.final_parts = index + 1;
        }
        Ok(())
    }
}

#[async_trait]
impl ReplySink for Reply {
    async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError> {
        let shown = if done {
            self.put_final(answer).await
        } else {
            let body = progress(answer);
            if body.is_empty() {
                return Ok(());
            }
            self.put(&body).await
        };
        shown.map_err(|error| send_error(&error))
    }

    fn edit_interval(&self, _shown: u32) -> Duration {
        if self.private {
            PRIVATE_EDIT_INTERVAL
        } else {
            GROUP_EDIT_INTERVAL
        }
    }
}

/// The sink of a message this connection cannot answer.
pub(crate) struct Silent;

#[async_trait]
impl ReplySink for Silent {
    async fn show(&mut self, _answer: &Answer, _done: bool) -> Result<(), SendError> {
        Ok(())
    }

    fn edit_interval(&self, _shown: u32) -> Duration {
        GROUP_EDIT_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reply::{Link, Plan};

    #[test]
    fn progress_fits_one_message_with_the_plan_on_top() {
        for text in ["ä".repeat(5_000), "😀".repeat(3_000), "漢字".repeat(2_500)] {
            let answer = Answer {
                text,
                plan: Some(Plan {
                    steps: vec![(1, "Read the question".into())],
                    current_step: 1,
                    current_message: String::new(),
                }),
                links: Vec::new(),
            };
            let body = progress(&answer);
            assert!(body.html.starts_with("┌─ 🧠 <b>Thinking</b>"));
            let plan = utf16_len(&plan_telegram(answer.plan.as_ref().expect("a plan")));
            assert!(plan + 2 + utf16_len(&body.plain) <= limit());
            assert!(body.plain.ends_with('…'));
        }
    }

    #[test]
    fn short_progress_is_shown_whole() {
        let answer = Answer {
            text: "Grüße 😀".into(),
            plan: None,
            links: Vec::new(),
        };
        assert_eq!(progress(&answer).plain, "Grüße 😀");
    }

    #[test]
    fn final_answer_lists_its_files_as_links() {
        let answer = Answer {
            text: "Done.".into(),
            plan: None,
            links: vec![Link {
                url: "https://example.com/report.pdf".into(),
                name: Some("report.pdf".into()),
            }],
        };
        assert_eq!(
            final_markdown(&answer),
            "Done.\n\n📎 Attachments:\n- [report.pdf](https://example.com/report.pdf)"
        );
    }

    #[test]
    fn a_plain_body_is_escaped_for_html() {
        let body = Body::plain("a < b & c");
        assert_eq!(body.html, "a &lt; b &amp; c");
        assert_eq!(body.plain, "a < b & c");
    }
}
