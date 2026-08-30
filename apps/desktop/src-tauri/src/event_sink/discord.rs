use anyhow::Result;
use flow_like::flow_like_model_provider::response::Response;
use flow_like_catalog::events::chat_event::{
    Attachment, ChatResponse, ChatStreamingResponse, Reasoning,
};
use flow_like_types::{intercom::BufferedInterComHandler, sync::Mutex};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serenity::all::{
    Colour, CreateAttachment, CreateEmbed, CreateMessage, EditMessage, GatewayIntents,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use crate::utils::UiEmitTarget;

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSink {
    pub token: String,
    #[serde(default)]
    pub bot_name: Option<String>,
    #[serde(default)]
    pub bot_description: Option<String>,
    #[serde(default, deserialize_with = "deserialize_intents")]
    pub intents: Option<Vec<String>>,
    #[serde(default)]
    pub channel_whitelist: Option<Vec<String>>,
    #[serde(default)]
    pub channel_blacklist: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub respond_to_mentions: bool,
    #[serde(default = "default_true")]
    pub respond_to_dms: bool,
    #[serde(default = "default_command_prefix")]
    pub command_prefix: String,
}

fn default_true() -> bool {
    true
}

fn default_command_prefix() -> String {
    "!".to_string()
}

fn deserialize_intents<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(deserializer)
}

fn parse_intents(intent_names: &[String]) -> GatewayIntents {
    let mut intents = GatewayIntents::empty();

    for name in intent_names {
        let intent = match name.as_str() {
            "Guilds" => GatewayIntents::GUILDS,
            "GuildMembers" => GatewayIntents::GUILD_MEMBERS,
            "GuildModeration" => GatewayIntents::GUILD_MODERATION,
            "GuildEmojisAndStickers" => GatewayIntents::GUILD_EMOJIS_AND_STICKERS,
            "GuildIntegrations" => GatewayIntents::GUILD_INTEGRATIONS,
            "GuildWebhooks" => GatewayIntents::GUILD_WEBHOOKS,
            "GuildInvites" => GatewayIntents::GUILD_INVITES,
            "GuildVoiceStates" => GatewayIntents::GUILD_VOICE_STATES,
            "GuildPresences" => GatewayIntents::GUILD_PRESENCES,
            "GuildMessages" => GatewayIntents::GUILD_MESSAGES,
            "GuildMessageReactions" => GatewayIntents::GUILD_MESSAGE_REACTIONS,
            "GuildMessageTyping" => GatewayIntents::GUILD_MESSAGE_TYPING,
            "DirectMessages" => GatewayIntents::DIRECT_MESSAGES,
            "DirectMessageReactions" => GatewayIntents::DIRECT_MESSAGE_REACTIONS,
            "DirectMessageTyping" => GatewayIntents::DIRECT_MESSAGE_TYPING,
            "MessageContent" => GatewayIntents::MESSAGE_CONTENT,
            "GuildScheduledEvents" => GatewayIntents::GUILD_SCHEDULED_EVENTS,
            "AutoModerationConfiguration" => GatewayIntents::AUTO_MODERATION_CONFIGURATION,
            "AutoModerationExecution" => GatewayIntents::AUTO_MODERATION_EXECUTION,
            _ => {
                tracing::warn!("Unknown gateway intent: {}", name);
                GatewayIntents::empty()
            }
        };
        intents |= intent;
    }

    intents
}

// Global Discord client manager
// This ensures all bots run in the same Tauri process
lazy_static::lazy_static! {
    static ref DISCORD_MANAGER: Arc<flow_like_types::tokio::sync::Mutex<DiscordClientManager>> =
        Arc::new(flow_like_types::tokio::sync::Mutex::new(DiscordClientManager::new()));
}

struct BotInstance {
    token: String,
    intents: serenity::all::GatewayIntents,
    handlers: HashMap<String, EventHandler>,
}

#[derive(Clone)]
struct EventHandler {
    event_id: String,
    app_id: String,
    channel_whitelist: Vec<String>,
    channel_blacklist: Vec<String>,
    respond_to_mentions: bool,
    respond_to_dms: bool,
    #[allow(dead_code)]
    // configured + persisted prefix; the live should_process_message path does not consult it yet
    command_prefix: String,
}

struct DiscordClientManager {
    bots: HashMap<String, Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>>,
    running_clients: HashMap<String, flow_like_types::tokio::task::JoinHandle<()>>,
}

