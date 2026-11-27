//! Microsoft Teams for bots: a Teams connection bound to one of the owner's bots instead of an app
//! event. Messages addressed to the bot go into the chat's guest room
//! ([`crate::bot_channels::post_outside_message`]); the bot's reply comes back through
//! [`deliver_reply`].

use super::{
    Connection, auth,
    context::{self, clean_label},
    files::{self, FileEntry, FileOwner},
    management::{
        self, Target, TeamsAccessView, TeamsConnectionView, TeamsPackage, TeamsSetupRequest,
    },
    now,
    runtime::{
        self, DAY_MS, Dispatch, Gate, MAX_MESSAGE_BYTES, PREPARE_BUDGET, Session, activity_url,
        claim_dispatch, deliver, dispatch_claim, record_activity, release_dispatch, string,
    },
    store,
};
use crate::{
    bot_channels::{
        ChannelProvider, ChannelReply, OutsideAttachment, OutsideMessage, OutsideSender,
        PostStatus, ensure_channel_bot, post_outside_message, trigger_message_id,
    },
    entity::{bot, bot_channel_link},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::{owned_bot, workspace},
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use flow_like_types::tokio::{self, time::Instant};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use std::time::Duration;

const REPLY_PREFIX: &str = "botreply:";
const REPLY_TTL_MS: i64 = 2 * DAY_MS;
const REPLY_OVERHEAD_BYTES: usize = 200;
const MAX_REPLY_PARTS: usize = 8;
const SIDE_MESSAGE_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_TITLE_CHARS: usize = 100;
const MAX_NAME_CHARS: usize = 80;
const FALLBACK_TITLE: &str = "Teams chat";
const BARE_MENTION_TEXT: &str = "(The message only mentioned you.)";
const TRUNCATION_NOTE: &str = "\n\n(The reply was shortened.)";

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{bot_id}/teams",
            get(get_bot_teams)
                .put(setup_bot_teams)
                .delete(disconnect_bot_teams),
        )
        .route("/{bot_id}/teams/package", get(bot_teams_package))
        .route("/{bot_id}/teams/rotate", post(rotate_bot_teams))
        .route("/{bot_id}/teams/access", get(bot_teams_access))
}

async fn existing_target(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
) -> Result<Target, ApiError> {
    let owner = user.sub()?;
    let bot = owned_bot(&state.db, &owner, bot_id).await?;
    ensure_channel_bot(&bot)?;
    let app = workspace::live_workspace(&state.db, &owner, bot_id)
        .await?
        .unwrap_or_default();
    Ok(Target::bot(&app, bot_id))
}

