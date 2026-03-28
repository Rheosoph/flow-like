use anyhow::Result;
use flow_like::flow_like_model_provider::response::Response;
use flow_like_catalog::events::chat_event::{
    Attachment, ChatResponse, ChatStreamingResponse, Reasoning,
};
use flow_like_types::{intercom::BufferedInterComHandler, sync::Mutex};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use teloxide::prelude::*;
use teloxide::respond;
use teloxide::types::{
    ChatId, InputFile, MediaKind, MediaText, MessageKind, ParseMode, ReplyParameters,
};

use crate::utils::UiEmitTarget;

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSink {
    pub bot_token: String,
    #[serde(default)]
    pub bot_name: Option<String>,
    #[serde(default)]
    pub bot_description: Option<String>,
    #[serde(default)]
    pub chat_whitelist: Option<Vec<String>>,
    #[serde(default)]
    pub chat_blacklist: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub respond_to_mentions: bool,
    #[serde(default = "default_true")]
    pub respond_to_private: bool,
    #[serde(default = "default_command_prefix")]
    pub command_prefix: String,
}

fn default_true() -> bool {
    true
}

fn default_command_prefix() -> String {
    "/".to_string()
}

lazy_static::lazy_static! {
    static ref TELEGRAM_MANAGER: Arc<flow_like_types::tokio::sync::Mutex<TelegramClientManager>> =
        Arc::new(flow_like_types::tokio::sync::Mutex::new(TelegramClientManager::new()));
}

struct BotInstance {
    token: String,
    handlers: HashMap<String, EventHandler>,
}

#[derive(Clone)]
struct EventHandler {
    event_id: String,
    app_id: String,
    chat_whitelist: Vec<String>,
    chat_blacklist: Vec<String>,
    respond_to_mentions: bool,
    respond_to_private: bool,
    command_prefix: String,
}

struct TelegramClientManager {
    bots: HashMap<String, Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>>,
    running_clients: HashMap<String, flow_like_types::tokio::task::JoinHandle<()>>,
}

impl TelegramClientManager {
    fn new() -> Self {
        Self {
            bots: HashMap::new(),
            running_clients: HashMap::new(),
        }
    }

    async fn add_or_update_bot(
        &mut self,
        app_handle: &AppHandle,
        db: &DbConnection,
        registration: &EventRegistration,
        config: &TelegramSink,
    ) -> Result<()> {
        let token = config.bot_token.clone();

        let handler = EventHandler {
            event_id: registration.event_id.clone(),
            app_id: registration.app_id.clone(),
            chat_whitelist: config.chat_whitelist.clone().unwrap_or_default(),
            chat_blacklist: config.chat_blacklist.clone().unwrap_or_default(),
            respond_to_mentions: config.respond_to_mentions,
            respond_to_private: config.respond_to_private,
            command_prefix: config.command_prefix.clone(),
        };

        if let Some(bot_instance) = self.bots.get(&token) {
            let mut bot = bot_instance.lock().await;
            bot.handlers.insert(registration.event_id.clone(), handler);
            tracing::info!(
                "Updated Telegram bot {} with new handler for event {}",
                &token[..8.min(token.len())],
                registration.event_id
            );
        } else {
            let mut handlers = HashMap::new();
            handlers.insert(registration.event_id.clone(), handler);

            let bot_instance = BotInstance {
                token: token.clone(),
                handlers,
            };

            let bot_arc = Arc::new(flow_like_types::tokio::sync::Mutex::new(bot_instance));
            self.bots.insert(token.clone(), bot_arc.clone());

            self.start_bot(app_handle, db, token.clone(), bot_arc)
                .await?;
        }

        Ok(())
    }

    async fn start_bot(
        &mut self,
        app_handle: &AppHandle,
        db: &DbConnection,
        token: String,
        bot_instance: Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>,
    ) -> Result<()> {
        let app_handle = app_handle.clone();
        let db = db.clone();
        let token_key = token.clone();

        let join_handle = flow_like_types::tokio::spawn(async move {
            if let Err(e) = run_telegram_bot(app_handle, db, token, bot_instance).await {
                tracing::error!("Telegram bot error: {}", e);
            }
        });

        self.running_clients.insert(token_key, join_handle);
        tracing::info!("Started Telegram bot client");

        Ok(())
    }