impl DiscordClientManager {
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
        config: &DiscordSink,
    ) -> Result<()> {
        let token = config.token.clone();

        // Parse intents
        let intents = if let Some(intent_list) = &config.intents {
            parse_intents(intent_list)
        } else {
            // Default intents if none specified
            serenity::all::GatewayIntents::GUILDS
                | serenity::all::GatewayIntents::GUILD_MESSAGES
                | serenity::all::GatewayIntents::MESSAGE_CONTENT
        };

        let handler = EventHandler {
            event_id: registration.event_id.clone(),
            app_id: registration.app_id.clone(),
            channel_whitelist: config.channel_whitelist.clone().unwrap_or_default(),
            channel_blacklist: config.channel_blacklist.clone().unwrap_or_default(),
            respond_to_mentions: config.respond_to_mentions,
            respond_to_dms: config.respond_to_dms,
            command_prefix: config.command_prefix.clone(),
        };

        // Check if bot already exists
        if let Some(bot_instance) = self.bots.get(&token) {
            // Update existing bot with new handler
            let mut bot = bot_instance.lock().await;
            bot.handlers.insert(registration.event_id.clone(), handler);
            println!(
                "Updated Discord bot {} with new handler for event {}",
                token[..8].to_string() + "...",
                registration.event_id
            );
        } else {
            // Create new bot instance
            let mut handlers = HashMap::new();
            handlers.insert(registration.event_id.clone(), handler);

            let bot_instance = BotInstance {
                token: token.clone(),
                intents,
                handlers,
            };

            let bot_arc = Arc::new(flow_like_types::tokio::sync::Mutex::new(bot_instance));
            self.bots.insert(token.clone(), bot_arc.clone());

            // Start the Discord client
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

        // Get intents from bot instance
        let intents = {
            let bot = bot_instance.lock().await;
            bot.intents
        };

        let join_handle = flow_like_types::tokio::spawn(async move {
            if let Err(e) = run_discord_bot(app_handle, db, token, intents, bot_instance).await {
                tracing::error!("Discord bot error: {}", e);
            }
        });

        self.running_clients.insert(token_key, join_handle);
        tracing::info!("Started Discord bot client");

        Ok(())
    }

    #[allow(dead_code)] // superset of DiscordSink::remove_handler: also prunes the orphaned discord_bots row
    fn remove_handler_from_db(db: &DbConnection, event_id: &str) -> Result<String> {
        let conn = db.lock().unwrap();

        // Get the bot token before removing
        let token: String = conn.query_row(
            "SELECT bot_token FROM discord_handlers WHERE event_id = ?1",
            params![event_id],
            |row| row.get(0),
        )?;

        // Remove handler
        conn.execute(
            "DELETE FROM discord_handlers WHERE event_id = ?1",
            params![event_id],
        )?;

        // Check if bot still has handlers
        let handler_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM discord_handlers WHERE bot_token = ?1",
            params![&token],
            |row| row.get(0),
        )?;

        // If no handlers left, remove bot
        if handler_count == 0 {
            conn.execute("DELETE FROM discord_bots WHERE token = ?1", params![&token])?;
        }

        Ok(token)
    }

    async fn remove_handler(&mut self, token: &str, event_id: &str) -> Result<()> {
        if let Some(bot_instance) = self.bots.get(token) {
            let mut bot = bot_instance.lock().await;
            bot.handlers.remove(event_id);

            // If no more handlers, stop the bot
            if bot.handlers.is_empty() {
                drop(bot); // Release lock before stopping
                self.stop_bot(token).await?;
            }
        }

        Ok(())
    }

    async fn stop_bot(&mut self, token: &str) -> Result<()> {
        // Remove bot instance
        self.bots.remove(token);

        // Stop the running client
        if let Some(handle) = self.running_clients.remove(token) {
            handle.abort();
            tracing::info!("Stopped Discord bot: {}", &token[..8]);
        }

        Ok(())
    }
}

// Handler for Discord events
struct DiscordEventHandler {
    app_handle: AppHandle,
    db: DbConnection,
    bot_instance: Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>,
}

