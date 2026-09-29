use super::{Connection, auth, hash, now, store};
use crate::{error::ApiError, state::AppState};
use reqwest::{StatusCode, header::HeaderMap};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{sync::LazyLock, time::Duration};
use utoipa::ToSchema;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0/";
const CONNECTOR: &str = "Teams";
const GRAPH: &str = "Microsoft Graph";
const HOUR_MS: i64 = 3_600_000;
const MEMBER_TTL_MS: i64 = 12 * HOUR_MS;

static HTTP: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .ok()
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MicrosoftError {
    /// The bot has no service principal or admin consent in the customer tenant.
    ConsentRequired,
    /// RSC is not granted, or the bot is not in the team, chat or meeting.
    Forbidden,
    NotFound,
    /// Retry-After seconds, when Microsoft sent them.
    Throttled(Option<u64>),
    Unavailable(String),
    Invalid(String),
}

impl MicrosoftError {
    pub fn into_api(self, what: &str) -> ApiError {
        match self {
            Self::ConsentRequired => ApiError::conflict(format!(
                "Microsoft needs an administrator's consent before the bot can read {what}. Check Graph access in the Teams bot setup."
            )),
            Self::Forbidden => ApiError::forbidden(format!(
                "Microsoft did not allow the bot to read {what}. Check that the bot is installed there and its permissions were approved."
            )),
            Self::NotFound => ApiError::not_found(format!("Microsoft could not find {what}")),
            Self::Throttled(after) => ApiError::too_many_requests(format!(
                "Microsoft is throttling requests for {what}. Retry after {} seconds.",
                after.unwrap_or(1)
            )),
            Self::Unavailable(reason) | Self::Invalid(reason) => {
                ApiError::bad_gateway(format!("Could not load {what}. {reason}"))
            }
        }
    }
}

impl From<ApiError> for MicrosoftError {
    fn from(error: ApiError) -> Self {
        let reason = error
            .public_message()
            .unwrap_or("Flow-Like could not prepare the Microsoft request.")
            .to_owned();
        if error.status().is_client_error() {
            Self::Invalid(reason)
        } else {
            Self::Unavailable(reason)
        }
    }
}

/// A conversation member as the Bot Framework roster reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TeamsMemberInfo {
    /// Bot Framework user ID (`29:…`).
    pub id: String,
    /// Microsoft Entra object ID. Missing for anonymous meeting guests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aad_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_principal_name: Option<String>,
    /// The member's home tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Role in the conversation, such as `user`, `guest` or `anonymous`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_role: Option<String>,
}

