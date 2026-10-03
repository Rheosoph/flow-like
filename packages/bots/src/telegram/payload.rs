//! The `Chat` payload of a run for a Telegram message: the handle in place of the token, the
//! message it replies to and the message itself, images as `data:` URLs.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use base64::Engine;
use serde_json::{Value, json};
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{Chat, FileId, Message, User};
use tokio::io::AsyncWrite;

use super::filter::{Me, is_image_document};
use crate::limits::{IMAGE_BYTES, ImageBudget};

/// An image of a message, as Telegram describes it.
struct ImageRef {
    file: FileId,
    size: u64,
    mime: String,
}

/// The photo size closest to the limit, or the document when it is an image.
fn image_of(message: &Message) -> Option<ImageRef> {
    if let Some(sizes) = message.photo() {
        let fitting = sizes
            .iter()
            .filter(|size| u64::from(size.file.size) <= IMAGE_BYTES)
            .max_by_key(|size| size.file.size);
        let chosen = fitting.or_else(|| sizes.iter().min_by_key(|size| size.file.size))?;
        return Some(ImageRef {
            file: chosen.file.id.clone(),
            size: u64::from(chosen.file.size),
            mime: "image/jpeg".to_string(),
        });
    }
    let document = message
        .document()
        .filter(|document| is_image_document(document))?;
    Some(ImageRef {
        file: document.file.id.clone(),
        size: u64::from(document.file.size),
        mime: document
            .mime_type
            .as_ref()
            .map_or_else(|| "image/jpeg".to_string(), ToString::to_string),
    })
}

/// Collects a download and refuses more than its limit.
struct Capped {
    bytes: Vec<u8>,
    limit: usize,
}

impl AsyncWrite for Capped {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        chunk: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.bytes.len() + chunk.len() > self.limit {
            return Poll::Ready(Err(io::Error::other("the image is over its size limit")));
        }
        self.bytes.extend_from_slice(chunk);
        Poll::Ready(Ok(chunk.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

async fn download(bot: &Bot, file: FileId) -> Option<Vec<u8>> {
    let file = bot.get_file(file).await.ok()?;
    let mut capped = Capped {
        bytes: Vec::new(),
        limit: usize::try_from(IMAGE_BYTES).unwrap_or(usize::MAX),
    };
    bot.download_file(&file.path, &mut capped).await.ok()?;
    Some(capped.bytes)
}

/// The image of `message` as a `data:` URL when the run's budget takes it.
async fn image_url(bot: &Bot, message: &Message, images: &mut ImageBudget) -> Option<String> {
    let image = image_of(message)?;
    if !images.take(image.size) {
        return None;
    }
    let Some(bytes) = download(bot, image.file).await else {
        images.leave_out();
        return None;
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{encoded}", image.mime))
}

fn name_of(user: Option<&User>) -> String {
    user.map(User::full_name).unwrap_or_default()
}

fn id_of(user: Option<&User>) -> String {
    user.map(|user| user.id.0.to_string()).unwrap_or_default()
}

/// The content parts of one message: `{name}[id: {id}]: {text}` and its image.
async fn content(bot: &Bot, message: &Message, images: &mut ImageBudget) -> Vec<Value> {
    let author = message.from.as_ref();
    let mut parts = Vec::new();
    if let Some(text) = message.text().or_else(|| message.caption()) {
        parts.push(json!({
            "type": "text",
            "text": format!("{}[id: {}]: {text}", name_of(author), id_of(author)),
        }));
    }
    if let Some(url) = image_url(bot, message, images).await {
        parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
    }
    parts
}

pub(crate) fn chat_type(chat: &Chat) -> &'static str {
    if chat.is_private() {
        "private"
    } else if chat.is_supergroup() {
        "supergroup"
    } else if chat.is_channel() {
        "channel"
    } else {
        "group"
    }
}

/// The payload of a run for `message`. `handle` stands for the token.
pub(crate) async fn build(
    bot: &Bot,
    handle: &str,
    me: &Me,
    message: &Message,
    images: &mut ImageBudget,
) -> Value {
    let author = message.from.as_ref();
    let own = content(bot, message, images).await;
    let mut history = Vec::new();
    if let Some(replied) = message.reply_to_message() {
        let earlier = content(bot, replied, images).await;
        if !earlier.is_empty() {
            let replied_author = replied.from.as_ref();
            let role = if replied_author.is_some_and(|user| user.is_bot) {
                "assistant"
            } else {
                "user"
            };
            history
                .push(json!({"role": role, "content": earlier, "name": name_of(replied_author)}));
        }
    }
    history.push(json!({"role": "user", "content": own, "name": name_of(author)}));
    json!({
        "local_session": {
            "bot_token": handle,
            "bot_username": me.username,
            "chat_id": message.chat.id.to_string(),
            "chat_type": chat_type(&message.chat),
            "message_id": message.id.to_string(),
            "chat_title": message.chat.title().unwrap_or(""),
            "reply_to_message_id": message.reply_to_message().map(|replied| replied.id.to_string()),
            "user": {
                "id": id_of(author),
                "name": name_of(author),
                "username": author.and_then(|user| user.username.clone()),
                "is_bot": author.is_some_and(|user| user.is_bot),
            },
        },
        "messages": history,
        "attachments": [],
    })
}
