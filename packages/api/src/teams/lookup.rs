use super::{
    Connection, TeamsMemberInfo, TeamsPermission,
    microsoft::{self, MicrosoftError, base_conversation, graph_path},
    runtime::{Conversation, Session, session_for_run},
    store,
};
use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_types::tokio;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

const MAX_MESSAGES: i64 = 50;
const MAX_MEMBERS: i64 = 500;
const MIN_MEMBER_PAGE: usize = 50;
const MAX_TEXT_BYTES: usize = 8000;
const MAX_HTML_BYTES: usize = 24_000;
const MAX_CURSOR_BYTES: usize = 8192;

fn default_messages() -> i64 {
    20
}

fn default_members() -> i64 {
    100
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamsMessagesScope {
    /// The channel thread that started the run.
    #[default]
    Thread,
    /// The latest posts of the whole channel.
    Conversation,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamsMessagesRequest {
    /// Run ID from the Teams event's session.
    session_id: String,
    /// Channels only. Chats always return their latest messages.
    #[serde(default)]
    scope: TeamsMessagesScope,
    /// Number of messages, 1–50.
    #[serde(default = "default_messages")]
    #[schema(minimum = 1, maximum = 50, default = 20)]
    limit: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamsMessagesSource {
    /// Read from Microsoft Graph with the bot's resource-specific consent.
    Graph,
    /// The conversation history Flow-Like keeps for a personal chat.
    BotHistory,
}

/// A file shared in a message. Only its link is available, not its content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct TeamsFileLink {
    name: Option<String>,
    /// Attachment content type, such as `reference` for a shared file.
    #[serde(rename = "type")]
    content_type: Option<String>,
    /// Link that opens the file in Teams, SharePoint or OneDrive.
    link: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct TeamsHistoryMessage {
    /// Teams message ID. Missing for history recorded before IDs were stored.
    id: Option<String>,
    /// Microsoft Entra object ID of a user, or the application ID of a bot.
    author_id: Option<String>,
    /// Microsoft Entra object ID of a user from an organization.
    author_aad_object_id: Option<String>,
    author_name: Option<String>,
    /// Whether a bot or app sent the message.
    is_bot: bool,
    /// Plain text, at most 8000 bytes.
    text: String,
    /// The HTML body, when Microsoft sent one of at most 24000 bytes.
    html: Option<String>,
    /// When the message was sent (ISO 8601).
    created_at: Option<String>,
    /// ID of the thread's root message, for channel replies.
    reply_to_id: Option<String>,
    attachments: Vec<TeamsFileLink>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamsMessagesResponse {
    source: TeamsMessagesSource,
    /// Oldest first.
    messages: Vec<TeamsHistoryMessage>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamsMembersRequest {
    /// Run ID from the Teams event's session.
    session_id: String,
    /// Members per page, 1–500.
    #[serde(default = "default_members")]
    #[schema(minimum = 1, maximum = 500, default = 100)]
    limit: i64,
    /// `continuation_token` of the previous page. Empty or missing: the first page.
    #[serde(default)]
    continuation_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamsMembersResponse {
    members: Vec<TeamsMemberInfo>,
    /// Pass it to the next request to read more members. `null` on the last page.
    continuation_token: Option<String>,
}

fn ensure_run_caller(user: &AppUser, app: &str, session_id: &str) -> Result<(), ApiError> {
    match user {
        AppUser::Executor(claims) if claims.app_id == app && claims.run_id == session_id => Ok(()),
        _ => Err(ApiError::FORBIDDEN),
    }
}

async fn running_session(
    state: &AppState,
    app: &str,
    run: &str,
) -> Result<(Session, Connection), ApiError> {
    let (session, c, record) = session_for_run(state, app, run)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if record.status.is_terminal() {
        return Err(ApiError::forbidden("This Teams execution has ended"));
    }
    Ok((session, c))
}

fn bounded(limit: i64, max: i64, what: &str) -> Result<usize, ApiError> {
    if !(1..=max).contains(&limit) {
        return Err(ApiError::bad_request(format!(
            "Request between 1 and {max} {what}, not {limit}"
        )));
    }
    Ok(limit as usize)
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/teams/messages",
    operation_id = "get_teams_messages",
    tag = "execution",
    description = "Read the latest messages of the Teams conversation that started this run, oldest first. Channels and group chats are read from Microsoft Graph and need the 'Read channel and chat messages' permission in the bot setup; personal chats return the history Flow-Like keeps. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = TeamsMessagesRequest,
    responses(
        (status = 200, description = "The messages, oldest first", body = TeamsMessagesResponse),
        (status = 400, description = "Invalid limit, or the conversation does not support message lookups"),
        (status = 403, description = "The caller is not this app's running Teams execution, or Microsoft did not grant message access"),
        (status = 404, description = "Microsoft could not find the team, channel, thread or chat"),
        (status = 409, description = "Message reading is turned off in the bot setup, or an administrator must consent first"),
        (status = 429, description = "Microsoft is throttling the bot"),
        (status = 502, description = "Microsoft could not be reached or answered unexpectedly")
    ),
    security(("executor_jwt" = []))
)]
pub async fn messages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app): Path<String>,
    Json(input): Json<TeamsMessagesRequest>,
) -> Result<Json<TeamsMessagesResponse>, ApiError> {
    ensure_run_caller(&user, &app, &input.session_id)?;
    let limit = bounded(input.limit, MAX_MESSAGES, "Teams messages")?;
    let (session, c) = running_session(&state, &app, &input.session_id).await?;
    let (source, messages) = match session.conversation_type.as_str() {
        "personal" => (
            TeamsMessagesSource::BotHistory,
            bot_history(&state, &session, &c, limit).await?,
        ),
        kind @ ("channel" | "groupChat") => {
            ensure_read_messages(&session)?;
            let messages = if kind == "channel" {
                channel_messages(&state, &session, &c, input.scope, limit).await
            } else {
                chat_messages(&state, &session, &c, limit).await
            };
            (
                TeamsMessagesSource::Graph,
                messages.map_err(message_access_error)?,
            )
        }
        "" => {
            return Err(ApiError::bad_request(
                "This Teams run started before message lookups were available. Send the bot a new message and retry.",
            ));
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "Teams conversations of type '{other}' do not support message lookups"
            )));
        }
    };
    Ok(Json(TeamsMessagesResponse { source, messages }))
}