async fn workspace_target(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
) -> Result<Target, ApiError> {
    let owner = user.sub()?;
    let bot = owned_bot(&state.db, &owner, bot_id).await?;
    ensure_channel_bot(&bot)?;
    let app = workspace::ensure(state, &owner, bot_id).await?.app_id;
    Ok(Target::bot(&app, bot_id))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/teams",
    operation_id = "get_bot_teams",
    tag = "Bots",
    description = "Show how your bot is connected to Microsoft Teams: its status, messaging endpoint and last verified activity.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's Teams connection", body = TeamsConnectionView),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_CHANNEL_NOT_ALLOWED: this bot can't join Teams chats")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_bot_teams(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    let target = existing_target(&state, &user, &bot_id).await?;
    management::get_inner(&state, &target).await.map(Json)
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/teams",
    operation_id = "setup_bot_teams",
    tag = "Bots",
    description = "Connect or update your bot's Microsoft Teams bot. New credentials are verified with Microsoft before they replace the saved ones. People in Teams who message the bot get their own guest room in your bot list.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = TeamsSetupRequest,
    responses(
        (status = 200, description = "The bot is connected", body = TeamsConnectionView),
        (status = 400, description = "Invalid settings, or Microsoft rejected the credentials"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Teams bots are disabled on this server"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "Another Teams operation is running, the bot changed meanwhile, or BOT_CHANNEL_NOT_ALLOWED"),
        (status = 429, description = "Too many Flow-Like-managed setups for this bot"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []))
)]
pub async fn setup_bot_teams(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(input): Json<TeamsSetupRequest>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    let target = workspace_target(&state, &user, &bot_id).await?;
    management::setup_view(&state, &target, input)
        .await
        .map(Json)
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/teams",
    operation_id = "disconnect_bot_teams",
    tag = "Bots",
    description = "Disconnect your bot from Microsoft Teams. A Flow-Like-managed bot's Microsoft registration is deleted; a customer-managed registration stays in your account.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot is disconnected", body = TeamsConnectionView),
        (status = 400, description = "Microsoft refused to remove the registration"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no Teams connection"),
        (status = 409, description = "Another Teams operation is running"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []))
)]
pub async fn disconnect_bot_teams(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    let target = existing_target(&state, &user, &bot_id).await?;
    management::disconnect_view(&state, &target).await.map(Json)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/teams/package",
    operation_id = "download_bot_teams_package",
    tag = "Bots",
    description = "Download the Teams app package (manifest and icons) to upload in Microsoft Teams so people can add your bot. It contains no credentials.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The Teams app package", body = TeamsPackage),
        (status = 400, description = "Bot setup is not complete"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no Teams connection")
    ),
    security(("bearer_auth" = []))
)]
pub async fn bot_teams_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TeamsPackage>, ApiError> {
    let target = existing_target(&state, &user, &bot_id).await?;
    management::package_inner(&state, &target).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/teams/rotate",
    operation_id = "rotate_bot_teams_secret",
    tag = "Bots",
    description = "Renew the client secret of your bot's Flow-Like-managed Teams bot. The current secret keeps working until the new one is verified.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The secret was renewed", body = TeamsConnectionView),
        (status = 400, description = "The Teams bot is customer-managed or not set up"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it has no Teams connection"),
        (status = 409, description = "Another Teams operation is running"),
        (status = 502, description = "Microsoft could not be reached")
    ),
    security(("bearer_auth" = []))
)]
pub async fn rotate_bot_teams(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TeamsConnectionView>, ApiError> {
    let target = existing_target(&state, &user, &bot_id).await?;
    management::rotate_view(&state, &target).await.map(Json)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/teams/access",
    operation_id = "check_bot_teams_access",
    tag = "Bots",
    description = "Check whether Microsoft lets your bot's Teams bot read what its permissions request. When an administrator has to approve it first, a Flow-Like-managed bot gets an admin-consent link.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's Microsoft Graph access", body = TeamsAccessView),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
pub async fn bot_teams_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TeamsAccessView>, ApiError> {
    let target = existing_target(&state, &user, &bot_id).await?;
    management::access_inner(&state, &target).await.map(Json)
}

/// Disconnects and removes a deleted bot's Teams connection. Never fails the caller.
pub(crate) async fn cleanup_bot(state: &AppState, bot_id: &str) {
    if let Err(error) = management::cleanup_bot_connection(state, bot_id).await {
        tracing::warn!(bot_id, %error, "Could not remove the Teams connection of a deleted bot");
    }
}

/// The activity as an outside message. `tenant` is the connection's verified customer tenant.
fn outside_message(
    tenant: &str,
    connection_id: &str,
    activity: &Value,
    attachments: Vec<OutsideAttachment>,
) -> Result<OutsideMessage, ApiError> {
    let conversation = string(activity, "/conversation/id")?;
    let sender = string(activity, "/from/aadObjectId").or_else(|_| string(activity, "/from/id"))?;
    let scope = context::scope(activity);
    let mut text = context::clean_text(activity);
    if text.is_empty() && attachments.is_empty() {
        text = BARE_MENTION_TEXT.into();
    }
    Ok(OutsideMessage {
        provider: ChannelProvider::Teams,
        chat_key: format!("{tenant}:{conversation}"),
        chat_title: chat_title(activity, &scope.conversation_type),
        message_id: string(activity, "/id")?.into(),
        sender: OutsideSender {
            id: format!("{tenant}:{sender}"),
            name: context::display_name(context::field(activity, "/from/name"), false)
                .chars()
                .take(MAX_NAME_CHARS)
                .collect(),
        },
        text,
        attachments,
        meta: json!({
            "connection_id": connection_id,
            "conversation_type": scope.conversation_type,
        }),
    })
}

fn chat_title(activity: &Value, conversation_type: &str) -> String {
    let name = |pointer: &str| {
        context::field(activity, pointer)
            .map(|raw| clean_label(raw, MAX_TITLE_CHARS))
            .filter(|name| !name.is_empty())
    };
    let title = match conversation_type {
        "channel" => match (
            name("/channelData/team/name"),
            name("/channelData/channel/name"),
        ) {
            (Some(team), Some(channel)) => Some(format!("{team} / {channel}")),
            (team, channel) => channel.or(team),
        },
        "personal" => name("/from/name"),
        _ => name("/conversation/name"),
    };
    title
        .map(|title| clean_label(&title, MAX_TITLE_CHARS))
        .unwrap_or_else(|| FALLBACK_TITLE.into())
}

fn link_attachments(files: &[FileEntry]) -> Vec<OutsideAttachment> {
    files
        .iter()
        .filter_map(|file| {
            let url = file
                .url
                .as_ref()
                .filter(|url| url.starts_with("https://"))?;
            Some(OutsideAttachment {
                name: file.name.clone(),
                url: url.clone(),
                content_type: Some(file.mime.clone()).filter(|mime| !mime.is_empty()),
            })
        })
        .take(crate::bot_channels::MAX_ATTACHMENTS)
        .collect()
}

fn reply_session(
    c: &Connection,
    activity: &Value,
    message: &OutsideMessage,
    trigger: &str,
) -> Result<Session, ApiError> {
    let scope = context::scope(activity);
    Ok(Session {
        connection_id: c.id.clone(),
        app_id: c.app_id.clone(),
        event_id: c.event_id.clone(),
        run_id: trigger.into(),
        tenant_id: c.customer_tenant_id.clone(),
        client_id: c.client_id.clone(),
        conversation_id: string(activity, "/conversation/id")?.into(),
        service_url: string(activity, "/serviceUrl")?.into(),
        activity_id: string(activity, "/id")?.into(),
        user_id: message.sender.id.clone(),
        bot_id: string(activity, "/recipient/id")?.into(),
        history_key: String::new(),
        conversation_type: scope.conversation_type,
        team_id: scope.team_id,
        channel_id: scope.channel_id,
        thread_id: scope.thread_id,
        meeting_id: scope.meeting_id,
        permissions: vec![],
    })
}

/// Handles a verified activity of a bot connection. Answers fast: the turn is never awaited.
pub(super) async fn incoming(
    state: &AppState,
    c: &Connection,
    activity: &Value,
) -> Result<Json<Value>, ApiError> {
    match runtime::gate(activity, false)? {
        Gate::Dispatch => {}
        Gate::ForgetTeam(team) => {
            if let Err(error) = super::microsoft::forget_team(state, c, &team).await {
                tracing::warn!(connection_id = %c.id, ?error, "Could not drop cached Teams team details");
            }
            return Ok(Json(json!({})));
        }
        _ => return Ok(Json(json!({}))),
    }
    let conversation = string(activity, "/conversation/id")?;
    let activity_id = string(activity, "/id")?;
    let run = super::hash(&[
        &c.id,
        &c.client_id,
        &c.customer_tenant_id,
        conversation,
        activity_id,
    ]);
    let claimed = store::get::<Dispatch>(state, &format!("dispatch:{run}")).await?;
    if claimed.is_some_and(|(claim, _)| dispatch_claim(&claim, now()) == runtime::Claim::Done) {
        return Ok(Json(json!({})));
    }
    record_activity(state, c).await;
    let Some(lease) = claim_dispatch(state, c, &run).await? else {
        return Ok(Json(json!({})));
    };
    let result = accept(state, c, activity, &run).await;
    release_dispatch(state, &run, lease, result.is_ok()).await;
    result.map(|()| Json(json!({})))
}

async fn live_bot(state: &AppState, c: &Connection) -> Result<bot::Model, ApiError> {
    if c.status != "ready" {
        return Err(ApiError::forbidden("This Teams bot is not connected"));
    }
    let bot_id = c.bot_id.as_deref().ok_or(ApiError::FORBIDDEN)?;
    let bot = bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    ensure_channel_bot(&bot)?;
    let workspace = workspace::live_workspace(&state.db, &bot.owner_id, &bot.id).await?;
    if workspace.as_deref() != Some(c.app_id.as_str()) {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(bot)
}

async fn accept(
    state: &AppState,
    c: &Connection,
    activity: &Value,
    run: &str,
) -> Result<(), ApiError> {
    let bot = live_bot(state, c).await?;
    let deadline = Instant::now() + PREPARE_BUDGET;
    let fresh = auth::fresh_connection(state, c).await.unwrap_or_else(|error| {
        tracing::warn!(connection_id = %c.id, %error, "Could not renew the Teams bot credentials before preparing a message");
        c.clone()
    });
    let files = files::collect(
        state,
        &fresh,
        FileOwner::Subject(&bot.owner_id),
        run,
        files::classify(activity),
        deadline,
    )
    .await;
    let message = outside_message(
        &c.customer_tenant_id,
        &c.id,
        activity,
        link_attachments(&files),
    )?;
    let trigger = trigger_message_id(
        &bot.id,
        ChannelProvider::Teams,
        &message.chat_key,
        &message.message_id,
    );
    let session = reply_session(c, activity, &message, &trigger)?;
    store::put(
        state,
        &format!("{REPLY_PREFIX}{trigger}"),
        &c.id,
        &session,
        now() + REPLY_TTL_MS,
    )
    .await?;
    let posted = post_outside_message(state, &bot, &message).await?;
    match (posted.status, posted.notice) {
        (PostStatus::Limited, Some(notice)) => {
            let sent = tokio::time::timeout(
                SIDE_MESSAGE_TIMEOUT,
                deliver(
                    state,
                    &fresh,
                    &session,
                    "notice",
                    message_body(&notice),
                    None,
                ),
            )
            .await;
            if !matches!(sent, Ok(Ok(_))) {
                tracing::warn!(connection_id = %c.id, bot_id = %bot.id, "Could not tell a Teams chat that its message was limited");
            }
        }
        (PostStatus::Queued, _) => {
            let typing =
                tokio::time::timeout(SIDE_MESSAGE_TIMEOUT, send_typing(state, &fresh, &session))
                    .await;
            if typing.is_err() {
                tracing::debug!(connection_id = %c.id, "Teams typing indicator timed out");
            }
        }
        _ => {}
    }
    Ok(())
}

fn message_body(text: &str) -> Value {
    json!({"type":"message","textFormat":"markdown","text":text})
}

async fn send_typing(state: &AppState, c: &Connection, session: &Session) {
    let sent = async {
        let url = activity_url(session, None, false)?;
        let token = auth::bot_token(state, c).await?;
        let response = super::client()?
            .post(url)
            .bearer_auth(token)
            .timeout(SIDE_MESSAGE_TIMEOUT)
            .json(&json!({"type":"typing"}))
            .send()
            .await
            .map_err(|_| ApiError::bad_gateway("Teams could not be reached"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ApiError::bad_gateway(format!(
                "Teams refused the typing indicator (HTTP {})",
                response.status()
            )))
        }
    }
    .await;
    if let Err(error) = sent {
        tracing::debug!(connection_id = %c.id, %error, "Could not send the Teams typing indicator");
    }
}

/// Sends the bot's reply into the Teams chat the message came from, as replies to that message.
pub(crate) async fn deliver_reply(
    state: &AppState,
    link: &bot_channel_link::Model,
    reply: &ChannelReply<'_>,
) -> Result<(), ApiError> {
    let text = reply.text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let (session, _) = store::get::<Session>(
        state,
        &format!("{REPLY_PREFIX}{}", reply.trigger_message_id),
    )
    .await?
    .ok_or_else(|| {
        ApiError::not_found(format!(
            "The Teams address of message '{}' expired or was never stored",
            reply.trigger_message_id
        ))
    })?;
    let (c, _) = store::connection(state, &session.connection_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Teams connection '{}' of bot '{}' no longer exists",
                session.connection_id, link.bot_id
            ))
        })?;
    if c.bot_id.as_deref() != Some(link.bot_id.as_str()) || c.status != "ready" {
        return Err(ApiError::forbidden(format!(
            "Teams connection '{}' is not an active connection of bot '{}'",
            c.id, link.bot_id
        )));
    }
    let c = auth::fresh_connection(state, &c).await.unwrap_or(c);
    for (index, part) in split_reply(text, MAX_MESSAGE_BYTES - REPLY_OVERHEAD_BYTES)
        .iter()
        .enumerate()
    {
        let key = format!("{}:{index}", reply.message_id);
        deliver(state, &c, &session, &key, message_body(part), None).await?;
    }
    Ok(())
}