#[serenity::async_trait]
impl serenity::client::EventHandler for DiscordEventHandler {
    async fn message(
        &self,
        ctx: serenity::client::Context,
        msg: serenity::model::channel::Message,
    ) {
        if msg.author.bot {
            return;
        }

        let bot = self.bot_instance.lock().await;
        let handlers: Vec<EventHandler> = bot.handlers.values().cloned().collect();
        let bot_token = bot.token.clone();
        drop(bot);

        // Get bot user id from cache
        let bot_user_id = ctx.cache.current_user().id.get();

        for handler in handlers {
            // Check if should process this message
            if !should_process_message(&ctx, &msg, &handler) {
                println!(
                    "Skipping message from channel {} for event {}",
                    msg.channel_id, handler.event_id
                );
                continue;
            }

            // Prepare payload with context
            let payload = prepare_message_payload(&ctx, &msg, &bot_token, Some(bot_user_id)).await;

            // Fire the event (parallelism handled by event bus consumer)
            if let Err(e) = fire_discord_event(
                &self.app_handle,
                &self.db,
                &handler.event_id,
                &handler.app_id,
                payload,
                &ctx,
                &msg,
            )
            .await
            {
                eprintln!("Failed to fire Discord event {}: {}", handler.event_id, e);
            }
        }
    }

    async fn ready(&self, _ctx: serenity::client::Context, ready: serenity::model::gateway::Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}

fn is_channel_allowed(channel_id: &str, handler: &EventHandler) -> bool {
    // Check whitelist
    if !handler.channel_whitelist.is_empty()
        && !handler.channel_whitelist.contains(&channel_id.to_string())
    {
        return false;
    }

    // Check blacklist
    if handler.channel_blacklist.contains(&channel_id.to_string()) {
        return false;
    }

    true
}

#[allow(dead_code)] // prefix/mention targeting, superseded by should_process_message which does not check the prefix
fn is_message_targeted(msg: &serenity::model::channel::Message, handler: &EventHandler) -> bool {
    // Check if message starts with prefix
    if msg.content.starts_with(&handler.command_prefix) {
        return true;
    }

    // Check if bot is mentioned
    !msg.mentions.is_empty()
}

fn should_process_message(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    handler: &EventHandler,
) -> bool {
    let channel_id = msg.channel_id.to_string();

    // Check channel whitelist/blacklist
    if !is_channel_allowed(&channel_id, handler) {
        println!(
            "Channel {} is not allowed for event {}",
            channel_id, handler.event_id
        );
        return false;
    }

    // Check if DM (guild_id is None for DMs)
    if msg.guild_id.is_none() {
        return handler.respond_to_dms;
    }

    let author = &msg.author;
    let me = ctx.cache.current_user();
    if author.id == me.id {
        return false;
    }

    if let Some(referenced) = msg.referenced_message.as_ref()
        && referenced.author.id == me.id
    {
        return true;
    }

    let only_respond_to_mentions = handler.respond_to_mentions;
    let includes_mention = msg.mentions.iter().any(|u| u.id == me.id);

    if only_respond_to_mentions && !includes_mention {
        return false;
    }

    true
}

async fn prepare_message_payload(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    bot_token: &str,
    bot_user_id: Option<u64>,
) -> flow_like_types::Value {
    // Build content array with text and images
    let mut content_parts = Vec::new();

    // Add text content if present
    if !msg.content.is_empty() {
        let nickname = msg
            .author_nick(&ctx.http)
            .await
            .unwrap_or(msg.author.display_name().to_string());
        content_parts.push(serde_json::json!({
            "type": "text",
            "text": format!("{}[id: {}]:{}", nickname, msg.author.id, msg.content.clone()),
        }));
    }

    // Add image attachments as content parts
    for attachment in &msg.attachments {
        // Check if attachment is an image
        if let Some(content_type) = &attachment.content_type
            && content_type.starts_with("image/")
        {
            content_parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": attachment.url.clone(),
                }
            }));
        }
    }

    // Fetch previous messages (up to 10) for context
    let mut messages = Vec::new();

    if let Ok(history) = msg
        .channel_id
        .messages(&ctx.http, serenity::builder::GetMessages::new().limit(10))
        .await
    {
        // Reverse to get chronological order
        for hist_msg in history.iter().rev() {
            // Skip messages after the current one
            if hist_msg.id >= msg.id {
                continue;
            }

            // Build content for historical message
            let mut hist_content = Vec::new();

            if !hist_msg.content.is_empty() {
                let nickname = hist_msg
                    .author_nick(&ctx.http)
                    .await
                    .unwrap_or(hist_msg.author.display_name().to_string());
                hist_content.push(serde_json::json!({
                    "type": "text",
                    "text": format!("{}[id: {}]:{}", nickname, hist_msg.author.id, hist_msg.content.clone()),
                }));
            }

            // Add images from historical message
            for attachment in &hist_msg.attachments {
                if let Some(content_type) = &attachment.content_type
                    && content_type.starts_with("image/")
                {
                    hist_content.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": {
                            "url": attachment.url.clone(),
                        }
                    }));
                }
            }

            // Only add message if it has content
            if !hist_content.is_empty() {
                messages.push(serde_json::json!({
                    "role": if hist_msg.author.bot { "assistant" } else { "user" },
                    "content": hist_content,
                    "name": hist_msg.author.name.clone(),
                }));
            }
        }
    }

    // Add current message
    messages.push(serde_json::json!({
        "role": "user",
        "content": content_parts,
        "name": msg.author.name.clone(),
    }));

    // Collect non-image attachments separately
    let other_attachments: Vec<String> = msg
        .attachments
        .iter()
        .filter(|a| {
            a.content_type
                .as_ref()
                .map(|ct| !ct.starts_with("image/"))
                .unwrap_or(true)
        })
        .map(|a| a.url.clone())
        .collect();

    serde_json::json!({
        "local_session": {
            "bot_token": bot_token,
            "bot_user_id": bot_user_id.map(|id| id.to_string()),
            "guild_id": msg.guild_id.map(|id| id.to_string()),
            "message_id": msg.id.to_string(),
            "channel_id": msg.channel_id.to_string(),
            "user": {
                "id": msg.author.id.to_string(),
                "name": msg.author.name,
                "discriminator": msg.author.discriminator,
                "bot": msg.author.bot,
            },
        },
        "messages": messages,
        "attachments": other_attachments,
    })
}