    async fn remove_handler(&mut self, token: &str, event_id: &str) -> Result<()> {
        if let Some(bot_instance) = self.bots.get(token) {
            let mut bot = bot_instance.lock().await;
            bot.handlers.remove(event_id);

            if bot.handlers.is_empty() {
                drop(bot);
                self.stop_bot(token).await?;
            }
        }

        Ok(())
    }

    async fn stop_bot(&mut self, token: &str) -> Result<()> {
        self.bots.remove(token);

        if let Some(handle) = self.running_clients.remove(token) {
            handle.abort();
            tracing::info!("Stopped Telegram bot: {}", &token[..8.min(token.len())]);
        }

        Ok(())
    }
}

fn is_chat_allowed(chat_id: &str, handler: &EventHandler) -> bool {
    if !handler.chat_whitelist.is_empty() && !handler.chat_whitelist.contains(&chat_id.to_string())
    {
        return false;
    }

    if handler.chat_blacklist.contains(&chat_id.to_string()) {
        return false;
    }

    true
}

fn should_process_message(
    msg: &Message,
    handler: &EventHandler,
    bot_username: Option<&str>,
) -> bool {
    let chat_id = msg.chat.id.to_string();

    tracing::debug!(
        "[TELEGRAM] Checking message in chat {} (private: {})",
        chat_id,
        msg.chat.is_private()
    );

    if !is_chat_allowed(&chat_id, handler) {
        tracing::debug!(
            "[TELEGRAM] Chat {} not allowed by whitelist/blacklist",
            chat_id
        );
        return false;
    }

    let is_private = msg.chat.is_private();

    if is_private {
        tracing::debug!(
            "[TELEGRAM] Private chat, respond_to_private: {}",
            handler.respond_to_private
        );
        return handler.respond_to_private;
    }

    let text = match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(MediaText { text, .. }) => text.as_str(),
            _ => "",
        },
        _ => "",
    };

    tracing::debug!(
        "[TELEGRAM] Group message text: '{}', prefix: '{}', respond_to_mentions: {}",
        text, handler.command_prefix, handler.respond_to_mentions
    );

    if text.starts_with(&handler.command_prefix) {
        tracing::debug!("[TELEGRAM] Message starts with command prefix, processing");
        return true;
    }

    if handler.respond_to_mentions
        && let Some(username) = bot_username
        && text.contains(&format!("@{}", username))
    {
        tracing::debug!(
            "[TELEGRAM] Message mentions bot @{}, processing",
            username
        );
        return true;
    }

    tracing::debug!("[TELEGRAM] Message does not match criteria, skipping");
    false
}