impl TeamsMemberInfo {
    /// Reads a Bot Framework `TeamsChannelAccount`. `None` without an `id`.
    pub(super) fn from_bot_framework(value: &Value) -> Option<Self> {
        Some(Self {
            id: text(value, "/id")?,
            aad_object_id: text(value, "/aadObjectId").or_else(|| text(value, "/objectId")),
            name: text(value, "/name"),
            given_name: text(value, "/givenName"),
            surname: text(value, "/surname"),
            email: text(value, "/email"),
            user_principal_name: text(value, "/userPrincipalName"),
            tenant_id: text(value, "/tenantId"),
            user_role: text(value, "/userRole"),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct TeamInfo {
    pub id: String,
    pub name: Option<String>,
    pub aad_group_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ChannelInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct MeetingInfo {
    pub title: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub join_url: Option<String>,
    /// The connector reports the organizer by ID only; the name comes from the roster.
    pub organizer_name: Option<String>,
    pub organizer_id: Option<String>,
    pub organizer_aad_object_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct GraphTeam {
    pub description: Option<String>,
    pub web_url: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct GraphChannel {
    pub description: Option<String>,
    pub membership_type: Option<String>,
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct GraphChat {
    pub topic: Option<String>,
    pub chat_type: Option<String>,
    pub web_url: Option<String>,
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A channel conversation ID without its `;messageid=` thread suffix.
pub(super) fn base_conversation(id: &str) -> &str {
    id.split_once(";messageid=").map_or(id, |(base, _)| base)
}

pub(super) fn retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn rejection(service: &str, status: StatusCode, retry_after: Option<u64>) -> MicrosoftError {
    match status {
        StatusCode::FORBIDDEN => MicrosoftError::Forbidden,
        StatusCode::NOT_FOUND => MicrosoftError::NotFound,
        StatusCode::TOO_MANY_REQUESTS => MicrosoftError::Throttled(retry_after),
        StatusCode::UNAUTHORIZED => MicrosoftError::Unavailable(format!(
            "{service} did not accept the bot's token. Retry shortly."
        )),
        status if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT => {
            MicrosoftError::Unavailable(format!("{service} returned HTTP {status}. Retry shortly."))
        }
        status => MicrosoftError::Invalid(format!(
            "{service} rejected the request with HTTP {status}."
        )),
    }
}

fn transport_error(service: &str, error: &reqwest::Error) -> MicrosoftError {
    MicrosoftError::Unavailable(if error.is_timeout() {
        format!("{service} did not respond within 10 seconds.")
    } else {
        format!("{service} could not be reached.")
    })
}

async fn get_json(
    service: &str,
    url: reqwest::Url,
    token: &str,
    on_rejection: impl FnOnce(StatusCode),
) -> Result<Value, MicrosoftError> {
    let client = HTTP.as_ref().ok_or_else(|| {
        MicrosoftError::Unavailable("The Microsoft HTTP client could not be initialized.".into())
    })?;
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| transport_error(service, &error))?;
    let status = response.status();
    if !status.is_success() {
        on_rejection(status);
        return Err(rejection(service, status, retry_after(response.headers())));
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| transport_error(service, &error))?;
    serde_json::from_slice(&body).map_err(|_| {
        MicrosoftError::Invalid(format!("{service} returned a response that is not JSON."))
    })
}

fn connector_url(
    service_url: &str,
    segments: &[&str],
    query: &[(&str, &str)],
) -> Result<reqwest::Url, MicrosoftError> {
    let untrusted = || {
        MicrosoftError::Invalid(
            "The Teams service URL is not a Microsoft Bot Framework host.".into(),
        )
    };
    let mut url = auth::service_url(service_url).map_err(|_| untrusted())?;
    url.path_segments_mut()
        .map_err(|_| untrusted())?
        .pop_if_empty()
        .extend(segments);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}

/// GETs a Bot Framework connector resource with the bot token.
pub(super) async fn connector_get(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    segments: &[&str],
    query: &[(&str, &str)],
) -> Result<Value, MicrosoftError> {
    let url = connector_url(service_url, segments, query)?;
    let token = auth::bot_token(state, c).await?;
    get_json(CONNECTOR, url, &token, |status| {
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            auth::forget_bot_token(c);
        }
    })
    .await
}

/// Percent-encodes each segment, so IDs such as `19:…@thread.tacv2` stay one segment.
pub(super) fn graph_path(segments: &[&str]) -> String {
    segments
        .iter()
        .map(|segment| urlencoding::encode(segment))
        .collect::<Vec<_>>()
        .join("/")
}

/// Refuses paths the URL parser would rewrite, such as dot or empty segments, since they
/// would read a different Graph resource than the one the caller named.
fn graph_url(path_and_query: &str) -> Result<reqwest::Url, MicrosoftError> {
    let invalid = || {
        MicrosoftError::Invalid(format!(
            "The Microsoft Graph path '{path_and_query}' is not a canonical resource path."
        ))
    };
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    if path.split('/').any(str::is_empty) {
        return Err(invalid());
    }
    let url =
        reqwest::Url::parse(&format!("{GRAPH_BASE}{path_and_query}")).map_err(|_| invalid())?;
    if url.path() != format!("/v1.0/{path}") {
        return Err(invalid());
    }
    Ok(url)
}

/// GETs `https://graph.microsoft.com/v1.0/{path_and_query}` with the customer-tenant Graph
/// token. Build the path with [`graph_path`].
pub(super) async fn graph_get(
    state: &AppState,
    c: &Connection,
    path_and_query: &str,
) -> Result<Value, MicrosoftError> {
    let url = graph_url(path_and_query)?;
    let token = auth::graph_token(state, c).await?;
    get_json(GRAPH, url, &token, |status| {
        if status == StatusCode::UNAUTHORIZED {
            auth::forget_graph_token(c);
        }
    })
    .await
}

fn cache_key(c: &Connection, kind: &str, ids: &[&str]) -> String {
    let mut parts = vec![
        c.id.as_str(),
        c.client_id.as_str(),
        c.customer_tenant_id.as_str(),
    ];
    parts.extend_from_slice(ids);
    format!("{kind}:{}", hash(&parts))
}

/// Serves `lookup` from `TeamsBotState` for `ttl_ms`. Storage failures only cost a refetch.
async fn cached<T: Serialize + DeserializeOwned>(
    state: &AppState,
    c: &Connection,
    kind: &str,
    ids: &[&str],
    ttl_ms: i64,
    lookup: impl Future<Output = Result<T, MicrosoftError>>,
) -> Result<T, MicrosoftError> {
    let key = cache_key(c, kind, ids);
    match store::get::<T>(state, &key).await {
        Ok(Some((value, _))) => return Ok(value),
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(connection_id = %c.id, kind, %error, "Could not read a cached Teams lookup")
        }
    }
    let value = lookup.await?;
    if let Err(error) = store::put(state, &key, &c.id, &value, now() + ttl_ms).await {
        tracing::warn!(connection_id = %c.id, kind, %error, "Could not cache a Teams lookup");
    }
    Ok(value)
}

pub(super) async fn member(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    conversation_id: &str,
    user_id: &str,
) -> Result<TeamsMemberInfo, MicrosoftError> {
    let conversation_id = base_conversation(conversation_id);
    cached(
        state,
        c,
        "member",
        &[conversation_id, user_id],
        MEMBER_TTL_MS,
        async {
            let value = connector_get(
                state,
                c,
                service_url,
                &["v3", "conversations", conversation_id, "members", user_id],
                &[],
            )
            .await?;
            TeamsMemberInfo::from_bot_framework(&value).ok_or_else(|| {
                MicrosoftError::Invalid("Teams returned a member without an ID.".into())
            })
        },
    )
    .await
}

fn team_info(value: &Value, team_id: &str) -> TeamInfo {
    TeamInfo {
        id: text(value, "/id").unwrap_or_else(|| team_id.to_owned()),
        name: text(value, "/name"),
        aad_group_id: text(value, "/aadGroupId"),
    }
}

pub(super) async fn team(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    team_id: &str,
) -> Result<TeamInfo, MicrosoftError> {
    cached(state, c, "team", &[team_id], HOUR_MS, async {
        let value = connector_get(state, c, service_url, &["v3", "teams", team_id], &[]).await?;
        Ok(team_info(&value, team_id))
    })
    .await
}

/// Teams reports the General channel without a name.
fn channel_list(value: &Value) -> Result<Vec<ChannelInfo>, MicrosoftError> {
    let channels = value["conversations"].as_array().ok_or_else(|| {
        MicrosoftError::Invalid("Teams returned a channel list without conversations.".into())
    })?;
    Ok(channels
        .iter()
        .filter_map(|channel| {
            Some(ChannelInfo {
                id: text(channel, "/id")?,
                name: text(channel, "/name").unwrap_or_else(|| "General".into()),
            })
        })
        .collect())
}

pub(super) async fn channels(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    team_id: &str,
) -> Result<Vec<ChannelInfo>, MicrosoftError> {
    cached(state, c, "channels", &[team_id], HOUR_MS, async {
        let value = connector_get(
            state,
            c,
            service_url,
            &["v3", "teams", team_id, "conversations"],
            &[],
        )
        .await?;
        channel_list(&value)
    })
    .await
}

fn meeting_info(value: &Value) -> MeetingInfo {
    MeetingInfo {
        title: text(value, "/details/title"),
        start: text(value, "/details/scheduledStartTime"),
        end: text(value, "/details/scheduledEndTime"),
        join_url: text(value, "/details/joinUrl"),
        organizer_name: text(value, "/organizer/name"),
        organizer_id: text(value, "/organizer/id"),
        organizer_aad_object_id: text(value, "/organizer/aadObjectId")
            .or_else(|| text(value, "/organizer/objectId")),
    }
}

/// Needs the `MeetingDetails` RSC permission.
pub(super) async fn meeting(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    meeting_id: &str,
) -> Result<MeetingInfo, MicrosoftError> {
    cached(state, c, "meeting", &[meeting_id], HOUR_MS, async {
        let value =
            connector_get(state, c, service_url, &["v1", "meetings", meeting_id], &[]).await?;
        Ok(meeting_info(&value))
    })
    .await
}

/// `Organizer`, `Presenter` or `Attendee`.
pub(super) async fn meeting_role(
    state: &AppState,
    c: &Connection,
    service_url: &str,
    meeting_id: &str,
    aad_object_id: &str,
) -> Result<Option<String>, MicrosoftError> {
    let value = connector_get(
        state,
        c,
        service_url,
        &["v1", "meetings", meeting_id, "participants", aad_object_id],
        &[("tenantId", &c.customer_tenant_id)],
    )
    .await?;
    Ok(text(&value, "/meeting/role"))
}

pub(super) async fn graph_team(
    state: &AppState,
    c: &Connection,
    group_id: &str,
) -> Result<GraphTeam, MicrosoftError> {
    cached(state, c, "gteam", &[group_id], HOUR_MS, async {
        let path = format!(
            "{}?$select=displayName,description,webUrl,visibility",
            graph_path(&["teams", group_id])
        );
        let value = graph_get(state, c, &path).await?;
        Ok(GraphTeam {
            description: text(&value, "/description"),
            web_url: text(&value, "/webUrl"),
            visibility: text(&value, "/visibility"),
        })
    })
    .await
}

pub(super) async fn graph_channel(
    state: &AppState,
    c: &Connection,
    group_id: &str,
    channel_id: &str,
) -> Result<GraphChannel, MicrosoftError> {
    cached(
        state,
        c,
        "gchannel",
        &[group_id, channel_id],
        HOUR_MS,
        async {
            let path = format!(
                "{}?$select=displayName,description,membershipType,webUrl",
                graph_path(&["teams", group_id, "channels", channel_id])
            );
            let value = graph_get(state, c, &path).await?;
            Ok(GraphChannel {
                description: text(&value, "/description"),
                membership_type: text(&value, "/membershipType"),
                web_url: text(&value, "/webUrl"),
            })
        },
    )
    .await
}

pub(super) async fn graph_chat(
    state: &AppState,
    c: &Connection,
    chat_id: &str,
) -> Result<GraphChat, MicrosoftError> {
    cached(state, c, "gchat", &[chat_id], HOUR_MS, async {
        let path = format!(
            "{}?$select=topic,chatType,webUrl",
            graph_path(&["chats", chat_id])
        );
        let value = graph_get(state, c, &path).await?;
        Ok(GraphChat {
            topic: text(&value, "/topic"),
            chat_type: text(&value, "/chatType"),
            web_url: text(&value, "/webUrl"),
        })
    })
    .await
}

/// Drops the cached team and channel names after the bot sees the team change.
pub(super) async fn forget_team(
    state: &AppState,
    c: &Connection,
    team_id: &str,
) -> Result<(), MicrosoftError> {
    for kind in ["team", "channels"] {
        store::remove(state, &cache_key(c, kind, &[team_id])).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn members_parse_from_bot_framework_and_serialize_snake_case() {
        let member = TeamsMemberInfo::from_bot_framework(&json!({
            "id": "29:1GcS4EyB_oSI8A88XmWBN7NJFyMqe3QGnJdgLfFGkJnVelzRGos0bPbpsfJjcbAD22bmKc4GMbrY2g4JDrrA8vM06X1-cHHle4zOE6U4ttcc",
            "name": "Felix Schultz",
            "objectId": "9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c",
            "aadObjectId": "9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c",
            "givenName": "Felix",
            "surname": "Schultz",
            "email": "felix@example.com",
            "userPrincipalName": "felix@example.onmicrosoft.com",
            "tenantId": "72f988bf-86f1-41af-91ab-2d7cd011db47",
            "userRole": "user"
        }))
        .unwrap();
        assert_eq!(member.name.as_deref(), Some("Felix Schultz"));
        assert_eq!(
            member.aad_object_id.as_deref(),
            Some("9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c")
        );
        let wire = serde_json::to_value(&member).unwrap();
        assert_eq!(wire["given_name"], "Felix");
        assert_eq!(wire["user_principal_name"], "felix@example.onmicrosoft.com");
        assert_eq!(wire["tenant_id"], "72f988bf-86f1-41af-91ab-2d7cd011db47");
        assert_eq!(wire["user_role"], "user");
        assert!(wire.get("givenName").is_none());
        assert_eq!(
            serde_json::from_value::<TeamsMemberInfo>(wire).unwrap(),
            member
        );
    }

    #[test]
    fn anonymous_members_keep_only_what_teams_reports() {
        let member = TeamsMemberInfo::from_bot_framework(
            &json!({"id":"29:guest","name":"Guest","objectId":"legacy","userRole":"anonymous","email":""}),
        )
        .unwrap();
        assert_eq!(member.aad_object_id.as_deref(), Some("legacy"));
        assert_eq!(member.email, None);
        assert_eq!(
            serde_json::to_value(&member).unwrap(),
            json!({"id":"29:guest","aad_object_id":"legacy","name":"Guest","user_role":"anonymous"})
        );
        assert!(TeamsMemberInfo::from_bot_framework(&json!({"name":"No ID"})).is_none());
    }

    #[test]
    fn graph_paths_keep_teams_ids_in_one_segment() {
        assert_eq!(
            graph_path(&["chats", "19:abc@thread.tacv2", "messages"]),
            "chats/19%3Aabc%40thread.tacv2/messages"
        );
        assert_eq!(graph_path(&["teams", "a/b?c#d"]), "teams/a%2Fb%3Fc%23d");
        let url = graph_url(&format!(
            "{}?$select=topic,chatType",
            graph_path(&["chats", "19:meeting_x@thread.v2"])
        ))
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://graph.microsoft.com/v1.0/chats/19%3Ameeting_x%40thread.v2?$select=topic,chatType"
        );
        assert_eq!(
            graph_url("chats/x/messages?$top=5&$orderby=createdDateTime desc")
                .unwrap()
                .query(),
            Some("$top=5&$orderby=createdDateTime%20desc")
        );
    }

    #[test]
    fn graph_urls_refuse_paths_that_would_name_another_resource() {
        for path in [
            "teams/..",
            "teams/%2e%2e/channels",
            "teams/./x",
            "teams//x",
            "teams/",
            "",
            "teams/a b",
            "/teams/x",
        ] {
            assert!(graph_url(path).is_err(), "{path}");
        }
        assert!(graph_url("teams/1f0c9c5e-1b0c-4e8a-9d0f-0a7b3f4f3e11").is_ok());
    }

    #[test]
    fn connector_urls_stay_on_the_service_host_and_encode_ids() {
        let url = connector_url(
            "https://smba.trafficmanager.net/emea/",
            &[
                "v3",
                "conversations",
                "19:abc@thread.tacv2",
                "members",
                "29:x/y",
            ],
            &[],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://smba.trafficmanager.net/emea/v3/conversations/19:abc@thread.tacv2/members/29:x%2Fy"
        );
        let url = connector_url(
            "https://smba.trafficmanager.net/emea",
            &["v1", "meetings", "MCMx+=", "participants", "oid"],
            &[("tenantId", "tenant&x=1")],
        )
        .unwrap();
        assert_eq!(url.path(), "/emea/v1/meetings/MCMx+=/participants/oid");
        assert_eq!(url.query(), Some("tenantId=tenant%26x%3D1"));
        assert!(connector_url("https://attacker.invalid/", &["v3"], &[]).is_err());
    }

    #[test]
    fn statuses_map_to_lookup_errors() {
        assert_eq!(
            rejection(GRAPH, StatusCode::FORBIDDEN, None),
            MicrosoftError::Forbidden
        );
        assert_eq!(
            rejection(GRAPH, StatusCode::NOT_FOUND, None),
            MicrosoftError::NotFound
        );
        assert_eq!(
            rejection(GRAPH, StatusCode::TOO_MANY_REQUESTS, Some(12)),
            MicrosoftError::Throttled(Some(12))
        );
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::BAD_GATEWAY,
        ] {
            assert!(matches!(
                rejection(CONNECTOR, status, None),
                MicrosoftError::Unavailable(_)
            ));
        }
        assert!(matches!(
            rejection(CONNECTOR, StatusCode::BAD_REQUEST, None),
            MicrosoftError::Invalid(_)
        ));
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, " 30 ".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(30));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn lookup_errors_become_user_facing_api_errors() {
        for (error, status) in [
            (MicrosoftError::ConsentRequired, StatusCode::CONFLICT),
            (MicrosoftError::Forbidden, StatusCode::FORBIDDEN),
            (MicrosoftError::NotFound, StatusCode::NOT_FOUND),
            (
                MicrosoftError::Throttled(None),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                MicrosoftError::Unavailable("x".into()),
                StatusCode::BAD_GATEWAY,
            ),
            (MicrosoftError::Invalid("x".into()), StatusCode::BAD_GATEWAY),
        ] {
            let api = error.into_api("Teams messages");
            assert_eq!(api.status(), status);
            assert!(api.public_message().unwrap().contains("Teams messages"));
        }
        assert!(
            MicrosoftError::Throttled(Some(9))
                .into_api("the roster")
                .public_message()
                .unwrap()
                .contains("9 seconds")
        );
    }

    #[test]
    fn team_channel_and_meeting_details_parse_from_the_connector() {
        assert_eq!(
            team_info(
                &json!({"id":"19:team@thread.tacv2","name":"Support","aadGroupId":"group"}),
                "19:team@thread.tacv2"
            ),
            TeamInfo {
                id: "19:team@thread.tacv2".into(),
                name: Some("Support".into()),
                aad_group_id: Some("group".into()),
            }
        );
        assert_eq!(
            channel_list(&json!({"conversations":[
                {"id":"19:team@thread.tacv2","name":null},
                {"id":"19:other@thread.tacv2","name":"Escalations"},
                {"name":"No ID"}
            ]}))
            .unwrap(),
            vec![
                ChannelInfo {
                    id: "19:team@thread.tacv2".into(),
                    name: "General".into()
                },
                ChannelInfo {
                    id: "19:other@thread.tacv2".into(),
                    name: "Escalations".into()
                },
            ]
        );
        assert!(channel_list(&json!({})).is_err());
        let meeting = meeting_info(&json!({
            "details": {
                "title": "All Hands",
                "scheduledStartTime": "2026-09-28T09:00:00+00:00",
                "scheduledEndTime": "2026-09-28T10:00:00+00:00",
                "joinUrl": "https://teams.microsoft.com/l/meetup-join/x"
            },
            "organizer": {"id":"29:organizer","aadObjectId":"oid","objectId":"oid","tenantId":"tenant"}
        }));
        assert_eq!(meeting.title.as_deref(), Some("All Hands"));
        assert_eq!(meeting.end.as_deref(), Some("2026-09-28T10:00:00+00:00"));
        assert_eq!(meeting.organizer_name, None);
        assert_eq!(meeting.organizer_id.as_deref(), Some("29:organizer"));
        assert_eq!(meeting.organizer_aad_object_id.as_deref(), Some("oid"));
        let legacy = meeting_info(&json!({"organizer":{"id":"29:o","objectId":"legacy"}}));
        assert_eq!(legacy.organizer_aad_object_id.as_deref(), Some("legacy"));
        let cached: MeetingInfo =
            serde_json::from_value(json!({"title":"All Hands","organizer_name":null})).unwrap();
        assert_eq!(cached.organizer_id, None);
        assert_eq!(cached.title.as_deref(), Some("All Hands"));
    }

    #[test]
    fn cache_keys_are_scoped_to_the_bot_identity() {
        let connection = |client: &str| -> Connection {
            serde_json::from_value(json!({
                "id":"c", "app_id":"app", "event_id":"event", "mode":"customer_teams",
                "name":"Bot", "description":"", "customer_tenant_id":"tenant", "home_tenant_id":"home",
                "client_id":client, "secret":"s", "graph_object_id":null, "azure_resource_id":null,
                "secret_key_id":null, "secret_expires_at":null, "status":"ready", "allowed_responders":[]
            }))
            .unwrap()
        };
        let key = cache_key(&connection("a"), "team", &["19:t"]);
        assert!(key.starts_with("team:"));
        assert_ne!(key, cache_key(&connection("b"), "team", &["19:t"]));
        assert_ne!(key, cache_key(&connection("a"), "channels", &["19:t"]));
        assert_eq!(
            base_conversation("19:c@thread.tacv2;messageid=17"),
            "19:c@thread.tacv2"
        );
        assert_eq!(base_conversation("a:1"), "a:1");
    }
}