/// Create a Discord embed for reasoning/plan steps
/// Discord embed limits: 6000 total chars, 1024 per field, 25 fields max
fn create_reasoning_embed(reasoning: &Reasoning) -> Option<CreateEmbed> {
    if reasoning.plan.is_empty() && reasoning.current_message.is_empty() {
        return None;
    }

    let mut embed = CreateEmbed::new()
        .title("🧠 Thinking...")
        .colour(Colour::from_rgb(255, 191, 0)); // Amber color

    let current_step = reasoning.current_step;
    let total_steps = reasoning.plan.len();

    // Count completed steps
    let completed_count = reasoning
        .plan
        .iter()
        .filter(|(id, _)| *id < current_step)
        .count();

    // If we have many completed steps, show a summary instead
    if completed_count > 2 {
        embed = embed.field(
            format!("✅ {} steps completed", completed_count),
            "─".repeat(20),
            false,
        );
    } else {
        // Show individual completed steps (max 2)
        for (step_id, step_text) in &reasoning.plan {
            if *step_id < current_step {
                let truncated = if step_text.len() > 60 {
                    format!("{}...", &step_text[..57])
                } else {
                    step_text.clone()
                };
                embed = embed.field(format!("✅ Step {}", step_id), truncated, false);
            }
        }
    }

    // Show current step with full detail
    for (step_id, step_text) in &reasoning.plan {
        if *step_id == current_step {
            let mut value = if step_text.len() > 200 {
                format!("{}...", &step_text[..197])
            } else {
                step_text.clone()
            };

            // Add current message under the active step
            if !reasoning.current_message.is_empty() {
                let truncated = if reasoning.current_message.len() > 150 {
                    format!("{}...", &reasoning.current_message[..147])
                } else {
                    reasoning.current_message.clone()
                };
                value = format!("{}\n> *{}*", value, truncated);
            }

            embed = embed.field(format!("🔄 Step {}", step_id), value, false);
        }
    }

    // Show only next 2 upcoming steps briefly
    let mut upcoming_shown = 0;
    for (step_id, step_text) in &reasoning.plan {
        if *step_id > current_step && upcoming_shown < 2 {
            let truncated = if step_text.len() > 50 {
                format!("{}...", &step_text[..47])
            } else {
                step_text.clone()
            };
            embed = embed.field(format!("⏳ Step {}", step_id), truncated, false);
            upcoming_shown += 1;
        }
    }

    // If there are more upcoming steps, show count
    let remaining = total_steps.saturating_sub(current_step as usize + upcoming_shown);
    if remaining > 0 {
        embed = embed.field(
            format!("⏳ +{} more steps", remaining),
            "─".repeat(10),
            false,
        );
    }

    Some(embed)
}