/// Length of `text` inside a JSON string, escapes included.
fn encoded_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(text.len(), |encoded| encoded.len() - 2)
}

/// Splits on paragraph, then line, then word boundaries so each part's JSON-encoded text stays
/// within `max_bytes`. Parts beyond [`MAX_REPLY_PARTS`] are dropped with a note.
fn split_reply(text: &str, max_bytes: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if encoded_len(rest) <= max_bytes {
            parts.push(rest.to_owned());
            break;
        }
        let head = fitting_prefix(rest, max_bytes);
        let cut = if rest[head.len()..].starts_with(char::is_whitespace) {
            head.len()
        } else {
            ["\n\n", "\n", " "]
                .iter()
                .find_map(|separator| head.rfind(separator).filter(|at| *at > 0))
                .unwrap_or(head.len())
        };
        let part = rest[..cut].trim_end();
        if !part.is_empty() {
            parts.push(part.to_owned());
        }
        rest = rest[cut..].trim_start();
    }
    if parts.len() > MAX_REPLY_PARTS {
        parts.truncate(MAX_REPLY_PARTS);
        if let Some(last) = parts.last_mut() {
            *last = fit_with_note(last, max_bytes);
        }
    }
    parts
}

fn fitting_prefix(text: &str, max_bytes: usize) -> &str {
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        let mut buffer = [0u8; 4];
        used += encoded_len(ch.encode_utf8(&mut buffer));
        if used > max_bytes {
            return &text[..index];
        }
    }
    text
}

