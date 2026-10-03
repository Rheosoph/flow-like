//! A run's answer in a Discord channel: one reply to the message, edited while the run works,
//! and the final answer split into messages of at most 2,000 characters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serenity::builder::{CreateAllowedMentions, CreateEmbed, CreateMessage, EditMessage};
use serenity::http::{Http, HttpError};
use serenity::model::channel::MessageReference;
use serenity::model::id::{ChannelId, MessageId};

use crate::config::Provider;
use crate::render::{self, Embed};
use crate::reply::{Answer, ReplySink, SendError};

fn limit() -> usize {
    Provider::Discord.message_limit()
}

/// The text of an answer with its files: the Markdown text, then the links.
pub fn answer_text(answer: &Answer) -> String {
    let text = answer.text.trim_end();
    let links = render::links_markdown(&answer.links);
    match (text.is_empty(), links.is_empty()) {
        (false, false) => format!("{text}\n\n{links}"),
        (false, true) => text.to_string(),
        (true, _) => links,
    }
}

/// The content of an intermediate edit: as much of the answer as one message holds.
pub fn progress_text(answer: &Answer) -> String {
    let text = answer_text(answer);
    if render::utf16_len(&text) <= limit() {
        return text;
    }
    let first = render::split(&text, limit() - 1)
        .into_iter()
        .next()
        .unwrap_or_default();
    format!("{}…", first.trim_end())
}

/// The messages of the final answer, each within Discord's limit.
pub fn final_parts(answer: &Answer) -> Vec<String> {
    render::split(&answer_text(answer), limit())
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect()
}

/// An answer pings people, never `@everyone`, `@here` or a role: anyone can ask an open bot to
/// write them.
fn mentions() -> CreateAllowedMentions {
    CreateAllowedMentions::new()
        .all_users(true)
        .all_roles(false)
        .everyone(false)
        .replied_user(true)
}

fn embed(embed: &Embed) -> CreateEmbed {
    embed.fields.iter().fold(
        CreateEmbed::new().title(&embed.title).colour(embed.colour),
        |built, field| built.field(&field.name, &field.value, false),
    )
}

/// The HTTP status Discord answered with; 0 when no answer came.
fn status(error: &serenity::Error) -> u16 {
    match error {
        serenity::Error::Http(HttpError::UnsuccessfulRequest(response)) => {
            response.status_code.as_u16()
        }
        _ => 0,
    }
}

fn send_error(error: serenity::Error) -> SendError {
    tracing::warn!(status = status(&error), "Discord did not take the answer");
    SendError::Failed
}

fn reply_to(channel: ChannelId, message: MessageId) -> MessageReference {
    let mut reference = MessageReference::from((channel, message));
    reference.fail_if_not_exists = Some(false);
    reference
}

/// One short text as a reply to `message`, pinging nobody.
pub async fn notice(http: &Http, channel: ChannelId, message: MessageId, text: &str) {
    let notice = CreateMessage::new()
        .content(render::cut(text, limit()))
        .reference_message(reply_to(channel, message))
        .allowed_mentions(CreateAllowedMentions::new());
    if let Err(error) = channel.send_message(http, notice).await {
        send_error(error);
    }
}

/// Where the answer goes when the message it answers is unknown: nowhere.
pub struct NoReply;

#[async_trait]
impl ReplySink for NoReply {
    async fn show(&mut self, _answer: &Answer, _done: bool) -> Result<(), SendError> {
        Ok(())
    }

    fn edit_interval(&self, _shown: u32) -> Duration {
        Duration::from_secs(5)
    }
}

/// Sends one run's answer as a reply to the message that started it.
pub struct DiscordReply {
    http: Arc<Http>,
    channel: ChannelId,
    message: MessageId,
    reply: Option<MessageId>,
}

impl DiscordReply {
    pub fn new(http: Arc<Http>, channel: ChannelId, message: MessageId) -> Self {
        Self {
            http,
            channel,
            message,
            reply: None,
        }
    }

    /// Shows `content` in the reply: sends it the first time, edits it afterwards. A reply that
    /// someone deleted is sent anew.
    async fn put(&mut self, content: &str, plan: Option<&Embed>) -> Result<(), SendError> {
        let embeds = || plan.map(embed).into_iter().collect::<Vec<CreateEmbed>>();
        if let Some(reply) = self.reply {
            let edit = EditMessage::new()
                .content(content)
                .embeds(embeds())
                .allowed_mentions(mentions());
            match self.channel.edit_message(&*self.http, reply, edit).await {
                Ok(_) => return Ok(()),
                Err(error) if status(&error) == 404 => self.reply = None,
                Err(error) => return Err(send_error(error)),
            }
        }
        let message = CreateMessage::new()
            .content(content)
            .embeds(embeds())
            .reference_message(reply_to(self.channel, self.message))
            .allowed_mentions(mentions());
        let sent = self
            .channel
            .send_message(&*self.http, message)
            .await
            .map_err(send_error)?;
        self.reply = Some(sent.id);
        Ok(())
    }
}

#[async_trait]
impl ReplySink for DiscordReply {
    async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError> {
        if !done {
            let plan = answer.plan.as_ref().and_then(render::plan_discord);
            return self.put(&progress_text(answer), plan.as_ref()).await;
        }
        let mut parts = final_parts(answer).into_iter();
        let Some(first) = parts.next() else {
            return Ok(());
        };
        self.put(&first, None).await?;
        for part in parts {
            let message = CreateMessage::new()
                .content(part)
                .allowed_mentions(mentions());
            self.channel
                .send_message(&*self.http, message)
                .await
                .map_err(send_error)?;
        }
        Ok(())
    }

    /// Discord's client stops showing a message that is edited too often: 5 s between the first
    /// edits, then 10 s, then 15 s.
    fn edit_interval(&self, shown: u32) -> Duration {
        Duration::from_secs(match shown {
            0..=3 => 5,
            4..=8 => 10,
            _ => 15,
        })
    }
}