#[allow(clippy::too_many_arguments)]
async fn update_discord_message(
    ctx: &serenity::client::Context,
    message: &serenity::model::channel::Message,
    response_msg: &mut Option<serenity::all::Message>,
    content: String,
    attachments: &[Attachment],
    reasoning_embed: Option<CreateEmbed>,
    last_edit: &mut Instant,
    edit_count: &mut u32,
) -> Result<bool> {
    let now = Instant::now();
    let time_since_last_edit = now.duration_since(*last_edit);

    // Adaptive rate limiting: Discord's client stops rendering after too many rapid edits.
    // These intervals are intentionally generous to keep the UI responsive.
    let min_interval = if *edit_count < 3 {
        Duration::from_millis(5000)
    } else if *edit_count < 8 {
        Duration::from_millis(10000)
    } else {
        Duration::from_millis(15000)
    };

    if time_since_last_edit < min_interval {
        return Ok(false);
    }

    *last_edit = now;
    *edit_count += 1;

    // Prepare Discord attachments from our Attachment enum
    let discord_attachments = prepare_discord_attachments(attachments).await;

    if let Some(msg) = response_msg.as_mut() {
        let mut edit = EditMessage::new().content(content.clone());

        // Add reasoning embed if present
        if let Some(embed) = reasoning_embed {
            edit = edit.embed(embed);
        }

        // Note: Discord API doesn't support editing attachments directly
        // We include attachment URLs in the message content instead
        if !attachments.is_empty() {
            let attachment_text = format_attachments_text(attachments);
            edit = edit.content(format!("{}\n\n{}", content, attachment_text));
        }

        let _ = msg.edit(&ctx.http, edit).await;
    } else {
        let mut reply = CreateMessage::new()
            .content(content)
            .reference_message(message);

        // Add reasoning embed if present
        if let Some(embed) = reasoning_embed {
            reply = reply.embed(embed);
        }

        // Add attachments to the initial message
        for attachment in discord_attachments {
            reply = reply.add_file(attachment);
        }

        match message.channel_id.send_message(&ctx.http, reply).await {
            Ok(msg) => *response_msg = Some(msg),
            Err(e) => tracing::error!("Failed to send Discord message: {}", e),
        }
    }

    Ok(true)
}

async fn prepare_discord_attachments(attachments: &[Attachment]) -> Vec<CreateAttachment> {
    let mut discord_attachments = Vec::new();

    for attachment in attachments {
        match attachment {
            Attachment::Url(url) => {
                // Try to fetch and attach the file
                if let Ok(response) = flow_like_types::reqwest::get(url).await
                    && let Ok(bytes) = response.bytes().await
                {
                    let filename = url.split('/').next_back().unwrap_or("attachment");
                    discord_attachments.push(CreateAttachment::bytes(bytes.to_vec(), filename));
                }
            }
            Attachment::Complex(complex) => {
                // Try to fetch and attach the file from URL
                if let Ok(response) = flow_like_types::reqwest::get(&complex.url).await
                    && let Ok(bytes) = response.bytes().await
                {
                    let filename = complex.name.as_deref().unwrap_or_else(|| {
                        complex.url.split('/').next_back().unwrap_or("attachment")
                    });
                    discord_attachments.push(CreateAttachment::bytes(bytes.to_vec(), filename));
                }
            }
        }
    }

    discord_attachments
}

fn format_attachments_text(attachments: &[Attachment]) -> String {
    let mut text = String::from("📎 Attachments:");

    for attachment in attachments {
        match attachment {
            Attachment::Url(url) => {
                text.push_str(&format!("\n- {}", url));
            }
            Attachment::Complex(complex) => {
                let name = complex.name.as_deref().unwrap_or("Attachment");
                text.push_str(&format!("\n- [{}]({})", name, complex.url));
                if let Some(preview) = &complex.preview_text {
                    text.push_str(&format!(" - {}", preview));
                }
            }
        }
    }

    text
}