async fn prepare_message_payload(
    bot: &Bot,
    msg: &Message,
    bot_username: Option<&str>,
) -> flow_like_types::Value {
    let mut content_parts = Vec::new();

    let text = match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(MediaText { text, .. }) => text.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };

    let from = msg.from.as_ref();
    let user_name = from.map(|u| u.full_name()).unwrap_or_default();
    let user_id = from.map(|u| u.id.0.to_string()).unwrap_or_default();

    if !text.is_empty() {
        content_parts.push(serde_json::json!({
            "type": "text",
            "text": format!("{}[id: {}]: {}", user_name, user_id, text),
        }));
    }

    if let MessageKind::Common(common) = &msg.kind {
        match &common.media_kind {
            MediaKind::Photo(photo) => {
                if let Some(largest) = photo.photo.last()
                    && let Ok(file) = bot.get_file(largest.file.id.clone()).await
                {
                    let url = format!(
                        "https://api.telegram.org/file/bot{}/{}",
                        bot.token(),
                        file.path
                    );
                    content_parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }));
                }
            }
            MediaKind::Document(doc) => {
                if let Some(mime) = &doc.document.mime_type
                    && mime.type_().as_str() == "image"
                    && let Ok(file) = bot.get_file(doc.document.file.id.clone()).await
                {
                    let url = format!(
                        "https://api.telegram.org/file/bot{}/{}",
                        bot.token(),
                        file.path
                    );
                    content_parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }));
                }
            }
            _ => {}
        }
    }

    // Build message history from reply chain (up to 10 messages)
    let mut messages = Vec::new();
    let mut reply_chain = Vec::new();

    // Walk the reply chain to build history
    let mut current_reply: Option<Message> = msg.reply_to_message().cloned();
    let mut depth = 0;
    const MAX_REPLY_DEPTH: usize = 10;

    while let Some(reply_msg) = current_reply {
        if depth >= MAX_REPLY_DEPTH {
            break;
        }

        let reply_text = match &reply_msg.kind {
            MessageKind::Common(common) => match &common.media_kind {
                MediaKind::Text(MediaText { text, .. }) => text.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };

        if !reply_text.is_empty() {
            let reply_from = reply_msg.from.as_ref();
            let reply_user_name = reply_from
                .map(|u| u.full_name())
                .unwrap_or_else(|| "Unknown".to_string());
            let reply_user_id = reply_from
                .map(|u| u.id.0.to_string())
                .unwrap_or_else(|| "0".to_string());
            let is_bot = reply_from.map(|u| u.is_bot).unwrap_or(false);

            // Build content for this reply message
            let mut reply_content = vec![serde_json::json!({
                "type": "text",
                "text": format!("{}[id: {}]: {}", reply_user_name, reply_user_id, reply_text),
            })];

            // Check for images in reply
            if let MessageKind::Common(common) = &reply_msg.kind
                && let MediaKind::Photo(photo) = &common.media_kind
                && let Some(largest) = photo.photo.last()
                && let Ok(file) = bot.get_file(largest.file.id.clone()).await
            {
                let url = format!(
                    "https://api.telegram.org/file/bot{}/{}",
                    bot.token(),
                    file.path
                );
                reply_content.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                }));
            }

            reply_chain.push(serde_json::json!({
                "role": if is_bot { "assistant" } else { "user" },
                "content": reply_content,
                "name": reply_user_name,
            }));
        }

        current_reply = reply_msg.reply_to_message().cloned();
        depth += 1;
    }

    // Reverse to get chronological order (oldest first)
    reply_chain.reverse();
    messages.extend(reply_chain);

    // Add current message
    messages.push(serde_json::json!({
        "role": "user",
        "content": content_parts,
        "name": user_name,
    }));

    let mut attachments: Vec<String> = Vec::new();
    if let MessageKind::Common(common) = &msg.kind
        && let MediaKind::Document(doc) = &common.media_kind
        && let Some(mime) = &doc.document.mime_type
        && mime.type_().as_str() != "image"
        && let Ok(file) = bot.get_file(doc.document.file.id.clone()).await
    {
        let url = format!(
            "https://api.telegram.org/file/bot{}/{}",
            bot.token(),
            file.path
        );
        attachments.push(url);
    }

    serde_json::json!({
        "local_session": {
            "bot_token": bot.token(),
            "bot_username": bot_username.unwrap_or(""),
            "chat_id": msg.chat.id.to_string(),
            "chat_type": format!("{:?}", msg.chat.kind),
            "message_id": msg.id.to_string(),
            "chat_title": msg.chat.title().unwrap_or(""),
            "reply_to_message_id": msg.reply_to_message().map(|m| m.id.to_string()),
            "user": {
                "id": user_id,
                "name": user_name,
                "username": from.and_then(|u| u.username.clone()),
                "is_bot": from.map(|u| u.is_bot).unwrap_or(false),
            },
        },
        "messages": messages,
        "attachments": attachments,
    })
}

/// Convert markdown to Telegram-compatible HTML using pulldown-cmark
fn markdown_to_telegram_html(text: &str) -> String {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(text, options);
    let mut html = String::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { .. } => html.push_str("<b>"),
                Tag::BlockQuote(_) => {}
                Tag::CodeBlock(CodeBlockKind::Fenced(_)) => html.push_str("<pre>"),
                Tag::CodeBlock(CodeBlockKind::Indented) => html.push_str("<pre>"),
                Tag::List(_) => {}
                Tag::Item => html.push_str("• "),
                Tag::Emphasis => html.push_str("<i>"),
                Tag::Strong => html.push_str("<b>"),
                Tag::Strikethrough => html.push_str("<s>"),
                Tag::Link { dest_url, .. } => {
                    html.push_str(&format!(r#"<a href="{}">"#, dest_url));
                }
                Tag::Image { .. } => {}
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => html.push('\n'),
                TagEnd::Heading(_) => {
                    html.push_str("</b>\n");
                }
                TagEnd::BlockQuote(_) => {}
                TagEnd::CodeBlock => html.push_str("</pre>"),
                TagEnd::List(_) => html.push('\n'),
                TagEnd::Item => html.push('\n'),
                TagEnd::Emphasis => html.push_str("</i>"),
                TagEnd::Strong => html.push_str("</b>"),
                TagEnd::Strikethrough => html.push_str("</s>"),
                TagEnd::Link => html.push_str("</a>"),
                _ => {}
            },
            Event::Text(text) => {
                // Escape HTML special chars in text content
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                html.push_str(&escaped);
            }
            Event::Code(code) => {
                let escaped = code
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                html.push_str(&format!("<code>{}</code>", escaped));
            }
            Event::SoftBreak => html.push(' '),
            Event::HardBreak => html.push('\n'),
            Event::Rule => html.push('\n'),
            _ => {}
        }
    }

    html.trim().to_string()
}