fn ensure_read_messages(session: &Session) -> Result<(), ApiError> {
    if session.permissions.contains(&TeamsPermission::ReadMessages) {
        return Ok(());
    }
    Err(ApiError::conflict(
        "Turn on 'Read channel and chat messages' in the Teams bot setup, then update the app in Teams.",
    ))
}

fn message_access_error(error: MicrosoftError) -> ApiError {
    match error {
        MicrosoftError::Forbidden => ApiError::forbidden(
            "Microsoft did not grant message access for this team or chat. A team owner must approve the app's permissions (reinstall the updated app).",
        ),
        other => other.into_api("Teams messages"),
    }
}

async fn bot_history(
    state: &AppState,
    session: &Session,
    c: &Connection,
    limit: usize,
) -> Result<Vec<TeamsHistoryMessage>, ApiError> {
    let conversation = store::get::<Conversation>(state, &session.history_key)
        .await?
        .map(|(conversation, _)| conversation)
        .unwrap_or_default();
    Ok(keep_last(
        stored_messages(&conversation.messages, &c.name),
        limit,
    ))
}

/// Group and meeting chats: the latest messages, reversed to oldest first.
async fn chat_messages(
    state: &AppState,
    session: &Session,
    c: &Connection,
    limit: usize,
) -> Result<Vec<TeamsHistoryMessage>, MicrosoftError> {
    let path = format!(
        "{}?$top={limit}&$orderby=createdDateTime desc",
        graph_path(&[
            "chats",
            base_conversation(&session.conversation_id),
            "messages"
        ])
    );
    let page = microsoft::graph_get(state, c, &path).await?;
    Ok(keep_last(oldest_first(graph_list(&page)?), limit))
}

async fn channel_messages(
    state: &AppState,
    session: &Session,
    c: &Connection,
    scope: TeamsMessagesScope,
    limit: usize,
) -> Result<Vec<TeamsHistoryMessage>, MicrosoftError> {
    let group = team_group(state, session, c).await?;
    let channel = match session.channel_id.as_str() {
        "" => base_conversation(&session.conversation_id),
        channel => channel,
    };
    let posts = graph_path(&["teams", group.as_str(), "channels", channel, "messages"]);
    if scope == TeamsMessagesScope::Conversation || session.thread_id.is_empty() {
        let page = microsoft::graph_get(state, c, &format!("{posts}?$top={limit}")).await?;
        return Ok(keep_last(oldest_first(graph_list(&page)?), limit));
    }
    let root = format!("{posts}/{}", graph_path(&[session.thread_id.as_str()]));
    let replies = format!("{root}/replies?$top={limit}");
    let (root, replies) = tokio::try_join!(
        microsoft::graph_get(state, c, &root),
        microsoft::graph_get(state, c, &replies)
    )?;
    Ok(thread(history_message(&root), graph_list(&replies)?, limit))
}