fn fit_with_note(part: &str, max_bytes: usize) -> String {
    let room = max_bytes.saturating_sub(encoded_len(TRUNCATION_NOTE));
    format!("{}{TRUNCATION_NOTE}", fitting_prefix(part, room).trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: &str = "32345678-1234-1234-1234-123456789012";

    fn personal() -> Value {
        json!({
            "type":"message","id":"1727000000001","text":"What is on today?",
            "serviceUrl":"https://smba.trafficmanager.net/amer/",
            "from":{"id":"29:abc","name":"Ada Lovelace","aadObjectId":"42345678-1234-1234-1234-123456789012"},
            "recipient":{"id":"28:bot","name":"Helper"},
            "conversation":{"id":"a:1conv","conversationType":"personal","tenantId":TENANT},
            "channelData":{"tenant":{"id":TENANT}}
        })
    }

    fn channel() -> Value {
        json!({
            "type":"message","id":"1727000000002",
            "text":"<at>Helper</at> summarize this thread",
            "serviceUrl":"https://smba.trafficmanager.net/amer/",
            "from":{"id":"29:def","name":"Grace Hopper","aadObjectId":"52345678-1234-1234-1234-123456789012"},
            "recipient":{"id":"28:bot","name":"Helper"},
            "entities":[{"type":"mention","text":"<at>Helper</at>","mentioned":{"id":"28:bot","name":"Helper"}}],
            "conversation":{"id":"19:chan@thread.tacv2;messageid=1727000000000","conversationType":"channel"},
            "channelData":{"team":{"id":"19:team@thread.tacv2","name":"Platform"},"channel":{"id":"19:chan@thread.tacv2","name":"Ops"}}
        })
    }

    #[test]
    fn a_personal_chat_maps_to_a_room_named_after_the_person() {
        let message = outside_message(TENANT, "conn", &personal(), vec![]).unwrap();
        assert_eq!(message.provider, ChannelProvider::Teams);
        assert_eq!(message.chat_key, format!("{TENANT}:a:1conv"));
        assert_eq!(message.chat_title, "Ada Lovelace");
        assert_eq!(message.message_id, "1727000000001");
        assert_eq!(
            message.sender.id,
            format!("{TENANT}:42345678-1234-1234-1234-123456789012")
        );
        assert_eq!(message.sender.name, "Ada Lovelace");
        assert_eq!(message.text, "What is on today?");
        assert_eq!(
            message.meta,
            json!({"connection_id":"conn","conversation_type":"personal"})
        );
    }

    #[test]
    fn a_channel_thread_keeps_its_conversation_id_and_drops_the_mention() {
        let message = outside_message(TENANT, "conn", &channel(), vec![]).unwrap();
        assert_eq!(
            message.chat_key,
            format!("{TENANT}:19:chan@thread.tacv2;messageid=1727000000000")
        );
        assert_eq!(message.chat_title, "Platform / Ops");
        assert_eq!(message.text, "summarize this thread");
    }

    #[test]
    fn anonymous_guests_are_identified_by_their_teams_id() {
        let mut activity = channel();
        activity["from"] = json!({"id":"29:guest","name":"Guest"});
        let message = outside_message(TENANT, "conn", &activity, vec![]).unwrap();
        assert_eq!(message.sender.id, format!("{TENANT}:29:guest"));
    }

    #[test]
    fn titles_fall_back_to_the_chat_name_then_a_generic_label() {
        let group = json!({"conversation":{"name":"Launch crew"}});
        assert_eq!(chat_title(&group, "groupChat"), "Launch crew");
        assert_eq!(chat_title(&json!({}), "groupChat"), FALLBACK_TITLE);
        assert_eq!(chat_title(&json!({}), "channel"), FALLBACK_TITLE);
        let team_only = json!({"channelData":{"team":{"name":"Platform"}}});
        assert_eq!(chat_title(&team_only, "channel"), "Platform");
    }

    #[test]
    fn a_bare_mention_still_carries_text_and_files_replace_it() {
        let mut activity = channel();
        activity["text"] = json!("<at>Helper</at>");
        let bare = outside_message(TENANT, "c", &activity, vec![]).unwrap();
        assert_eq!(bare.text, BARE_MENTION_TEXT);
        let attached = vec![OutsideAttachment {
            name: "a.png".into(),
            url: "https://files.example/a.png".into(),
            content_type: None,
        }];
        assert_eq!(
            outside_message(TENANT, "c", &activity, attached)
                .unwrap()
                .text,
            ""
        );
    }

    #[test]
    fn a_missing_conversation_or_sender_is_a_bad_request() {
        let mut activity = personal();
        activity["conversation"] = json!({});
        assert!(outside_message(TENANT, "c", &activity, vec![]).is_err());
        let mut activity = personal();
        activity["from"] = json!({});
        assert!(outside_message(TENANT, "c", &activity, vec![]).is_err());
    }

    #[test]
    fn only_https_downloads_become_attachments() {
        let file = |url: Option<&str>| FileEntry {
            name: "f".into(),
            mime: "image/png".into(),
            url: url.map(Into::into),
            ..FileEntry::default()
        };
        let links = link_attachments(&[
            file(Some("https://x.example/a")),
            file(Some("http://x.example/b")),
            file(None),
        ]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://x.example/a");
        assert_eq!(links[0].content_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn short_replies_are_one_part() {
        assert_eq!(
            split_reply("  Hello\n\nthere ", 100),
            vec!["Hello\n\nthere"]
        );
        assert!(split_reply("   ", 100).is_empty());
    }

    #[test]
    fn long_replies_split_on_paragraphs_first() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(40),
            "b".repeat(40),
            "c".repeat(40)
        );
        let parts = split_reply(&text, 90);
        assert_eq!(
            parts,
            vec![
                format!("{}\n\n{}", "a".repeat(40), "b".repeat(40)),
                "c".repeat(40)
            ]
        );
    }

    #[test]
    fn long_paragraphs_split_on_lines_then_words_then_anywhere() {
        let lines = "line one\nline two\nline three";
        assert_eq!(
            split_reply(lines, 18),
            vec!["line one\nline two", "line three"]
        );
        assert_eq!(
            split_reply("alpha beta gamma", 11),
            vec!["alpha beta", "gamma"]
        );
        assert_eq!(split_reply("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn parts_respect_the_json_encoded_size_and_lose_no_text() {
        let text = "\"quoted\"\n".repeat(500);
        let parts = split_reply(&text, 1_000);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| encoded_len(part) <= 1_000));
        let joined = parts.join("\n");
        assert_eq!(joined.replace('\n', ""), text.replace('\n', "").trim());
        let multibyte = "ü".repeat(50);
        assert!(split_reply(&multibyte, 25).iter().all(|p| p.len() <= 25));
    }

    #[test]
    fn too_many_parts_are_cut_with_a_note() {
        let text = "word ".repeat(400);
        let parts = split_reply(&text, 50);
        assert_eq!(parts.len(), MAX_REPLY_PARTS);
        assert!(
            parts
                .last()
                .unwrap()
                .ends_with("(The reply was shortened.)")
        );
        assert!(parts.iter().all(|part| encoded_len(part) <= 50));
    }
}