/// Format reasoning/plan steps for Telegram display using simple HTML
fn format_reasoning_for_telegram(reasoning: &Reasoning) -> String {
    let mut output = String::new();

    if reasoning.plan.is_empty() && reasoning.current_message.is_empty() {
        return output;
    }

    output.push_str("┌─ 🧠 <b>Thinking</b> ─────────\n");

    for (step_id, step_text) in &reasoning.plan {
        let is_current = *step_id == reasoning.current_step;
        let is_completed = *step_id < reasoning.current_step;

        let status = if is_completed {
            "│ ✅"
        } else if is_current {
            "│ 🔄"
        } else {
            "│ ⏳"
        };

        // Escape HTML in step text
        let escaped_step = step_text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");

        output.push_str(&format!(
            "{} <b>{}</b>: {}\n",
            status, step_id, escaped_step
        ));

        // Show current message under the active step
        if is_current && !reasoning.current_message.is_empty() {
            let truncated = if reasoning.current_message.len() > 150 {
                format!("{}...", &reasoning.current_message[..147])
            } else {
                reasoning.current_message.clone()
            };
            let escaped = truncated
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            output.push_str(&format!("│    <i>{}</i>\n", escaped));
        }
    }

    output.push_str("└────────────────────");
    output
}

async fn update_telegram_message(
    bot: &Bot,
    chat_id: ChatId,
    response_msg: &mut Option<Message>,
    content: String,
    reasoning_html: Option<String>,
    reply_to: i32,
    last_edit: &mut Instant,
) -> Result<()> {
    let now = Instant::now();
    let time_since_last_edit = now.duration_since(*last_edit);

    if time_since_last_edit < Duration::from_secs(1) {
        return Ok(());
    }

    *last_edit = now;

    let truncated = if content.len() > 4000 {
        format!("{}...", &content[..3997])
    } else {
        content.clone()
    };

    // Convert markdown content to Telegram HTML
    let content_html = markdown_to_telegram_html(&truncated);

    // Combine reasoning (already HTML) with content (converted to HTML)
    let full_html = match &reasoning_html {
        Some(reasoning) => format!("{}\n\n{}", reasoning, content_html),
        None => content_html,
    };

    // Truncate final output if needed
    let final_html = if full_html.len() > 4000 {
        format!("{}...", &full_html[..3997])
    } else {
        full_html
    };

    if let Some(msg) = response_msg.as_ref() {
        // Try with HTML first, fall back to plain text if it fails
        let edit_result = bot
            .edit_message_text(chat_id, msg.id, &final_html)
            .parse_mode(ParseMode::Html)
            .await;

        if edit_result.is_err() {
            // Fallback to plain text if HTML parsing fails
            let _ = bot.edit_message_text(chat_id, msg.id, &truncated).await;
        }
    } else {
        // Try with HTML first, fall back to plain text if it fails
        let send_result = bot
            .send_message(chat_id, &final_html)
            .parse_mode(ParseMode::Html)
            .reply_parameters(ReplyParameters::new(teloxide::types::MessageId(reply_to)))
            .await;

        match send_result {
            Ok(msg) => *response_msg = Some(msg),
            Err(_) => {
                // Fallback to plain text
                match bot
                    .send_message(chat_id, &truncated)
                    .reply_parameters(ReplyParameters::new(teloxide::types::MessageId(reply_to)))
                    .await
                {
                    Ok(msg) => *response_msg = Some(msg),
                    Err(e) => tracing::error!("Failed to send Telegram message: {}", e),
                }
            }
        }
    }

    Ok(())
}

async fn send_telegram_attachments(bot: &Bot, chat_id: ChatId, attachments: &[Attachment]) {
    for attachment in attachments {
        let url = match attachment {
            Attachment::Url(url) => url.clone(),
            Attachment::Complex(complex) => complex.url.clone(),
        };

        let _ = bot
            .send_document(chat_id, InputFile::url(url.parse().unwrap()))
            .await;
    }
}