async fn team_group(
    state: &AppState,
    session: &Session,
    c: &Connection,
) -> Result<String, MicrosoftError> {
    if session.team_id.is_empty() {
        return Err(MicrosoftError::Invalid(
            "Teams did not report the team of this channel.".into(),
        ));
    }
    microsoft::team(state, c, &session.service_url, &session.team_id)
        .await?
        .aad_group_id
        .ok_or_else(|| {
            MicrosoftError::Invalid(
                "Teams did not report the Microsoft 365 group of this team.".into(),
            )
        })
}

fn oldest_first(mut messages: Vec<TeamsHistoryMessage>) -> Vec<TeamsHistoryMessage> {
    messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    messages
}

fn keep_last<T>(mut items: Vec<T>, limit: usize) -> Vec<T> {
    let excess = items.len().saturating_sub(limit);
    items.drain(..excess);
    items
}

/// The root, then the latest replies oldest first, `limit` messages in total.
fn thread(
    root: Option<TeamsHistoryMessage>,
    replies: Vec<TeamsHistoryMessage>,
    limit: usize,
) -> Vec<TeamsHistoryMessage> {
    let replies = oldest_first(replies);
    let Some(root) = root else {
        return keep_last(replies, limit);
    };
    let mut messages = vec![root];
    messages.extend(keep_last(replies, limit.saturating_sub(1)));
    messages
}

fn field(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn graph_list(page: &Value) -> Result<Vec<TeamsHistoryMessage>, MicrosoftError> {
    let items = page["value"].as_array().ok_or_else(|| {
        MicrosoftError::Invalid("Microsoft Graph returned a message list without values.".into())
    })?;
    Ok(items.iter().filter_map(history_message).collect())
}

/// Users signed in with an organization account carry their Entra object ID.
fn is_organization_user(user: &Value) -> bool {
    matches!(
        user["userIdentityType"].as_str(),
        None | Some("aadUser" | "onPremiseAadUser" | "federatedUser")
    )
}

/// A Graph `chatMessage`. System and deleted messages are skipped.
fn history_message(message: &Value) -> Option<TeamsHistoryMessage> {
    if message["messageType"] != "message" || !message["deletedDateTime"].is_null() {
        return None;
    }
    let application = message
        .pointer("/from/application")
        .filter(|a| a.is_object());
    let user = message.pointer("/from/user").filter(|u| u.is_object());
    let author = application.or(user);
    let body = message["body"]["content"].as_str().unwrap_or_default();
    let is_html = message["body"]["contentType"]
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("html"));
    Some(TeamsHistoryMessage {
        id: Some(field(message, "/id")?),
        author_id: author.and_then(|a| field(a, "/id")),
        author_aad_object_id: user
            .filter(|u| is_organization_user(u))
            .and_then(|u| field(u, "/id")),
        author_name: author.and_then(|a| field(a, "/displayName")),
        is_bot: application.is_some(),
        text: cap(
            if is_html {
                html_to_text(body)
            } else {
                body.trim().to_owned()
            },
            MAX_TEXT_BYTES,
        ),
        html: (is_html && !body.trim().is_empty() && body.len() <= MAX_HTML_BYTES)
            .then(|| body.to_owned()),
        created_at: field(message, "/createdDateTime"),
        reply_to_id: field(message, "/replyToId"),
        attachments: file_links(message),
    })
}

fn file_links(message: &Value) -> Vec<TeamsFileLink> {
    message["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|attachment| {
            let link = field(attachment, "/contentUrl").filter(|link| link.starts_with("https://"));
            let name = field(attachment, "/name");
            (link.is_some() || name.is_some()).then(|| TeamsFileLink {
                name,
                content_type: field(attachment, "/contentType"),
                link,
            })
        })
        .collect()
}