async fn fire_discord_event(
    app_handle: &AppHandle,
    _db: &DbConnection,
    event_id: &str,
    _app_id: &str,
    payload: flow_like_types::Value,
    ctx: &serenity::client::Context,
    message: &serenity::model::channel::Message,
) -> Result<()> {
    use crate::state::TauriEventSinkManagerState;

    println!("🔥 Firing Discord event: {}", event_id);
    let app_handle_clone = app_handle.clone();

    let context = Arc::new(Mutex::new(Response::new()));
    let response: Arc<Mutex<Option<serenity::all::Message>>> = Arc::new(Mutex::new(None));
    let last_edit: Arc<Mutex<Instant>> =
        Arc::new(Mutex::new(Instant::now() - Duration::from_secs(2)));
    let collected_attachments: Arc<Mutex<Vec<Attachment>>> = Arc::new(Mutex::new(Vec::new()));
    let reasoning_state: Arc<Mutex<Option<Reasoning>>> = Arc::new(Mutex::new(None));
    let edit_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let last_sent_content: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // Clone for final flush
    let context_final = context.clone();
    let response_final = response.clone();
    let last_edit_final = last_edit.clone();
    let collected_attachments_final = collected_attachments.clone();
    let _reasoning_state_final = reasoning_state.clone();
    let edit_count_final = edit_count.clone();
    let ctx_final = ctx.clone();
    let message_final = message.clone();

    let ctx_clone = ctx.clone();
    let message_clone = message.clone();

    let callback = BufferedInterComHandler::new(
        Arc::new(move |events| {
            let app_handle = app_handle_clone.clone();
            let cloned_context = context.clone();
            let response = response.clone();
            let last_edit = last_edit.clone();
            let collected_attachments = collected_attachments.clone();
            let reasoning = reasoning_state.clone();
            let edit_count = edit_count.clone();
            let last_sent_content = last_sent_content.clone();
            let ctx = ctx_clone.clone();
            let message = message_clone.clone();
            Box::pin({
                async move {
                    for event in &events {
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

                            // Handle chunks
                            let mut context = cloned_context.lock().await;
                            if let Some(chunk) = &payload.chunk {
                                context.push_chunk(chunk.clone());
                            }
                            drop(context);

                            // Collect attachments
                            if !payload.attachments.is_empty() {
                                let mut attachments = collected_attachments.lock().await;
                                attachments.extend(payload.attachments.clone());
                            }

                            // Update message with rate limiting
                            let context = cloned_context.lock().await;
                            let last_message = context.last_message();
                            let content = last_message
                                .and_then(|m| m.content.clone())
                                .unwrap_or_default();

                            // Create reasoning embed
                            let reasoning_lock = reasoning.lock().await;
                            let reasoning_embed =
                                reasoning_lock.as_ref().and_then(create_reasoning_embed);
                            drop(reasoning_lock);

                            if !content.is_empty() || reasoning_embed.is_some() {
                                // Skip if content hasn't changed since last successful edit
                                let last_content = last_sent_content.lock().await;
                                if *last_content == content && reasoning_embed.is_none() {
                                    drop(last_content);
                                } else {
                                    drop(last_content);
                                    let mut resp_lock = response.lock().await;
                                    let mut last_edit_lock = last_edit.lock().await;
                                    let mut edit_count_lock = edit_count.lock().await;
                                    let attachments = collected_attachments.lock().await;

                                    let did_edit = update_discord_message(
                                        &ctx,
                                        &message,
                                        &mut resp_lock,
                                        content.clone(),
                                        &attachments,
                                        reasoning_embed,
                                        &mut last_edit_lock,
                                        &mut edit_count_lock,
                                    )
                                    .await;

                                    if did_edit.unwrap_or(false) {
                                        let mut last_content = last_sent_content.lock().await;
                                        *last_content = content;
                                    }
                                }
                            }
                        }

                        if event.event_type == "chat_stream" {
                            let payload: ChatResponse = flow_like_types::json::from_value(
                                event.payload.clone(),
                            )
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to deserialize chat_out payload: {}", e)
                            })?;

                            // Collect final attachments
                            if !payload.attachments.is_empty() {
                                let mut attachments = collected_attachments.lock().await;
                                attachments.extend(payload.attachments.clone());
                            }

                            // Final update (force immediate update) - no embed on final message
                            let last_message = payload.response.last_message();
                            if let Some(last_message) = last_message
                                && let Some(content) = &last_message.content
                            {
                                let mut resp_lock = response.lock().await;
                                let mut last_edit_lock = last_edit.lock().await;
                                let mut edit_count_lock = edit_count.lock().await;
                                *last_edit_lock = Instant::now() - Duration::from_secs(20); // Force update
                                let attachments = collected_attachments.lock().await;

                                let _ = update_discord_message(
                                    &ctx,
                                    &message,
                                    &mut resp_lock,
                                    content.clone(),
                                    &attachments,
                                    None, // No embed on final message
                                    &mut last_edit_lock,
                                    &mut edit_count_lock,
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
            tracing::error!("Failed to fire Discord event: {}", e);
            return Err(e);
        }

        // Final flush: ensure last message is sent even if within rate limit window
        let context_lock = context_final.lock().await;
        let last_message = context_lock.last_message();
        if let Some(last_message) = last_message
            && let Some(content) = &last_message.content
        {
            let mut resp_lock = response_final.lock().await;
            let mut last_edit_lock = last_edit_final.lock().await;
            let mut edit_count_lock = edit_count_final.lock().await;
            let attachments = collected_attachments_final.lock().await;

            // Force final update by resetting the last edit time
            *last_edit_lock = Instant::now() - Duration::from_secs(20);

            let _ = update_discord_message(
                &ctx_final,
                &message_final,
                &mut resp_lock,
                content.clone(),
                &attachments,
                None, // No embed on final flush
                &mut last_edit_lock,
                &mut edit_count_lock,
            )
            .await;
        }
    } else {
        return Err(anyhow::anyhow!("EventSinkManager state not available"));
    }

    Ok(())
}

async fn run_discord_bot(
    app_handle: AppHandle,
    db: DbConnection,
    token: String,
    intents: serenity::all::GatewayIntents,
    bot_instance: Arc<flow_like_types::tokio::sync::Mutex<BotInstance>>,
) -> Result<()> {
    let handler = DiscordEventHandler {
        app_handle,
        db,
        bot_instance,
    };

    let mut client = serenity::Client::builder(&token, intents)
        .event_handler(handler)
        .await?;

    tracing::info!("Starting Discord client...");
    client.start().await?;

    Ok(())
}

impl DiscordSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS discord_bots (
                token TEXT PRIMARY KEY,
                bot_name TEXT,
                bot_description TEXT,
                intents TEXT,
                connected INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS discord_handlers (
                event_id TEXT PRIMARY KEY,
                bot_token TEXT NOT NULL,
                channel_whitelist TEXT,
                channel_blacklist TEXT,
                respond_to_mentions INTEGER NOT NULL DEFAULT 1,
                respond_to_dms INTEGER NOT NULL DEFAULT 1,
                command_prefix TEXT NOT NULL DEFAULT '!',
                created_at INTEGER NOT NULL,
                FOREIGN KEY(bot_token) REFERENCES discord_bots(token) ON DELETE CASCADE
            )",
            [],
        )?;

        Ok(())
    }

    fn add_bot_and_handler(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &DiscordSink,
    ) -> Result<()> {
        let conn = db.lock().unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let intents_json = config
            .intents
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let channel_whitelist_json = config
            .channel_whitelist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let channel_blacklist_json = config
            .channel_blacklist
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        conn.execute(
            "INSERT OR REPLACE INTO discord_bots (token, bot_name, bot_description, intents, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                config.token,
                config.bot_name,
                config.bot_description,
                intents_json,
                now
            ],
        )?;

        conn.execute(
            "INSERT OR REPLACE INTO discord_handlers
             (event_id, bot_token, channel_whitelist, channel_blacklist, respond_to_mentions, respond_to_dms, command_prefix, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                registration.event_id,
                config.token,
                channel_whitelist_json,
                channel_blacklist_json,
                config.respond_to_mentions as i32,
                config.respond_to_dms as i32,
                config.command_prefix,
                now,
            ],
        )?;

        Ok(())
    }

    fn remove_handler(db: &DbConnection, event_id: &str) -> Result<String> {
        let conn = db.lock().unwrap();

        // Get the token before deleting
        let token: String = conn.query_row(
            "SELECT bot_token FROM discord_handlers WHERE event_id = ?1",
            params![event_id],
            |row| row.get(0),
        )?;

        conn.execute(
            "DELETE FROM discord_handlers WHERE event_id = ?1",
            params![event_id],
        )?;

        Ok(token)
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_discord_config(
        _event_id: String,
        token: String,
        bot_name: Option<String>,
        bot_description: Option<String>,
        intents_json: Option<String>,
        channel_whitelist_json: Option<String>,
        channel_blacklist_json: Option<String>,
        respond_to_mentions: bool,
        respond_to_dms: bool,
        command_prefix: String,
    ) -> DiscordSink {
        let intents = intents_json.and_then(|json| serde_json::from_str(&json).ok());
        let channel_whitelist =
            channel_whitelist_json.and_then(|json| serde_json::from_str(&json).ok());
        let channel_blacklist =
            channel_blacklist_json.and_then(|json| serde_json::from_str(&json).ok());

        DiscordSink {
            token,
            bot_name,
            bot_description,
            intents,
            channel_whitelist,
            channel_blacklist,
            respond_to_mentions,
            respond_to_dms,
            command_prefix,
        }
    }

    fn create_event_registration(event_id: String, config: DiscordSink) -> EventRegistration {
        EventRegistration {
            event_id: event_id.clone(),
            name: format!("Discord Handler {}", event_id),
            r#type: "discord_bot".to_string(),
            updated_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            config: super::EventConfig::Discord(config.clone()),
            offline: false,
            app_id: "unknown".to_string(),
            default_payload: None,
            personal_access_token: None,
            oauth_tokens: std::collections::HashMap::new(),
        }
    }

    async fn load_handlers_from_db(
        db: &DbConnection,
    ) -> Result<Vec<(EventRegistration, DiscordSink)>> {
        let conn = db.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT h.event_id, h.bot_token, b.bot_name, b.bot_description, b.intents,
                    h.channel_whitelist, h.channel_blacklist, h.respond_to_mentions,
                    h.respond_to_dms, h.command_prefix
             FROM discord_handlers h
             JOIN discord_bots b ON h.bot_token = b.token",
        )?;

        let results = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i32>(7)? != 0,
                row.get::<_, i32>(8)? != 0,
                row.get::<_, String>(9)?,
            ))
        })?;

        let mut handlers = Vec::new();

        for result in results {
            let (
                event_id,
                token,
                bot_name,
                bot_description,
                intents_json,
                channel_whitelist_json,
                channel_blacklist_json,
                respond_to_mentions,
                respond_to_dms,
                command_prefix,
            ) = result?;

            let config = Self::parse_discord_config(
                event_id.clone(),
                token,
                bot_name,
                bot_description,
                intents_json,
                channel_whitelist_json,
                channel_blacklist_json,
                respond_to_mentions,
                respond_to_dms,
                command_prefix,
            );

            let registration = Self::create_event_registration(event_id, config.clone());
            handlers.push((registration, config));
        }

        Ok(handlers)
    }
}