async fn fire_telegram_event(
    app_handle: &AppHandle,
    _db: &DbConnection,
    event_id: &str,
    _app_id: &str,
    payload: flow_like_types::Value,
    bot: &Bot,
    msg: &Message,
) -> Result<()> {
    use crate::state::TauriEventSinkManagerState;

    tracing::info!("Firing Telegram event: {}", event_id);
    let app_handle_clone = app_handle.clone();

    let context = Arc::new(Mutex::new(Response::new()));
    let response: Arc<Mutex<Option<Message>>> = Arc::new(Mutex::new(None));
    let last_edit: Arc<Mutex<Instant>> =
        Arc::new(Mutex::new(Instant::now() - Duration::from_secs(2)));
    let collected_attachments: Arc<Mutex<Vec<Attachment>>> = Arc::new(Mutex::new(Vec::new()));
    let reasoning_state: Arc<Mutex<Option<Reasoning>>> = Arc::new(Mutex::new(None));

    let context_final = context.clone();
    let response_final = response.clone();
    let last_edit_final = last_edit.clone();
    let collected_attachments_final = collected_attachments.clone();
    let _reasoning_state_final = reasoning_state.clone();
    let bot_final = bot.clone();
    let chat_id = msg.chat.id;
    let reply_to = msg.id.0;

    let bot_clone = bot.clone();

    let callback = BufferedInterComHandler::new(
        Arc::new(move |events| {
            let app_handle = app_handle_clone.clone();
            let cloned_context = context.clone();
            let response = response.clone();
            let last_edit = last_edit.clone();
            let collected_attachments = collected_attachments.clone();
            let reasoning = reasoning_state.clone();
            let bot = bot_clone.clone();
            Box::pin({
                async move {
                    tracing::debug!("[TELEGRAM] Received {} events in callback", events.len());

                    for event in &events {
                        tracing::debug!("[TELEGRAM] Event type: {}", event.event_type);

                        if event.event_type == "chat_stream_partial" {
                            let payload: ChatStreamingResponse = flow_like_types::json::from_value(
                                event.payload.clone(),
                            )
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to deserialize chat_stream_partial payload: {}",
                                    e
                                )
                            })?;

                            // Update reasoning state if present
                            if let Some(plan) = &payload.plan {
                                let mut reasoning_lock = reasoning.lock().await;
                                *reasoning_lock = Some(plan.clone());
                            }

                            // Push chunk and get content in one lock scope
                            let (content_to_send, reasoning_html) = {
                                let mut ctx = cloned_context.lock().await;
                                if let Some(chunk) = &payload.chunk {
                                    tracing::debug!("[TELEGRAM] Pushing chunk to context");
                                    ctx.push_chunk(chunk.clone());
                                }

                                let last_message = ctx.last_message();
                                tracing::debug!(
                                    "[TELEGRAM] Context has {} choices, last_message content len: {:?}",
                                    ctx.choices.len(),
                                    last_message.and_then(|m| m.content.as_ref().map(|c| c.len()))
                                );

                                let content = last_message.and_then(|m| m.content.clone());

                                // Build reasoning HTML (kept separate from content)
                                let reasoning_lock = reasoning.lock().await;
                                let reasoning_html =
                                    reasoning_lock.as_ref().map(format_reasoning_for_telegram);

                                (content, reasoning_html)
                            };

                            // Send if we have reasoning or content
                            if reasoning_html.is_some() || content_to_send.is_some() {
                                let content = content_to_send.unwrap_or_default();
                                tracing::debug!(
                                    "[TELEGRAM] Updating message with {} chars content, reasoning: {}",
                                    content.len(),
                                    reasoning_html.is_some()
                                );

                                let mut resp_lock = response.lock().await;
                                let mut last_edit_lock = last_edit.lock().await;

                                let _ = update_telegram_message(
                                    &bot,
                                    chat_id,
                                    &mut resp_lock,
                                    content,
                                    reasoning_html,
                                    reply_to,
                                    &mut last_edit_lock,
                                )
                                .await;
                            }

                            if !payload.attachments.is_empty() {
                                let mut attachments = collected_attachments.lock().await;
                                attachments.extend(payload.attachments.clone());
                            }
                        }

                        if event.event_type == "chat_stream" {
                            let payload: ChatResponse = flow_like_types::json::from_value(
                                event.payload.clone(),
                            )
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to deserialize chat_out payload: {}", e)
                            })?;

                            if !payload.attachments.is_empty() {
                                let mut attachments = collected_attachments.lock().await;
                                attachments.extend(payload.attachments.clone());
                            }

                            let last_message = payload.response.last_message();
                            if let Some(last_message) = last_message
                                && let Some(content) = &last_message.content
                            {
                                let mut resp_lock = response.lock().await;
                                let mut last_edit_lock = last_edit.lock().await;
                                *last_edit_lock = Instant::now() - Duration::from_secs(2);

                                let _ = update_telegram_message(
                                    &bot,
                                    chat_id,
                                    &mut resp_lock,
                                    content.clone(),
                                    None, // No reasoning on final message
                                    reply_to,
                                    &mut last_edit_lock,
                                )
                                .await;
                            }
                        }
                    }

                    let first_event = events.first();
                    if let Some(first_event) = first_event {
                        crate::utils::emit_throttled(
                            &app_handle,
                            UiEmitTarget::All,
                            &first_event.event_type,
                            events.clone(),
                            std::time::Duration::from_millis(150),
                        );
                    }

                    Ok(())
                }
            })
        }),
        Some(100),
        Some(400),
        Some(true),
    );

    if let Some(manager_state) = app_handle.try_state::<TauriEventSinkManagerState>() {
        let result = match manager_state.0.try_lock() {
            Ok(manager) => {
                manager.fire_event(app_handle, event_id, Some(payload), Some(callback.clone()))
            }
            Err(_) => {
                tracing::error!("EventSinkManager is locked, cannot fire event");
                return Err(anyhow::anyhow!("EventSinkManager is locked"));
            }
        };

        if let Err(e) = result {
            tracing::error!("Failed to fire Telegram event: {}", e);
            return Err(e);
        }

        let context_lock = context_final.lock().await;
        let last_message = context_lock.last_message();
        if let Some(last_message) = last_message
            && let Some(content) = &last_message.content
        {
            let mut resp_lock = response_final.lock().await;
            let mut last_edit_lock = last_edit_final.lock().await;
            *last_edit_lock = Instant::now() - Duration::from_secs(2);

            let _ = update_telegram_message(
                &bot_final,
                chat_id,
                &mut resp_lock,
                content.clone(),
                None, // No reasoning on final flush
                reply_to,
                &mut last_edit_lock,
            )
            .await;

            let attachments = collected_attachments_final.lock().await;
            if !attachments.is_empty() {
                send_telegram_attachments(&bot_final, chat_id, &attachments).await;
            }
        }
    } else {
        return Err(anyhow::anyhow!("EventSinkManager state not available"));
    }

    Ok(())
}