/// Entries of `Conversation.messages`. User entries keep their stored author; replies are
/// the bot's.
fn stored_messages(entries: &[Value], bot_name: &str) -> Vec<TeamsHistoryMessage> {
    entries
        .iter()
        .filter_map(|entry| {
            let is_bot = match entry["role"].as_str()? {
                "user" => false,
                "assistant" => true,
                _ => return None,
            };
            let text = cap(stored_text(&entry["content"]), MAX_TEXT_BYTES);
            if text.is_empty() {
                return None;
            }
            Some(TeamsHistoryMessage {
                id: field(entry, "/id"),
                author_name: if is_bot {
                    Some(bot_name.trim().to_owned()).filter(|name| !name.is_empty())
                } else {
                    field(entry, "/author")
                },
                is_bot,
                text,
                created_at: timestamp(&entry["at"]),
                ..TeamsHistoryMessage::default()
            })
        })
        .collect()
}

fn stored_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_owned(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str().map(str::trim))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// ISO 8601 text as stored, or Unix milliseconds.
fn timestamp(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_owned()).filter(|text| !text.is_empty()),
        Value::Number(millis) => millis
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        _ => None,
    }
}

/// Cuts on a char boundary and marks the cut with "…", staying within `max` bytes.
fn cap(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
    text
}

fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        push_html_text(&mut text, &rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = &rest[start..];
            break;
        };
        text.push_str(tag_text(&rest[start + 1..start + end]));
        rest = &rest[start + end + 1..];
    }
    push_html_text(&mut text, rest);
    collapse_whitespace(&text)
}

/// Source whitespace, line breaks included, is a plain space in HTML.
fn push_html_text(text: &mut String, raw: &str) {
    text.extend(
        decode_entities(raw)
            .chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c }),
    );
}

fn tag_text(tag: &str) -> &'static str {
    let (closing, tag) = match tag.strip_prefix('/') {
        Some(tag) => (true, tag),
        None => (false, tag),
    };
    let end = tag
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(tag.len());
    let name = tag[..end].to_ascii_lowercase();
    let heading = matches!(name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    match (closing, name.as_str()) {
        (_, "br" | "ul" | "ol" | "table") => "\n",
        (true, "p" | "div" | "li" | "tr" | "blockquote" | "pre") => "\n",
        (true, _) if heading => "\n",
        (false, "li") => "- ",
        (false, "at") => "@",
        (_, "img") => " [image] ",
        _ => "",
    }
}

fn decode_entities(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        text.push_str(&rest[..start]);
        let body = &rest[start + 1..];
        if let Some(end) = body.find(';').filter(|end| *end <= 10)
            && let Some(decoded) = entity(&body[..end])
        {
            text.push(decoded);
            rest = &body[end + 1..];
        } else {
            text.push('&');
            rest = body;
        }
    }
    text.push_str(rest);
    text
}

fn entity(name: &str) -> Option<char> {
    let decoded = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let code = name.strip_prefix('#')?;
            let value = match code.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => code.parse().ok()?,
            };
            char::from_u32(value)?
        }
    };
    (!decoded.is_control() || decoded.is_whitespace()).then_some(decoded)
}

/// One space between words and one line break between lines, without empty lines.
fn collapse_whitespace(text: &str) -> String {
    text.split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/teams/members",
    operation_id = "get_teams_members",
    tag = "execution",
    description = "List the members of the Teams conversation that started this run, one page at a time. Needs no read permission. Executor only.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = TeamsMembersRequest,
    responses(
        (status = 200, description = "A page of members", body = TeamsMembersResponse),
        (status = 400, description = "Invalid limit or continuation token"),
        (status = 403, description = "The caller is not this app's running Teams execution, or the bot is no longer in the conversation"),
        (status = 404, description = "Teams could not find the conversation"),
        (status = 429, description = "Teams is throttling the bot"),
        (status = 502, description = "Teams could not be reached or answered unexpectedly")
    ),
    security(("executor_jwt" = []))
)]
pub async fn members(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app): Path<String>,
    Json(input): Json<TeamsMembersRequest>,
) -> Result<Json<TeamsMembersResponse>, ApiError> {
    ensure_run_caller(&user, &app, &input.session_id)?;
    let limit = bounded(input.limit, MAX_MEMBERS, "Teams members")?;
    let cursor = MemberCursor::decode(input.continuation_token.as_deref())?;
    let (session, c) = running_session(&state, &app, &input.session_id).await?;
    let page_size = (cursor.skip + limit)
        .clamp(MIN_MEMBER_PAGE, MAX_MEMBERS as usize)
        .to_string();
    let mut query = vec![("pageSize", page_size.as_str())];
    if let Some(token) = &cursor.token {
        query.push(("continuationToken", token.as_str()));
    }
    let (page, token) = microsoft::connector_get(
        &state,
        &c,
        &session.service_url,
        &[
            "v3",
            "conversations",
            base_conversation(&session.conversation_id),
            "pagedmembers",
        ],
        &query,
    )
    .await
    .and_then(|page| roster_page(&page))
    .map_err(|error| error.into_api("the conversation's members"))?;
    let (members, next) = cursor.split(page, token, limit);
    Ok(Json(TeamsMembersResponse {
        members,
        continuation_token: next.map(|cursor| cursor.encode()),
    }))
}