#[async_trait::async_trait]
impl EventSink for DiscordSink {
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        tracing::info!("🤖 Starting Discord sink - initializing bot manager...");

        // Load all existing handlers from database and register them
        let handlers = Self::load_handlers_from_db(&db).await?;

        if !handlers.is_empty() {
            tracing::info!(
                "📋 Found {} Discord handlers in database, registering...",
                handlers.len()
            );

            let mut manager = DISCORD_MANAGER.lock().await;
            for (registration, config) in handlers {
                if let Err(e) = manager
                    .add_or_update_bot(app_handle, &db, &registration, &config)
                    .await
                {
                    tracing::error!(
                        "Failed to initialize Discord bot for event {}: {}",
                        registration.event_id,
                        e
                    );
                }
            }
        }

        tracing::info!("✅ Discord sink started - bot manager ready");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::info!("Stopping Discord sink...");

        let mut manager = DISCORD_MANAGER.lock().await;
        let tokens: Vec<String> = manager.bots.keys().cloned().collect();

        for token in tokens {
            if let Err(e) = manager.stop_bot(&token).await {
                tracing::error!("Failed to stop Discord bot: {}", e);
            }
        }

        tracing::info!("Discord sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::add_bot_and_handler(&db, registration, self)?;

        let mut manager = DISCORD_MANAGER.lock().await;
        manager
            .add_or_update_bot(app_handle, &db, registration, self)
            .await?;

        tracing::info!(
            "✅ Registered Discord bot: {} -> event {}",
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

        let mut manager = DISCORD_MANAGER.lock().await;
        manager
            .remove_handler(&token, &registration.event_id)
            .await?;

        tracing::info!("Unregistered Discord handler: {}", registration.event_id);

        Ok(())
    }
}