async fn run_telegram_bot(
    app_handle: AppHandle,
    db: DbConnection,
    token: String,
    bot_instance: Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>,
) -> Result<()> {
    tracing::info!(
        "[TELEGRAM] Starting bot with token {}...",
        &token[..8.min(token.len())]
    );

    let bot = Bot::new(&token);

    // Delete any existing webhook to ensure long polling works
    tracing::info!("[TELEGRAM] Deleting any existing webhook...");
    if let Err(e) = bot.delete_webhook().await {
        tracing::warn!(
            "[TELEGRAM] Failed to delete webhook (might not exist): {}",
            e
        );
    } else {
        tracing::info!("[TELEGRAM] Webhook deleted successfully");
    }

    let me = bot.get_me().await?;
    let bot_username = me.username.clone();
    tracing::info!(
        "[TELEGRAM] Bot @{} is connected and listening for messages!",
        bot_username.as_deref().unwrap_or("unknown")
    );

    // Clone values for the handler
    let app_handle_clone = app_handle.clone();
    let db_clone = db.clone();
    let bot_instance_clone = bot_instance.clone();
    let bot_username_clone = bot_username.clone();

    let handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
        let app_handle = app_handle_clone.clone();
        let db = db_clone.clone();
        let bot_instance = bot_instance_clone.clone();
        let bot_username = bot_username_clone.clone();

        async move {
            tracing::debug!(
                "[TELEGRAM] === MESSAGE RECEIVED === from {:?} in chat {}",
                msg.from.as_ref().map(|u| u.full_name()),
                msg.chat.id
            );

            if msg.from.as_ref().map(|u| u.is_bot).unwrap_or(false) {
                tracing::debug!("[TELEGRAM] Ignoring message from bot");
                return respond(());
            }

            let bot_locked = bot_instance.lock().await;
            let handlers: Vec<EventHandler> = bot_locked.handlers.values().cloned().collect();
            drop(bot_locked);

            tracing::debug!("[TELEGRAM] Found {} registered handlers", handlers.len());

            for handler in handlers {
                tracing::debug!(
                    "[TELEGRAM] Checking handler for event {}",
                    handler.event_id
                );

                if !should_process_message(&msg, &handler, bot_username.as_deref()) {
                    tracing::debug!(
                        "[TELEGRAM] Message not matched for event {}",
                        handler.event_id
                    );
                    continue;
                }

                tracing::debug!(
                    "[TELEGRAM] Message matched! Firing event {}",
                    handler.event_id
                );

                let payload = prepare_message_payload(&bot, &msg, bot_username.as_deref()).await;

                // Fire the event (parallelism handled by event bus consumer)
                if let Err(e) = fire_telegram_event(
                    &app_handle,
                    &db,
                    &handler.event_id,
                    &handler.app_id,
                    payload,
                    &bot,
                    &msg,
                )
                .await
                {
                    tracing::error!(
                        "[TELEGRAM] Failed to fire event {}: {}",
                        handler.event_id, e
                    );
                }
            }

            respond(())
        }
    });

    tracing::info!("[TELEGRAM] Setting up dispatcher...");

    let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
        .enable_ctrlc_handler()
        .build();

    tracing::info!("[TELEGRAM] Dispatcher built successfully");
    tracing::info!("[TELEGRAM] Starting long polling - messages should appear below...");

    dispatcher.dispatch().await;

    tracing::warn!("[TELEGRAM] Dispatcher stopped unexpectedly!");

    Ok(())
}

