use anyhow::Result;
use flow_like::flow_like_model_provider::response::Response;
use flow_like_bots::telegram::{Me, facts_of};
use flow_like_bots::{BotSettings, BotSpec, Provider};
use flow_like_catalog::events::chat_event::{
    Attachment, ChatResponse, ChatStreamingResponse, Reasoning,
};
use flow_like_catalog::telegram::session::{TelegramBroadcastEvent, broadcast_tg_event};
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
    CallbackQuery, ChatId, InputFile, MediaKind, MessageKind, ParseMode, ReplyParameters,
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
    #[serde(default = "default_true", deserialize_with = "deserialize_flag")]
    pub respond_to_mentions: bool,
    #[serde(default = "default_true", deserialize_with = "deserialize_flag")]
    pub respond_to_private: bool,
    /// Empty: no prefix is set. A config without the key or with `null` has none.
    #[serde(default, deserialize_with = "deserialize_prefix")]
    pub command_prefix: String,
}

fn default_true() -> bool {
    true
}

/// A flag that is `null` is on, like one without a key.
fn deserialize_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or_else(default_true))
}

fn deserialize_prefix<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

impl TelegramSink {
    /// Which messages start the event: the rule a device applies, without a device's bounds
    /// on the lists and the prefix.
    fn spec(&self, event_id: &str) -> BotSpec {
        BotSpec::unbounded(
            event_id,
            Provider::Telegram,
            BotSettings {
                allow: self.chat_whitelist.clone().unwrap_or_default(),
                deny: self.chat_blacklist.clone().unwrap_or_default(),
                respond_to_mentions: self.respond_to_mentions,
                respond_to_private: self.respond_to_private,
                command_prefix: self.command_prefix.clone(),
            },
        )
    }
}

lazy_static::lazy_static! {
    static ref TELEGRAM_MANAGER: Arc<flow_like_types::tokio::sync::Mutex<TelegramClientManager>> =
        Arc::new(flow_like_types::tokio::sync::Mutex::new(TelegramClientManager::new()));
}

struct BotInstance {
    #[allow(dead_code)]
    // instance identity, mirrors discord::BotInstance::token; run_telegram_bot takes the token as a parameter
    token: String,
    handlers: HashMap<String, EventHandler>,
}

#[derive(Clone)]
struct EventHandler {
    app_id: String,
    /// The event's id and its message rule.
    spec: BotSpec,
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
            app_id: registration.app_id.clone(),
            spec: config.spec(&registration.event_id),
        };

        if let Some(bot_instance) = self.bots.get(&token) {
            let mut bot = bot_instance.lock().await;
            bot.handlers.insert(registration.event_id.clone(), handler);
            tracing::info!(
                "Updated Telegram bot handler for event {}",
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
            tracing::info!("Stopped Telegram bot client");
        }

        Ok(())
    }
}

/// Whether `msg` starts a run of the handler's event: the chat lists, private chats and the
/// rule for groups are those of a device.
fn should_process_message(msg: &Message, handler: &EventHandler, me: &Me) -> bool {
    let facts = facts_of(msg, me);

    tracing::debug!(
        "[TELEGRAM] Checking message in chat {} (private: {}, addressed: {})",
        facts.chat,
        facts.private,
        facts.addressed
    );

    handler.spec.admits(&facts)
}

/// What the flow reads of a message: its text, or the caption of a photo or file, which can
/// start a run like a text does.
fn text_or_caption(message: &Message) -> String {
    message
        .text()
        .or_else(|| message.caption())
        .unwrap_or_default()
        .to_string()
}