fn roster_page(page: &Value) -> Result<(Vec<TeamsMemberInfo>, Option<String>), MicrosoftError> {
    let members = page["members"].as_array().ok_or_else(|| {
        MicrosoftError::Invalid("Teams returned a member page without members.".into())
    })?;
    Ok((
        members
            .iter()
            .filter_map(TeamsMemberInfo::from_bot_framework)
            .collect(),
        field(page, "/continuationToken"),
    ))
}

/// Position in the roster: a Teams page and how many of its members were already returned.
/// Teams pages hold at least 50 members, so a smaller `limit` reads one page several times.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct MemberCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default)]
    skip: usize,
}

impl MemberCursor {
    fn decode(value: Option<&str>) -> Result<Self, ApiError> {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::default());
        };
        let invalid = || {
            ApiError::bad_request(
                "The Teams members continuation token is not valid. Pass the token from the previous members response.",
            )
        };
        if value.len() > MAX_CURSOR_BYTES {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.skip >= MAX_MEMBERS as usize {
            return Err(invalid());
        }
        Ok(cursor)
    }

    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap_or_default())
    }

    /// The members to return and the cursor after them.
    fn split(
        self,
        page: Vec<TeamsMemberInfo>,
        next: Option<String>,
        limit: usize,
    ) -> (Vec<TeamsMemberInfo>, Option<Self>) {
        let mut members: Vec<_> = page.into_iter().skip(self.skip).collect();
        if members.len() > limit {
            members.truncate(limit);
            let skip = self.skip + limit;
            return (
                members,
                Some(Self {
                    token: self.token,
                    skip,
                }),
            );
        }
        (
            members,
            next.map(|token| Self {
                token: Some(token),
                skip: 0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graph(id: &str, created: &str, text: &str) -> Value {
        json!({
            "id": id, "messageType": "message", "createdDateTime": created, "deletedDateTime": null,
            "from": {"application": null, "device": null, "user": {
                "@odata.type": "#microsoft.graph.teamworkUserIdentity",
                "id": "8ea0e38b-efb3-4757-924a-5f94061cf8c2", "displayName": "Robin Kline",
                "userIdentityType": "aadUser", "tenantId": "t"
            }},
            "body": {"contentType": "text", "content": text},
            "attachments": []
        })
    }

    fn message(id: &str, created: &str) -> TeamsHistoryMessage {
        TeamsHistoryMessage {
            id: Some(id.into()),
            created_at: Some(created.into()),
            ..TeamsHistoryMessage::default()
        }
    }

    fn ids(messages: &[TeamsHistoryMessage]) -> Vec<&str> {
        messages.iter().filter_map(|m| m.id.as_deref()).collect()
    }

    fn member(id: usize) -> TeamsMemberInfo {
        TeamsMemberInfo {
            id: format!("29:{id}"),
            ..TeamsMemberInfo::default()
        }
    }

    #[test]
    fn html_becomes_plain_text_with_line_breaks_and_mentions() {
        assert_eq!(
            html_to_text(
                "<p>Hi <at id=\"0\">Felix</at>,</p>\n<p>see   the&nbsp;plan &amp; <b>notes</b>:<br>line&#160;two &lt;x&gt; &#x1F600; &quot;q&quot;</p>"
            ),
            "Hi @Felix,\nsee the plan & notes:\nline two <x> 😀 \"q\""
        );
        assert_eq!(
            html_to_text("<div><div>a</div></div><ul><li>one</li><li>two</li></ul><p>b</p>"),
            "a\n- one\n- two\nb"
        );
        assert_eq!(
            html_to_text("<p>a</p><p><br></p><p><br></p><p>b</p>"),
            "a\nb"
        );
        assert_eq!(
            html_to_text("<img src=\"https://x\" alt=\"image\">"),
            "[image]"
        );
        assert_eq!(
            html_to_text("a &unknown; b &amp c &#0; d < e"),
            "a &unknown; b &amp c &#0; d < e"
        );
        assert_eq!(html_to_text("<attachment id=\"1\"></attachment>"), "");
    }

    #[test]
    fn text_is_capped_on_a_char_boundary() {
        assert_eq!(cap("short".into(), 8000), "short");
        let capped = cap("ä".repeat(5000), 8000);
        assert!(capped.len() <= 8000 && capped.ends_with('…'));
        assert!(capped.trim_end_matches('…').chars().all(|c| c == 'ä'));
        assert_eq!(cap("abcdef".into(), 5), "ab…");
    }

    #[test]
    fn graph_messages_map_authors_bodies_and_files() {
        let mut html = graph(
            "1616965872395",
            "2026-09-28T09:00:00.000Z",
            "<p>Hello <at id=\"0\">Bot</at></p>",
        );
        html["body"]["contentType"] = json!("html");
        html["replyToId"] = json!("1616965000000");
        html["attachments"] = json!([
            {"id":"a","contentType":"reference","contentUrl":"https://contoso.sharepoint.com/sites/x/report.pdf","name":"report.pdf"},
            {"id":"b","contentType":"application/vnd.microsoft.card.adaptive","content":"{}","name":null},
            {"id":"c","contentType":"messageReference","content":"{}"},
            {"id":"d","contentType":"reference","contentUrl":"javascript:alert(1)","name":"x.txt"}
        ]);
        let mapped = history_message(&html).unwrap();
        assert_eq!(
            mapped,
            TeamsHistoryMessage {
                id: Some("1616965872395".into()),
                author_id: Some("8ea0e38b-efb3-4757-924a-5f94061cf8c2".into()),
                author_aad_object_id: Some("8ea0e38b-efb3-4757-924a-5f94061cf8c2".into()),
                author_name: Some("Robin Kline".into()),
                is_bot: false,
                text: "Hello @Bot".into(),
                html: Some("<p>Hello <at id=\"0\">Bot</at></p>".into()),
                created_at: Some("2026-09-28T09:00:00.000Z".into()),
                reply_to_id: Some("1616965000000".into()),
                attachments: vec![
                    TeamsFileLink {
                        name: Some("report.pdf".into()),
                        content_type: Some("reference".into()),
                        link: Some("https://contoso.sharepoint.com/sites/x/report.pdf".into()),
                    },
                    TeamsFileLink {
                        name: Some("x.txt".into()),
                        content_type: Some("reference".into()),
                        link: None,
                    },
                ],
            }
        );
        let wire = serde_json::to_value(&mapped).unwrap();
        assert_eq!(wire["attachments"][0]["type"], "reference");
        assert!(wire.get("content_type").is_none());

        let bot = json!({
            "id": "2", "messageType": "message", "createdDateTime": "2026-09-28T09:01:00.000Z",
            "from": {"user": null, "application": {"id": "bot-app", "displayName": "Support Bot", "applicationIdentityType": "bot"}},
            "body": {"contentType": "text", "content": "  Done.  "}
        });
        let bot = history_message(&bot).unwrap();
        assert!(bot.is_bot);
        assert_eq!(bot.author_id.as_deref(), Some("bot-app"));
        assert_eq!(bot.author_aad_object_id, None);
        assert_eq!(bot.author_name.as_deref(), Some("Support Bot"));
        assert_eq!(bot.text, "Done.");
        assert_eq!(bot.html, None);

        let mut guest = graph("3", "2026-09-28T09:02:00.000Z", "hi");
        guest["from"]["user"]["userIdentityType"] = json!("anonymousGuest");
        let guest = history_message(&guest).unwrap();
        assert_eq!(guest.author_aad_object_id, None);
        assert!(guest.author_id.is_some());
    }

    #[test]
    fn graph_lists_skip_system_and_deleted_messages() {
        let mut system = graph("s", "2026-09-28T08:00:00.000Z", "");
        system["messageType"] = json!("systemEventMessage");
        let mut deleted = graph("d", "2026-09-28T08:01:00.000Z", "gone");
        deleted["deletedDateTime"] = json!("2026-09-28T08:02:00.000Z");
        let page = json!({"value": [
            graph("b", "2026-09-28T09:00:00.000Z", "second"),
            system,
            deleted,
            {"messageType":"message","body":{"content":"no id"}},
            graph("a", "2026-09-28T08:30:00.000Z", "first"),
        ]});
        assert_eq!(ids(&oldest_first(graph_list(&page).unwrap())), ["a", "b"]);
        assert!(graph_list(&json!({"error": {}})).is_err());
    }

    #[test]
    fn threads_keep_the_root_and_the_latest_replies() {
        let root = message("root", "2026-09-28T08:00:00.000Z");
        let replies = vec![
            message("r3", "2026-09-28T08:03:00.000Z"),
            message("r1", "2026-09-28T08:01:00.000Z"),
            message("r2", "2026-09-28T08:02:00.000Z"),
        ];
        assert_eq!(
            ids(&thread(Some(root.clone()), replies.clone(), 20)),
            ["root", "r1", "r2", "r3"]
        );
        assert_eq!(
            ids(&thread(Some(root.clone()), replies.clone(), 3)),
            ["root", "r2", "r3"]
        );
        assert_eq!(ids(&thread(Some(root), replies.clone(), 1)), ["root"]);
        assert_eq!(ids(&thread(None, replies, 2)), ["r2", "r3"]);
    }

    #[test]
    fn chat_pages_are_reversed_to_oldest_first_and_trimmed() {
        let newest_first = vec![
            message("c", "2026-09-28T08:03:00.000Z"),
            message("b", "2026-09-28T08:02:00.000Z"),
            message("a", "2026-09-28T08:01:00.000Z"),
        ];
        assert_eq!(ids(&keep_last(oldest_first(newest_first), 2)), ["b", "c"]);
    }

    #[test]
    fn bot_history_maps_stored_entries_in_order() {
        let entries = vec![
            json!({"role":"user","content":"old message"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"system","content":"ignored"}),
            json!({"role":"user","content":"   "}),
            json!({"role":"user","content":"hi\n[image: photo.png]","id":"act-1","author":"Felix Schultz","at":"2026-09-28T09:00:00.000Z","extra":true}),
            json!({"role":"user","content":[{"type":"text","text":"with parts"},{"type":"image_url","image_url":{"url":"https://x"}}],"at":1790582400000_i64}),
        ];
        let messages = stored_messages(&entries, " Support Bot ");
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].text, "old message");
        assert!(!messages[0].is_bot && messages[0].author_name.is_none());
        assert!(messages[1].is_bot);
        assert_eq!(messages[1].author_name.as_deref(), Some("Support Bot"));
        assert_eq!(
            messages[2],
            TeamsHistoryMessage {
                id: Some("act-1".into()),
                author_name: Some("Felix Schultz".into()),
                text: "hi\n[image: photo.png]".into(),
                created_at: Some("2026-09-28T09:00:00.000Z".into()),
                ..TeamsHistoryMessage::default()
            }
        );
        assert_eq!(messages[3].text, "with parts");
        assert_eq!(
            messages[3].created_at.as_deref(),
            Some("2026-09-28T08:00:00.000Z")
        );
        assert_eq!(keep_last(messages, 2).len(), 2);
        assert!(
            stored_messages(&[json!({"role":"assistant","content":"x"})], "")[0]
                .author_name
                .is_none()
        );
    }

    #[test]
    fn requests_validate_limits_and_scope() {
        assert_eq!(bounded(20, MAX_MESSAGES, "Teams messages").unwrap(), 20);
        for limit in [0, -1, 51] {
            let error = bounded(limit, MAX_MESSAGES, "Teams messages").unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
            assert!(error.public_message().unwrap().contains(&limit.to_string()));
        }
        assert!(bounded(500, MAX_MEMBERS, "Teams members").is_ok());
        assert!(bounded(501, MAX_MEMBERS, "Teams members").is_err());

        let request: TeamsMessagesRequest =
            serde_json::from_value(json!({"session_id":"run"})).unwrap();
        assert_eq!(
            (request.scope, request.limit),
            (TeamsMessagesScope::Thread, 20)
        );
        let request: TeamsMessagesRequest =
            serde_json::from_value(json!({"session_id":"run","scope":"conversation","limit":5}))
                .unwrap();
        assert_eq!(
            (request.scope, request.limit),
            (TeamsMessagesScope::Conversation, 5)
        );
        for invalid in [
            json!({"session_id":"run","scope":"channel"}),
            json!({"session_id":"run","extra":1}),
            json!({"scope":"thread"}),
        ] {
            assert!(serde_json::from_value::<TeamsMessagesRequest>(invalid).is_err());
        }
        let request: TeamsMembersRequest =
            serde_json::from_value(json!({"session_id":"run"})).unwrap();
        assert_eq!((request.limit, request.continuation_token), (100, None));
        assert!(
            serde_json::from_value::<TeamsMembersRequest>(json!({"session_id":"run","page":2}))
                .is_err()
        );
    }

    #[test]
    fn read_messages_permission_is_required_for_graph_reads() {
        let mut session: Session = serde_json::from_value(json!({
            "connection_id":"c","app_id":"app","event_id":"e","run_id":"r","tenant_id":"t",
            "client_id":"id","conversation_id":"19:x@thread.v2","service_url":"https://smba.trafficmanager.net/emea/",
            "activity_id":"a","user_id":"u","bot_id":"b","history_key":"conversation:x",
            "conversation_type":"groupChat","permissions":["meeting_details"]
        }))
        .unwrap();
        let error = ensure_read_messages(&session).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
        assert!(
            error
                .public_message()
                .unwrap()
                .contains("Read channel and chat messages")
        );
        session.permissions.push(TeamsPermission::ReadMessages);
        assert!(ensure_read_messages(&session).is_ok());
        assert_eq!(
            message_access_error(MicrosoftError::Forbidden).status(),
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            message_access_error(MicrosoftError::ConsentRequired).status(),
            axum::http::StatusCode::CONFLICT
        );
    }

    #[test]
    fn member_pages_split_below_the_teams_minimum_without_skipping_anyone() {
        let page: Vec<_> = (0..50).map(member).collect();
        let (first, next) = MemberCursor::default().split(page.clone(), Some("bf-2".into()), 20);
        assert_eq!(first.len(), 20);
        let next = next.unwrap();
        assert_eq!(
            next,
            MemberCursor {
                token: None,
                skip: 20
            }
        );
        let next = MemberCursor::decode(Some(&next.encode())).unwrap();
        let (second, next) = next.split(page.clone(), Some("bf-2".into()), 20);
        assert_eq!(second[0].id, "29:20");
        let (third, next) = next.unwrap().split(page.clone(), Some("bf-2".into()), 20);
        assert_eq!(
            (third.len(), third[0].id.as_str(), third[9].id.as_str()),
            (10, "29:40", "29:49")
        );
        let next = next.unwrap();
        assert_eq!(
            next,
            MemberCursor {
                token: Some("bf-2".into()),
                skip: 0
            }
        );
        let (last, done) = next.split(page, None, 100);
        assert_eq!(last.len(), 50);
        assert!(done.is_none());
    }

    #[test]
    fn member_cursors_reject_foreign_tokens() {
        assert_eq!(MemberCursor::decode(None).unwrap(), MemberCursor::default());
        assert_eq!(
            MemberCursor::decode(Some("  ")).unwrap(),
            MemberCursor::default()
        );
        for invalid in [
            "not base64!".to_owned(),
            URL_SAFE_NO_PAD.encode("not json"),
            URL_SAFE_NO_PAD.encode(r#"{"token":"x","skip":500}"#),
            "a".repeat(MAX_CURSOR_BYTES + 1),
        ] {
            let error = MemberCursor::decode(Some(&invalid)).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        }
        let cursor = MemberCursor {
            token: Some("opaque+/=token".into()),
            skip: 7,
        };
        assert_eq!(
            MemberCursor::decode(Some(&cursor.encode())).unwrap(),
            cursor
        );
    }

    #[test]
    fn roster_pages_read_members_and_the_continuation_token() {
        let (members, token) = roster_page(&json!({
            "continuationToken": "next",
            "members": [
                {"id":"29:a","name":"Anna","aadObjectId":"oid","userRole":"user"},
                {"name":"no id"}
            ]
        }))
        .unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name.as_deref(), Some("Anna"));
        assert_eq!(token.as_deref(), Some("next"));
        assert_eq!(
            roster_page(&json!({"members":[],"continuationToken":""}))
                .unwrap()
                .1,
            None
        );
        assert!(roster_page(&json!({})).is_err());
    }
}