impl TelegramSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS telegram_bots (
                token TEXT PRIMARY KEY,
                bot_name TEXT,
                bot_description TEXT,
                connected INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS telegram_handlers (
                event_id TEXT PRIMARY KEY,
                bot_token TEXT NOT NULL,
                chat_whitelist TEXT,
                chat_blacklist TEXT,
                respond_to_mentions INTEGER NOT NULL DEFAULT 1,
                respond_to_private INTEGER NOT NULL DEFAULT 1,
                command_prefix TEXT NOT NULL DEFAULT '/',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(bot_token) REFERENCES telegram_bots(token) ON DELETE CASCADE
            )",
            [],
        )?;

        Ok(())
    }

    fn add_bot_and_handler(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &TelegramSink,
    ) -> Result<()> {
        let conn = db.lock().unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let chat_whitelist_json = config
            .chat_whitelist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let chat_blacklist_json = config
            .chat_blacklist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        conn.execute(
            "INSERT OR REPLACE INTO telegram_bots (token, bot_name, bot_description, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                config.bot_token,
                config.bot_name,
                config.bot_description,
                now
            ],
        )?;

        conn.execute(
            "INSERT OR REPLACE INTO telegram_handlers
             (event_id, bot_token, chat_whitelist, chat_blacklist, respond_to_mentions, respond_to_private, command_prefix, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                registration.event_id,
                config.bot_token,
                chat_whitelist_json,
                chat_blacklist_json,
                config.respond_to_mentions as i32,
                config.respond_to_private as i32,
                config.command_prefix,
                now,
            ],
        )?;

        Ok(())
    }

    fn remove_handler(db: &DbConnection, event_id: &str) -> Result<String> {
        let conn = db.lock().unwrap();

        let token: String = conn.query_row(
            "SELECT bot_token FROM telegram_handlers WHERE event_id = ?1",
            params![event_id],
            |row| row.get(0),
        )?;

        conn.execute(
            "DELETE FROM telegram_handlers WHERE event_id = ?1",
            params![event_id],
        )?;

        Ok(token)
    }

    fn parse_telegram_config(
        token: String,
        bot_name: Option<String>,
        bot_description: Option<String>,
        chat_whitelist_json: Option<String>,
        chat_blacklist_json: Option<String>,
        respond_to_mentions: bool,
        respond_to_private: bool,
        command_prefix: String,
    ) -> TelegramSink {
        let chat_whitelist = chat_whitelist_json.and_then(|json| serde_json::from_str(&json).ok());
        let chat_blacklist = chat_blacklist_json.and_then(|json| serde_json::from_str(&json).ok());

        TelegramSink {
            bot_token: token,
            bot_name,
            bot_description,
            chat_whitelist,
            chat_blacklist,
            respond_to_mentions,
            respond_to_private,
            command_prefix,
        }
    }

    fn create_event_registration(event_id: String, config: TelegramSink) -> EventRegistration {
        EventRegistration {
            event_id: event_id.clone(),
            name: format!("Telegram Handler {}", event_id),
            r#type: "telegram_bot".to_string(),
            updated_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            config: super::EventConfig::Telegram(config.clone()),
            offline: false,
            app_id: "unknown".to_string(),
            default_payload: None,
            personal_access_token: None,
            oauth_tokens: std::collections::HashMap::new(),
        }
    }

    async fn load_handlers_from_db(
        db: &DbConnection,
    ) -> Result<Vec<(EventRegistration, TelegramSink)>> {
        let conn = db.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT h.event_id, h.bot_token, b.bot_name, b.bot_description,
                    h.chat_whitelist, h.chat_blacklist, h.respond_to_mentions,
                    h.respond_to_private, h.command_prefix
             FROM telegram_handlers h
             JOIN telegram_bots b ON h.bot_token = b.token",
        )?;

        let results = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i32>(6)? != 0,
                row.get::<_, i32>(7)? != 0,
                row.get::<_, String>(8)?,
            ))
        })?;

        let mut handlers = Vec::new();

        for result in results {
            let (
                event_id,
                token,
                bot_name,
                bot_description,
                chat_whitelist_json,
                chat_blacklist_json,
                respond_to_mentions,
                respond_to_private,
                command_prefix,
            ) = result?;

            let config = Self::parse_telegram_config(
                token,
                bot_name,
                bot_description,
                chat_whitelist_json,
                chat_blacklist_json,
                respond_to_mentions,
                respond_to_private,
                command_prefix,
            );

            let registration = Self::create_event_registration(event_id, config.clone());
            handlers.push((registration, config));
        }

        Ok(handlers)
    }
}