async fn prepare_message_payload(
    bot: &Bot,
    msg: &Message,
    bot_username: Option<&str>,
) -> flow_like_types::Value {
    let mut content_parts = Vec::new();

    let text = text_or_caption(msg);

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

        let reply_text = text_or_caption(&reply_msg);

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
                        crate::utils::emit_event_batch_throttled(
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
    tracing::info!("[TELEGRAM] Starting bot");

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
    tracing::info!(
        "[TELEGRAM] Bot @{} is connected and listening for messages!",
        me.username.as_deref().unwrap_or("unknown")
    );
    let me = Me::of(&me);

    // Clone values for the handler
    let app_handle_clone = app_handle.clone();
    let db_clone = db.clone();
    let bot_instance_clone = bot_instance.clone();
    let token_for_broadcast = token.clone();

    let message_handler = Update::filter_message().endpoint(move |bot: Bot, msg: Message| {
        let app_handle = app_handle_clone.clone();
        let db = db_clone.clone();
        let bot_instance = bot_instance_clone.clone();
        let me = me.clone();
        let broadcast_token = token_for_broadcast.clone();

        async move {
            // Broadcast the message to any waiting interaction nodes
            broadcast_tg_event(
                &broadcast_token,
                TelegramBroadcastEvent::Message(Box::new(msg.clone())),
            )
            .await;

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
                let event_id = &handler.spec.event_id;
                tracing::debug!("[TELEGRAM] Checking handler for event {}", event_id);

                if !should_process_message(&msg, &handler, &me) {
                    tracing::debug!("[TELEGRAM] Message not matched for event {}", event_id);
                    continue;
                }

                tracing::debug!("[TELEGRAM] Message matched! Firing event {}", event_id);

                let payload = prepare_message_payload(&bot, &msg, Some(me.username.as_str())).await;

                // Fire the event (parallelism handled by event bus consumer)
                if let Err(e) = fire_telegram_event(
                    &app_handle,
                    &db,
                    event_id,
                    &handler.app_id,
                    payload,
                    &bot,
                    &msg,
                )
                .await
                {
                    tracing::error!("[TELEGRAM] Failed to fire event {}: {}", event_id, e);
                }
            }

            respond(())
        }
    });

    // Callback query handler: broadcast to interaction nodes waiting for button clicks
    let token_for_cb_broadcast = token.clone();
    let callback_handler =
        Update::filter_callback_query().endpoint(move |_bot: Bot, query: CallbackQuery| {
            let broadcast_token = token_for_cb_broadcast.clone();

            async move {
                broadcast_tg_event(
                    &broadcast_token,
                    TelegramBroadcastEvent::CallbackQuery(Box::new(query)),
                )
                .await;
                respond(())
            }
        });

    let handler = dptree::entry()
        .branch(message_handler)
        .branch(callback_handler);

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

        // `add_bot_and_handler` binds every column: the default prefix here is never applied.
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

    /// Updates the bot's row in place: replacing it would delete, through `ON DELETE CASCADE`,
    /// the stored handlers of every other event on the same bot.
    fn upsert_bot(conn: &rusqlite::Connection, config: &TelegramSink, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO telegram_bots (token, bot_name, bot_description, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(token) DO UPDATE SET
                 bot_name = excluded.bot_name,
                 bot_description = excluded.bot_description",
            params![
                config.bot_token,
                config.bot_name,
                config.bot_description,
                now
            ],
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

        Self::upsert_bot(&conn, config, now)?;

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

    #[allow(clippy::too_many_arguments)]
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
                    "[TELEGRAM_SINK] Registering handler for event {}",
                    registration.event_id
                );
                if let Err(e) = manager
                    .add_or_update_bot(app_handle, &db, &registration, &config)
                    .await
                {
                    tracing::error!(
                        "[TELEGRAM_SINK] Failed to initialize Telegram bot for event {}: {}",
                        registration.event_id,
                        e
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sink::EventConfig;
    use serde_json::{Value, json};

    const BOT_ID: u64 = 7_123_456_789;
    const EVENT: &str = "evt_tg";

    fn with(mut base: Value, extra: Value) -> Value {
        for (key, value) in extra.as_object().expect("an object") {
            base[key] = value.clone();
        }
        base
    }

    /// A sink read from an event's config: a token and `settings`.
    fn sink(settings: Value) -> TelegramSink {
        serde_json::from_value(with(json!({"bot_token": "a-token"}), settings))
            .expect("a Telegram config")
    }

    fn handler(settings: Value) -> EventHandler {
        EventHandler {
            app_id: "app".to_string(),
            spec: sink(settings).spec(EVENT),
        }
    }

    fn group() -> Value {
        json!({"id": -100_200, "type": "supergroup", "title": "Team"})
    }

    fn private() -> Value {
        json!({"id": 4242, "type": "private", "first_name": "Ada"})
    }

    /// Whether a message of a person in `chat` with `fields` starts a run under `settings`.
    fn starts(settings: Value, chat: Value, fields: Value) -> bool {
        let person = json!({"id": 4242, "is_bot": false, "first_name": "Ada"});
        let message =
            json!({"message_id": 10, "date": 1_790_000_000, "chat": chat, "from": person});
        let message: Message =
            serde_json::from_str(&with(message, fields).to_string()).expect("a message");
        let me = Me {
            id: BOT_ID,
            username: "helper_bot".to_string(),
        };
        should_process_message(&message, &handler(settings), &me)
    }

    fn in_group(settings: Value, fields: Value) -> bool {
        starts(settings, group(), fields)
    }

    fn text(text: &str) -> Value {
        json!({"text": text})
    }

    /// A text whose first word Telegram marks as a bot command.
    fn command(text: &str) -> Value {
        let length = text.split_whitespace().next().unwrap_or_default().len();
        json!({"text": text, "entities": [{"type": "bot_command", "offset": 0, "length": length}]})
    }

    fn mention() -> Value {
        let entity = json!({"type": "mention", "offset": 4, "length": 11});
        json!({"text": "hey @Helper_Bot look", "entities": [entity]})
    }

    fn reply_to_the_bot() -> Value {
        let bot = json!({"id": BOT_ID, "is_bot": true, "first_name": "Helper"});
        let replied = json!({"message_id": 9, "date": 1_789_999_990, "chat": group(),
            "from": bot, "text": "Sure?"});
        json!({"text": "yes", "reply_to_message": replied})
    }

    fn slash() -> Value {
        json!({"command_prefix": "/"})
    }

    #[test]
    fn a_config_without_a_prefix_has_none() {
        let unset = [
            json!({}),
            json!({"command_prefix": null}),
            json!({"command_prefix": ""}),
        ];
        for settings in unset {
            assert_eq!(sink(settings.clone()).command_prefix, "", "{settings}");
        }
        assert_eq!(sink(slash()).command_prefix, "/");
        assert_eq!(sink(json!({"command_prefix": " "})).command_prefix, " ");
    }

    #[test]
    fn flags_are_on_unless_the_config_turns_them_off() {
        let nulls = json!({"respond_to_mentions": null, "respond_to_private": null});
        for settings in [json!({}), nulls] {
            let read = sink(settings);
            assert!(read.respond_to_mentions && read.respond_to_private);
        }
        let off = sink(json!({"respond_to_mentions": false, "respond_to_private": false}));
        assert!(!off.respond_to_mentions && !off.respond_to_private);
    }

    /// A registration is saved as tagged JSON, which serde reads through a buffer.
    #[test]
    fn a_saved_registration_reads_like_an_event_config() {
        let read = |settings: Value| {
            let tagged = json!({"sink_type": "telegram", "bot_token": "a-token"});
            let saved = with(tagged, settings).to_string();
            match serde_json::from_str(&saved).expect("a saved config") {
                EventConfig::Telegram(sink) => sink,
                other => panic!("a Telegram config, not {other:?}"),
            }
        };
        let unset = read(json!({"command_prefix": null, "respond_to_mentions": null}));
        assert_eq!(unset.command_prefix, "");
        assert!(unset.respond_to_mentions && read(json!({})).respond_to_private);

        let saved = serde_json::to_value(EventConfig::Telegram(sink(json!({})))).expect("JSON");
        assert_eq!(saved["command_prefix"], "");
        assert_eq!(read(saved).command_prefix, "");
        assert_eq!(read(slash()).command_prefix, "/");
    }

    #[test]
    fn the_spec_takes_the_settings_without_the_bounds_of_a_device() {
        let chats: Vec<String> = (1..=300).map(|id| format!("-100{id}")).collect();
        let prefix = "/".repeat(40);
        let settings = json!({
            "chat_whitelist": chats,
            "chat_blacklist": ["7", "8"],
            "respond_to_mentions": false,
            "command_prefix": prefix,
        });
        let spec = sink(settings.clone()).spec(EVENT);
        assert_eq!(spec.event_id, EVENT);
        assert_eq!(spec.provider, Provider::Telegram);
        assert_eq!(spec.allow, chats);
        assert_eq!(spec.deny, ["7", "8"]);
        assert!(!spec.respond_to_mentions && spec.respond_to_private);
        assert_eq!(spec.command_prefix, prefix);

        let prefixed = || text(&format!("{prefix} now"));
        assert!(in_group(settings.clone(), prefixed()));
        assert!(!starts(settings, private(), prefixed()), "off the list");

        let private_off = sink(json!({"respond_to_private": false})).spec(EVENT);
        assert!(private_off.respond_to_mentions && !private_off.respond_to_private);
        assert!(private_off.allow.is_empty() && private_off.deny.is_empty());
        assert_eq!(private_off.command_prefix, "");
    }

    /// Rule R in a group: what a plain message, one with the prefix, a mention and a reply to
    /// the bot do under each pair of settings.
    #[test]
    fn groups_follow_the_rule_of_a_device() {
        let prefix_only = json!({"command_prefix": "/", "respond_to_mentions": false});
        let mentions_off = json!({"respond_to_mentions": false});
        let rows = [
            (slash(), [false, true, true, true]),
            (prefix_only, [false, true, false, false]),
            (json!({}), [true, true, true, true]),
            (mentions_off, [true, true, true, true]),
        ];
        for (settings, expected) in rows {
            let messages = [
                text("just chatting"),
                command("/ask now"),
                mention(),
                reply_to_the_bot(),
            ];
            let started = messages.map(|fields| in_group(settings.clone(), fields));
            assert_eq!(started, expected, "{settings}");
        }
    }

    #[test]
    fn a_caption_counts_like_text() {
        let photo = json!([{"file_id": "p1", "file_unique_id": "u1", "width": 90, "height": 90,
            "file_size": 1000}]);
        let captioned = |caption: &str| json!({"photo": photo, "caption": caption});
        assert!(in_group(slash(), captioned("/describe this")));
        assert!(!in_group(slash(), captioned("nice")));
    }

    /// The text that started a run reaches the flow, also when it is a caption.
    #[test]
    fn the_flow_reads_the_text_or_the_caption() {
        let read = |fields: Value| {
            let chat = group();
            let base = json!({"message_id": 10, "date": 1_790_000_000, "chat": chat});
            let message: Message =
                serde_json::from_str(&with(base, fields).to_string()).expect("a message");
            text_or_caption(&message)
        };
        let photo = json!([{"file_id": "p1", "file_unique_id": "u1", "width": 90, "height": 90}]);
        assert_eq!(read(text("/ask now")), "/ask now");
        assert_eq!(
            read(json!({"photo": photo, "caption": "/describe this"})),
            "/describe this"
        );
        assert_eq!(read(json!({"photo": photo})), "");
    }

    #[test]
    fn a_command_for_another_bot_is_no_prefix_match() {
        assert!(!in_group(slash(), command("/ask@otherbot now")));
        assert!(in_group(slash(), command("/ask@HELPER_BOT now")));
        assert!(in_group(json!({}), command("/ask@otherbot now")));
        let bang = json!({"command_prefix": "!"});
        assert!(in_group(bang.clone(), text("!ask@team now")));
        assert!(in_group(bang.clone(), command("/ask@helper_bot now")));
        assert!(!in_group(bang, command("/ask@otherbot now")));
    }

    /// Telegram names a forum topic's creation as the message every other message in the
    /// topic replies to.
    #[test]
    fn a_topic_the_bot_opened_is_no_reply_to_it() {
        let bot = json!({"id": BOT_ID, "is_bot": true, "first_name": "Helper"});
        let opened = json!({"message_id": 4, "date": 1_789_999_980, "chat": group(), "from": bot,
            "is_topic_message": true, "message_thread_id": 4,
            "forum_topic_created": {"name": "Support", "icon_color": 7_322_096}});
        let topic = json!({"is_topic_message": true, "message_thread_id": 4});
        let thanks = with(text("thanks"), json!({"reply_to_message": opened}));
        assert!(!in_group(slash(), with(topic.clone(), thanks)));
        assert!(in_group(slash(), with(topic, reply_to_the_bot())));
    }

    #[test]
    fn private_chats_follow_respond_to_private() {
        assert!(starts(slash(), private(), text("hello")));
        let groups_only = json!({"respond_to_private": false});
        assert!(!starts(groups_only, private(), text("hello")));
    }

    #[test]
    fn chat_lists_come_before_everything_else() {
        let ask = || command("/ask");
        assert!(in_group(json!({"chat_whitelist": ["-100200"]}), ask()));
        assert!(!in_group(json!({"chat_whitelist": ["4242"]}), ask()));
        assert!(!in_group(json!({"chat_blacklist": ["-100200"]}), ask()));
        let deny = json!({"chat_blacklist": ["4242"]});
        assert!(!starts(deny, private(), text("hello")));
    }

    /// A device ignores a message without text, caption or image; the desktop app considers
    /// every one.
    #[test]
    fn messages_without_text_are_considered() {
        let location = || json!({"location": {"latitude": 52.5, "longitude": 13.4}});
        assert!(in_group(json!({}), location()));
        assert!(!in_group(slash(), location()));
    }

    /// The sink's tables in a database that enforces foreign keys, as the app's does.
    fn database() -> DbConnection {
        let connection = rusqlite::Connection::open_in_memory().expect("a database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .expect("foreign keys");
        let db: DbConnection = Arc::new(std::sync::Mutex::new(connection));
        TelegramSink::init_tables(&db).expect("the tables");
        db
    }

    /// Stores the handler of `event_id` as registering the event does, with a bot of its own
    /// unless `settings` name a token.
    fn store(db: &DbConnection, event_id: &str, settings: Value) -> TelegramSink {
        let own = json!({"bot_token": format!("token-of-{event_id}")});
        let config = sink(with(own, settings));
        let registration =
            TelegramSink::create_event_registration(event_id.to_string(), config.clone());
        TelegramSink::add_bot_and_handler(db, &registration, &config).expect("a stored handler");
        config
    }

    /// What a restart does: `start` builds a handler from every row of the sink's tables.
    async fn restore(db: &DbConnection) -> Vec<BotSpec> {
        let stored = TelegramSink::load_handlers_from_db(db)
            .await
            .expect("the stored handlers");
        let mut restored: Vec<BotSpec> = stored
            .iter()
            .map(|(registration, config)| config.spec(&registration.event_id))
            .collect();
        restored.sort_by(|a, b| a.event_id.cmp(&b.event_id));
        restored
    }

    #[tokio::test]
    async fn a_restart_restores_the_prefix_as_it_was_saved() {
        let db = database();
        let none = store(&db, "evt_none", json!({}));
        let prefix_only = json!({"command_prefix": "/", "respond_to_mentions": false});
        let slash = store(&db, "evt_slash", prefix_only);

        let restored = restore(&db).await;
        assert_eq!(restored, [none.spec("evt_none"), slash.spec("evt_slash")]);
        assert_eq!(restored[0].command_prefix, "");
    }

    /// Events on one bot share its row: storing one updates the row and keeps the stored
    /// handlers of the others.
    #[tokio::test]
    async fn a_restart_restores_every_event_of_a_shared_bot() {
        let db = database();
        let shared = |settings: Value| with(json!({"bot_token": "a-shared-token"}), settings);
        let mentions = store(&db, "evt_mentions", shared(json!({"bot_name": "Old"})));
        let slash = json!({"command_prefix": "/", "bot_name": "Helper"});
        let slash = store(&db, "evt_slash", shared(slash));
        let both = [mentions.spec("evt_mentions"), slash.spec("evt_slash")];
        assert_eq!(restore(&db).await, both);

        let stored = TelegramSink::load_handlers_from_db(&db)
            .await
            .expect("the stored handlers");
        let names = stored.iter().map(|(_, config)| config.bot_name.as_deref());
        assert!(names.eq([Some("Helper"); 2]), "an updated bot row");

        TelegramSink::remove_handler(&db, "evt_slash").expect("a removed handler");
        assert_eq!(restore(&db).await, [mentions.spec("evt_mentions")]);
    }
}