#[async_trait::async_trait]
impl EventSink for TelegramSink {
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        tracing::info!("[TELEGRAM_SINK] Starting Telegram sink - initializing bot manager...");

        let handlers = Self::load_handlers_from_db(&db).await?;

        tracing::info!(
            "[TELEGRAM_SINK] Found {} Telegram handlers in telegram_handlers table",
            handlers.len()
        );

        if !handlers.is_empty() {
            let mut manager = TELEGRAM_MANAGER.lock().await;
            for (registration, config) in handlers {
                tracing::info!(
                    "[TELEGRAM_SINK] Registering handler for event {} with token {}...",
                    registration.event_id,
                    &config.bot_token[..8.min(config.bot_token.len())]
                );
                if let Err(e) = manager
                    .add_or_update_bot(app_handle, &db, &registration, &config)
                    .await
                {
                    tracing::error!(
                        "[TELEGRAM_SINK] Failed to initialize Telegram bot for event {}: {}",
                        registration.event_id, e
                    );
                }
            }
        } else {
            tracing::info!("[TELEGRAM_SINK] No handlers found in telegram_handlers table");
        }

        tracing::info!("[TELEGRAM_SINK] Telegram sink started - bot manager ready");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::info!("Stopping Telegram sink...");

        let mut manager = TELEGRAM_MANAGER.lock().await;
        let tokens: Vec<String> = manager.bots.keys().cloned().collect();

        for token in tokens {
            if let Err(e) = manager.stop_bot(&token).await {
                tracing::error!("Failed to stop Telegram bot: {}", e);
            }
        }

        tracing::info!("Telegram sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::add_bot_and_handler(&db, registration, self)?;

        let mut manager = TELEGRAM_MANAGER.lock().await;
        manager
            .add_or_update_bot(app_handle, &db, registration, self)
            .await?;

        tracing::info!(
            "✅ Registered Telegram bot: {} -> event {}",
            self.bot_name.as_deref().unwrap_or("Unnamed Bot"),
            registration.event_id
        );

        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        let token = Self::remove_handler(&db, &registration.event_id)?;

        let mut manager = TELEGRAM_MANAGER.lock().await;
        manager
            .remove_handler(&token, &registration.event_id)
            .await?;

        tracing::info!("Unregistered Telegram handler: {}", registration.event_id);

        Ok(())
    }
}
