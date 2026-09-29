use super::{
    Connection, TeamsPermission, auth, cards, client,
    context::{self, Context, Enrichment},
    files::{self, FileEntry},
    hash,
    microsoft::{self, MicrosoftError},
    now, store,
};
use crate::{
    entity::{app, event_sink, sea_orm_active_enums::Status},
    error::ApiError,
    execution::{ExecutionClaims, state::ExecutionRunRecord},
    middleware::jwt::AppUser,
    routes::{
        app::events::db::get_event_from_db,
        execution::progress::{ExecutionEventInput, get_state_store},
        sink::trigger::{TriggerEventInput, trigger_event_with_run_id},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use flow_like_types::{
    channel::{ChannelClientDescriptor, ChannelPush, ChannelPushKind},
    interaction::InteractionRequest,
    tokio::{self, time::Instant},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use utoipa::ToSchema;

const DAY_MS: i64 = 86_400_000;
const HISTORY_MS: i64 = 30 * DAY_MS;
const DISPATCH_LEASE_MS: i64 = 120_000;
const DELIVERY_LEASE_MS: i64 = 60_000;
const MAX_MESSAGE_BYTES: usize = 24_000;
const MAX_PASSIVE_BYTES: usize = 4_000;
const MAX_HISTORY: usize = 30;
/// Keeps the encrypted conversation row well under DSQL's 1 MiB limit.
const MAX_HISTORY_BYTES: usize = 200_000;
const MAX_COMPLETED_RUNS: usize = 100;
const MAX_RECORDED_REPLIES: usize = 200;
const MAX_PASSIVE_KEYS: usize = 500;
const HEALTH_REFRESH_MS: i64 = 60_000;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);
/// Shared by the context lookups and the file downloads, which run concurrently. Microsoft
/// expects the messaging endpoint to answer within about 15 seconds.
const PREPARE_BUDGET: Duration = Duration::from_secs(10);
const ANONYMOUS: &str = "anonymous:";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Session {
    pub(super) connection_id: String,
    pub(super) app_id: String,
    pub(super) event_id: String,
    pub(super) run_id: String,
    pub(super) tenant_id: String,
    pub(super) client_id: String,
    pub(super) conversation_id: String,
    pub(super) service_url: String,
    pub(super) activity_id: String,
    pub(super) user_id: String,
    pub(super) bot_id: String,
    pub(super) history_key: String,
    #[serde(default)]
    pub(super) conversation_type: String,
    #[serde(default)]
    pub(super) team_id: String,
    #[serde(default)]
    pub(super) channel_id: String,
    #[serde(default)]
    pub(super) thread_id: String,
    #[serde(default)]
    pub(super) meeting_id: String,
    #[serde(default)]
    pub(super) permissions: Vec<TeamsPermission>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Conversation {
    #[serde(default)]
    pub(super) messages: Vec<Value>,
    #[serde(default)]
    pub(super) local: Value,
    #[serde(default)]
    pub(super) last_run: String,
    /// Runs whose user message is already in `messages`.
    #[serde(default)]
    pub(super) completed_runs: Vec<String>,
    /// Replies already in `messages`, keyed by run and reply.
    #[serde(default)]
    pub(super) recorded: Vec<String>,
    /// `passive:{activity id}` of messages recorded as context or deleted, so a Microsoft
    /// retry never adds them again.
    #[serde(default)]
    pub(super) passive: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct UserSession {
    value: Value,
    completed_runs: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Action {
    session: Session,
    request: InteractionRequest,
    allowed_responders: Vec<String>,
    state: String,
    lease_until: i64,
    #[serde(default)]
    answer: Option<Value>,
    #[serde(default)]
    responder: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Delivery {
    status: String,
    message_id: Option<String>,
    hash: String,
    #[serde(default)]
    lease_until: i64,
}

#[derive(Serialize, Deserialize)]
struct Dispatch {
    status: String,
    lease_until: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum Claim {
    Done,
    InFlight,
    Retry,
}

/// What `incoming` does with an activity that is not a card action.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Ignore,
    /// The team changed, so cached team and channel names are stale.
    ForgetTeam(String),
    /// Context only: Teams delivered it because the bot may read every message.
    Passive,
    /// A message the history may hold was deleted or edited.
    Revise(Revision),
    Dispatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Revision {
    Delete,
    Edit,
}

fn reads_messages(c: &Connection) -> bool {
    c.permissions.contains(&TeamsPermission::ReadMessages)
}

/// Restoring a deleted message (`undeleteMessage`) is not tracked.
fn revision(activity: &Value) -> Option<Revision> {
    match (
        activity["type"].as_str(),
        context::field(activity, "/channelData/eventType"),
    ) {
        (Some("messageDelete"), None | Some("softDeleteMessage")) => Some(Revision::Delete),
        (Some("messageUpdate"), Some("editMessage")) => Some(Revision::Edit),
        _ => None,
    }
}

/// Installs, removals and renames arrive as `conversationUpdate` or `installationUpdate`.
fn other_activity(activity: &Value) -> Gate {
    let team_changed = matches!(
        activity["type"].as_str(),
        Some("conversationUpdate" | "installationUpdate")
    );
    match context::field(activity, "/channelData/team/id").filter(|_| team_changed) {
        Some(team) => Gate::ForgetTeam(team.to_owned()),
        None => Gate::Ignore,
    }
}

/// Other people's messages only reach the history when the bot may read every message.
/// Deletes and edits always apply, because they only change entries the history already holds.
fn gate(activity: &Value, reads_messages: bool) -> Result<Gate, ApiError> {
    match activity["type"].as_str() {
        Some("message") => {}
        Some("messageDelete" | "messageUpdate") => {
            return Ok(revision(activity).map_or(Gate::Ignore, Gate::Revise));
        }
        _ => return Ok(other_activity(activity)),
    }
    if activity["from"]["id"] == activity["recipient"]["id"] || context::from_bot(activity) {
        return Ok(Gate::Ignore);
    }
    let conversation_type = context::scope(activity).conversation_type;
    let mentions_bot = context::mentions_bot(activity);
    if !context::addressed(&conversation_type, mentions_bot) {
        return Ok(if reads_messages {
            Gate::Passive
        } else {
            Gate::Ignore
        });
    }
    if activity["text"].as_str().unwrap_or_default().len() > MAX_MESSAGE_BYTES {
        return Err(ApiError::bad_request(
            "Teams messages may contain at most 24000 bytes",
        ));
    }
    // Outside 1:1 chats a bare @mention asks the bot to look at the conversation.
    let bare_mention = mentions_bot && conversation_type != "personal";
    if !bare_mention
        && context::clean_text(activity).is_empty()
        && files::classify(activity).is_empty()
        && quote_line(activity).is_none()
    {
        return Ok(Gate::Ignore);
    }
    Ok(Gate::Dispatch)
}

/// The sender's Entra object ID, or a per-bot pseudonym for anonymous meeting guests that
/// never parses as a GUID, so approvals fail closed for them.
fn identity(connection_id: &str, activity: &Value) -> Result<(String, bool), ApiError> {
    match context::field(activity, "/from/aadObjectId") {
        Some(oid) => super::guid(oid)
            .map(|oid| (oid, false))
            .map_err(|_| ApiError::bad_request("Teams activity has an invalid sender object ID")),
        None => {
            let from = string(activity, "/from/id")?;
            Ok((format!("{ANONYMOUS}{}", hash(&[connection_id, from])), true))
        }
    }
}

fn history_key(c: &Connection, conversation: &str) -> String {
    format!(
        "conversation:{}",
        hash(&[&c.id, &c.client_id, &c.customer_tenant_id, conversation])
    )
}

/// Teams runs are identified by a [`hash`] of the originating activity.
fn is_teams_run(run_id: &str) -> bool {
    run_id.len() == 64 && run_id.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn live(
    state: &AppState,
    c: &Connection,
) -> Result<(event_sink::Model, flow_like::flow::event::Event), ApiError> {
    super::ensure_enabled(state)?;
    if c.status != "ready" {
        return Err(ApiError::forbidden("This Teams bot is not connected"));
    }
    if app::Entity::find_by_id(&c.app_id)
        .filter(app::Column::Status.eq(Status::Active))
        .one(&state.db)
        .await?
        .is_none()
    {
        return Err(ApiError::FORBIDDEN);
    }
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(&c.app_id))
        .filter(event_sink::Column::EventId.eq(&c.event_id))
        .filter(event_sink::Column::SinkType.eq("teams"))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    let event = get_event_from_db(&state.db, &c.event_id, &c.app_id).await?;
    if !event.active
        || event.event_type != "teams"
        || event.execution_mode != flow_like::flow::event::EventExecutionMode::Remote
    {
        return Err(ApiError::FORBIDDEN);
    }
    Ok((sink, event))
}

fn string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, ApiError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() < 4096)
        .ok_or_else(|| ApiError::bad_request("Teams activity is missing required context"))
}

#[utoipa::path(
    post,
    path = "/sink/trigger/teams/{connection_id}",
    operation_id = "receive_teams_activity",
    tag = "sink",
    description = "Messaging endpoint for Microsoft Teams. Accepts Bot Framework activities signed by Microsoft for this bot and runs its Teams event; card actions answer waiting Interactions.",
    params(("connection_id" = String, Path, description = "Teams bot connection ID from the bot setup")),
    request_body(content = Object, description = "Bot Framework activity"),
    responses(
        (status = 200, description = "Activity accepted; card actions return an invoke response", body = Object),
        (status = 400, description = "The activity is malformed or too large"),
        (status = 401, description = "The request is not signed by Microsoft for this bot"),
        (status = 403, description = "Teams is disabled, the bot, event or app is inactive, or the conversation belongs to another tenant"),
        (status = 404, description = "Unknown bot"),
        (status = 409, description = "The same message is already being dispatched"),
        (status = 500, description = "The workflow could not be dispatched; Microsoft may retry")
    )
)]
pub async fn incoming(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(activity): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    super::ensure_enabled(&state)?;
    let (c, _) = store::connection(&state, &id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if c.id != id {
        return Err(ApiError::NOT_FOUND);
    }
    auth::verify(&c, &headers, &activity).await?;
    let (sink, event) = live(&state, &c).await?;
    let action_data = activity
        .pointer("/value/action/data")
        .or_else(|| activity.get("value"));
    if let Some(data) = action_data.filter(|data| data.get("flow_like_action").is_some()) {
        record_activity(&state, &c).await;
        let result = respond(&state, &c, &activity, data).await;
        return match result {
            Ok(message) => Ok(Json(
                json!({"statusCode":200,"type":"application/vnd.microsoft.card.adaptive","value":{"type":"AdaptiveCard","version":"1.4","body":[{"type":"TextBlock","text":message,"wrap":true}]}}),
            )),
            Err(error) if error.status().is_client_error() => Ok(Json(
                json!({"statusCode":error.status().as_u16(),"type":"application/vnd.microsoft.error","value":{"code":"InvalidAction","message":error.public_message().unwrap_or("This action is not available to you")}}),
            )),
            Err(error) => Err(error),
        };
    }
    match gate(&activity, reads_messages(&c))? {
        Gate::Dispatch => record_activity(&state, &c).await,
        Gate::Ignore => return Ok(Json(json!({}))),
        Gate::ForgetTeam(team) => {
            if let Err(error) = microsoft::forget_team(&state, &c, &team).await {
                tracing::warn!(connection_id = %c.id, ?error, "Could not drop cached Teams team details");
            }
            return Ok(Json(json!({})));
        }
        Gate::Passive => {
            if let Err(error) = remember_passive(&state, &c, &activity).await {
                tracing::warn!(connection_id = %c.id, %error, "Could not add a Teams message to the conversation history");
            }
            return Ok(Json(json!({})));
        }
        Gate::Revise(revision) => {
            if let Err(error) = revise_history(&state, &c, &activity, revision).await {
                tracing::warn!(connection_id = %c.id, ?revision, %error, "Could not apply a changed Teams message to the conversation history");
            }
            return Ok(Json(json!({})));
        }
    }
    let conversation = string(&activity, "/conversation/id")?;
    let activity_id = string(&activity, "/id")?;
    let (user_id, _) = identity(&c.id, &activity)?;
    let run = hash(&[
        &c.id,
        &c.client_id,
        &c.customer_tenant_id,
        conversation,
        activity_id,
    ]);
    let scope = context::scope(&activity);
    let session = Session {
        connection_id: c.id.clone(),
        app_id: c.app_id.clone(),
        event_id: c.event_id.clone(),
        run_id: run.clone(),
        tenant_id: c.customer_tenant_id.clone(),
        client_id: c.client_id.clone(),
        conversation_id: conversation.into(),
        service_url: string(&activity, "/serviceUrl")?.into(),
        activity_id: activity_id.into(),
        user_id,
        bot_id: string(&activity, "/recipient/id")?.into(),
        history_key: history_key(&c, conversation),
        conversation_type: scope.conversation_type,
        team_id: scope.team_id,
        channel_id: scope.channel_id,
        thread_id: scope.thread_id,
        meeting_id: scope.meeting_id,
        permissions: c.permissions.clone(),
    };
    let Some(lease) = claim_dispatch(&state, &c, &run).await? else {
        return Ok(Json(json!({})));
    };
    let result = dispatch(&state, &c, sink, event, &activity, session).await;
    release_dispatch(&state, &run, lease, result.is_ok()).await;
    result.map(|()| Json(json!({})))
}

/// The setup page shows when the bot last handled a message or card action.
async fn record_activity(state: &AppState, c: &Connection) {
    if let Err(error) = save_activity(state, c).await {
        tracing::warn!(connection_id = %c.id, %error, "Could not record Teams bot activity");
    }
}

async fn save_activity(state: &AppState, c: &Connection) -> Result<(), ApiError> {
    let key = format!("health:{}", c.id);
    let stored = store::get::<Value>(state, &key).await?;
    if stored
        .as_ref()
        .is_some_and(|(health, _)| recent_activity(health, c, now()))
    {
        return Ok(());
    }
    let health = json!({"client_id":c.client_id,"tenant_id":c.customer_tenant_id,"at":now()});
    let expires = now() + HISTORY_MS;
    match stored {
        Some((_, revision)) => store::update_until(state, &key, &health, revision, expires).await?,
        None => store::insert(state, &key, &c.id, &health, expires).await?,
    };
    Ok(())
}

/// Busy bots write the health row at most once a minute.
fn recent_activity(health: &Value, c: &Connection, now: i64) -> bool {
    health["client_id"] == c.client_id
        && health["tenant_id"] == c.customer_tenant_id
        && health["at"]
            .as_i64()
            .is_some_and(|at| now - at < HEALTH_REFRESH_MS)
}

fn dispatch_claim(existing: &Dispatch, now: i64) -> Claim {
    match existing.status.as_str() {
        "dispatched" => Claim::Done,
        "dispatching" if existing.lease_until > now => Claim::InFlight,
        _ => Claim::Retry,
    }
}

/// Microsoft retries a message whose response was slow. The retry must neither start the
/// run a second time nor lose it when the first dispatch failed. Returns the lease revision,
/// or `None` when the message was already dispatched.
async fn claim_dispatch(
    state: &AppState,
    c: &Connection,
    run: &str,
) -> Result<Option<i32>, ApiError> {
    let key = format!("dispatch:{run}");
    let claim = Dispatch {
        status: "dispatching".into(),
        lease_until: now() + DISPATCH_LEASE_MS,
    };
    if store::insert(state, &key, &c.id, &claim, now() + DAY_MS).await? {
        return Ok(Some(0));
    }
    let in_flight = || ApiError::conflict("This Teams message is already being dispatched");
    let (existing, revision) = store::get::<Dispatch>(state, &key)
        .await?
        .ok_or_else(in_flight)?;
    match dispatch_claim(&existing, now()) {
        Claim::Done => Ok(None),
        Claim::InFlight => Err(in_flight()),
        Claim::Retry => {
            if store::update(state, &key, &claim, revision).await? {
                Ok(Some(revision + 1))
            } else {
                Err(in_flight())
            }
        }
    }
}

async fn release_dispatch(state: &AppState, run: &str, revision: i32, dispatched: bool) {
    let released = Dispatch {
        status: if dispatched { "dispatched" } else { "failed" }.into(),
        lease_until: 0,
    };
    match store::update(state, &format!("dispatch:{run}"), &released, revision).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(run_id = %run, "Teams dispatch lease changed before release"),
        Err(error) => {
            tracing::warn!(run_id = %run, %error, "Could not release the Teams dispatch lease")
        }
    }
}

async fn dispatch(
    state: &AppState,
    c: &Connection,
    sink: event_sink::Model,
    event: flow_like::flow::event::Event,
    activity: &Value,
    session: Session,
) -> Result<(), ApiError> {
    let run = session.run_id.clone();
    // A Microsoft retry keeps the deployment accepted with the first delivery.
    let event_key = format!("event:{run}");
    store::insert(state, &event_key, &c.id, &event, now() + DAY_MS).await?;
    let event = store::get::<flow_like::flow::event::Event>(state, &event_key)
        .await?
        .ok_or(ApiError::NOT_FOUND)?
        .0;
    store::insert(
        state,
        &format!("run:{run}"),
        &c.id,
        &session,
        now() + DAY_MS,
    )
    .await?;
    // Keep the original payload on retries, even if other messages arrived meanwhile.
    let payload_key = format!("input:{run}");
    let payload = match store::get::<Value>(state, &payload_key).await? {
        Some((payload, _)) => payload,
        None => {
            let payload = input_payload(state, c, &sink, activity, &session).await?;
            if store::insert(state, &payload_key, &c.id, &payload, now() + DAY_MS).await? {
                payload
            } else {
                store::get::<Value>(state, &payload_key)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?
                    .0
            }
        }
    };
    let result = trigger_event_with_run_id(
        state,
        sink,
        event,
        TriggerEventInput {
            event_id: c.event_id.clone(),
            payload: Some(payload),
            idempotency_key: Some(run.clone()),
        },
        run,
    )
    .await
    .map_err(ApiError::internal_error)?;
    if !result.triggered {
        return Err(ApiError::internal(
            "Teams workflow dispatch failed. Microsoft can retry this message.",
        ));
    }
    Ok(())
}

/// Builds the Chat Event payload once per message: it looks up context, downloads files and
/// persists the user's message to the conversation history before the run starts.
async fn input_payload(
    state: &AppState,
    c: &Connection,
    sink: &event_sink::Model,
    activity: &Value,
    session: &Session,
) -> Result<Value, ApiError> {
    let deadline = Instant::now() + PREPARE_BUDGET;
    // Renew credentials once, outside the lookups' timeouts, so no lookup starts a rotation.
    let c = &auth::fresh_connection(state, c)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(connection_id = %c.id, %error, "Could not renew the Teams bot credentials before preparing a message");
            c.clone()
        });
    let anonymous = session.user_id.starts_with(ANONYMOUS);
    let mut teams =
        Context::from_activity(activity, &session.run_id, &session.permissions, anonymous);
    let enrichment = async {
        let lookups = enrich(state, c, session, activity, teams.graph_group(), deadline);
        tokio::time::timeout_at(deadline, lookups)
            .await
            .unwrap_or_else(|_| {
                tracing::warn!(connection_id = %c.id, "Teams context lookups did not finish in time");
                Enrichment::default()
            })
    };
    let (enrichment, files) = tokio::join!(
        enrichment,
        files::collect(
            state,
            c,
            sink,
            &session.run_id,
            files::classify(activity),
            deadline
        ),
    );
    teams.enrich(enrichment);
    teams.set_files(files.iter().map(FileEntry::context).collect());
    let entry = user_entry(activity, &session.conversation_type, anonymous, &files);
    let conversation = update_history(state, &session.history_key, &c.id, |history| {
        record_invocation(history, &session.run_id, &entry)
    })
    .await?;
    let global = store::get::<UserSession>(state, &user_key(session))
        .await?
        .map(|v| v.0.value)
        .unwrap_or_default();
    Ok(payload(Invocation {
        chat_id: hash(&[&c.id, &session.conversation_id]),
        history: conversation.messages,
        entry,
        files: &files,
        local: conversation.local,
        global,
        teams: teams.into_value(),
        run: &session.run_id,
        sub: format!("{}:{}", c.customer_tenant_id, session.user_id),
        name: context::display_name(context::field(activity, "/from/name"), anonymous),
    }))
}

/// Each lookup gets `LOOKUP_TIMEOUT`, but never past the shared `deadline`.
async fn best_effort<T>(
    c: &Connection,
    deadline: Instant,
    lookup: &str,
    future: impl Future<Output = Result<T, MicrosoftError>>,
) -> Option<T> {
    let until = deadline.min(Instant::now() + LOOKUP_TIMEOUT);
    match tokio::time::timeout_at(until, future).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            tracing::warn!(connection_id = %c.id, lookup, ?error, "Teams context lookup failed");
            None
        }
        Err(_) => {
            tracing::warn!(connection_id = %c.id, lookup, "Teams context lookup timed out");
            None
        }
    }
}

/// Best-effort Microsoft lookups for `local_session.teams`; failures only leave fields out.
async fn enrich(
    state: &AppState,
    c: &Connection,
    session: &Session,
    activity: &Value,
    graph_group: Option<&str>,
    deadline: Instant,
) -> Enrichment {
    let details = session
        .permissions
        .contains(&TeamsPermission::ConversationDetails);
    let from = context::field(activity, "/from/id");
    let aad_object_id = context::field(activity, "/from/aadObjectId");
    let service_url = session.service_url.as_str();
    let member = async {
        let from = from.filter(|_| aad_object_id.is_some())?;
        best_effort(
            c,
            deadline,
            "member",
            microsoft::member(state, c, service_url, &session.conversation_id, from),
        )
        .await
    };
    let team = async {
        if session.conversation_type != "channel" || session.team_id.is_empty() {
            return (None, None, None, None);
        }
        let (team, channels) = tokio::join!(
            best_effort(
                c,
                deadline,
                "team",
                microsoft::team(state, c, service_url, &session.team_id)
            ),
            best_effort(
                c,
                deadline,
                "channels",
                microsoft::channels(state, c, service_url, &session.team_id)
            ),
        );
        let group = graph_group
            .map(str::to_owned)
            .or_else(|| team.as_ref().and_then(|team| team.aad_group_id.clone()))
            .filter(|_| details);
        let Some(group) = group else {
            return (team, channels, None, None);
        };
        let (graph_team, graph_channel) = tokio::join!(
            best_effort(
                c,
                deadline,
                "graph_team",
                microsoft::graph_team(state, c, &group)
            ),
            async {
                if session.channel_id.is_empty() {
                    return None;
                }
                best_effort(
                    c,
                    deadline,
                    "graph_channel",
                    microsoft::graph_channel(state, c, &group, &session.channel_id),
                )
                .await
            },
        );
        (team, channels, graph_team, graph_channel)
    };
    let meeting = async {
        if session.meeting_id.is_empty()
            || !session
                .permissions
                .contains(&TeamsPermission::MeetingDetails)
        {
            return None;
        }
        let mut meeting = best_effort(
            c,
            deadline,
            "meeting",
            microsoft::meeting(state, c, service_url, &session.meeting_id),
        )
        .await?;
        // The connector names the organizer by ID only. A sender who organized the meeting
        // is named from their own details instead.
        let organizer = meeting.organizer_id.clone().filter(|_| {
            meeting.organizer_name.is_none()
                && !context::organized_by(&meeting, from, aad_object_id)
        });
        if let Some(organizer) = organizer {
            meeting.organizer_name = best_effort(
                c,
                deadline,
                "organizer",
                microsoft::member(state, c, service_url, &session.conversation_id, &organizer),
            )
            .await
            .and_then(|member| member.name);
        }
        Some(meeting)
    };
    let meeting_role = async {
        let oid = aad_object_id.filter(|_| !session.meeting_id.is_empty())?;
        best_effort(
            c,
            deadline,
            "meeting_role",
            microsoft::meeting_role(state, c, service_url, &session.meeting_id, oid),
        )
        .await
        .flatten()
    };
    let chat = async {
        if !details || session.conversation_type != "groupChat" {
            return None;
        }
        best_effort(
            c,
            deadline,
            "graph_chat",
            microsoft::graph_chat(state, c, &session.conversation_id),
        )
        .await
    };
    let (member, (team, channels, graph_team, graph_channel), meeting, meeting_role, graph_chat) =
        tokio::join!(member, team, meeting, meeting_role, chat);
    Enrichment {
        member,
        team,
        channels,
        meeting,
        meeting_role,
        graph_team,
        graph_channel,
        graph_chat,
    }
}

fn quote_line(activity: &Value) -> Option<String> {
    context::quoted(activity)
        .as_ref()
        .and_then(context::quote_line)
}

/// The cleaned text, or `@{bot}` for a message that only @mentions the bot.
fn entry_text(activity: &Value) -> String {
    let text = context::clean_text(activity);
    if !text.is_empty() {
        return text;
    }
    context::bot_mention_name(activity)
        .map(|name| format!("@{name}"))
        .unwrap_or_default()
}

/// Files as history shows them before, or instead of, any download.
fn pending_files(activity: &Value) -> Vec<FileEntry> {
    files::classify(activity)
        .iter()
        .enumerate()
        .map(|(index, candidate)| FileEntry::pending(index, candidate))
        .collect()
}

fn has_content(activity: &Value, files: &[FileEntry]) -> bool {
    !files.is_empty() || !entry_text(activity).is_empty() || quote_line(activity).is_some()
}

/// A user message's history text: the author's name in front in group chats and channels,
/// a quoted reply first, and files as text placeholders.
fn entry_content(
    activity: &Value,
    conversation_type: &str,
    author: &str,
    files: &[FileEntry],
) -> String {
    let body = files::with_placeholders(&entry_text(activity), files);
    context::history_content(
        context::is_group(conversation_type).then_some(author),
        quote_line(activity).as_deref(),
        &body,
    )
}

/// A user message as history stores it.
fn user_entry(
    activity: &Value,
    conversation_type: &str,
    anonymous: bool,
    files: &[FileEntry],
) -> Value {
    let author = context::display_name(context::field(activity, "/from/name"), anonymous);
    let content = entry_content(activity, conversation_type, &author, files);
    let mut entry = json!({"role":"user","content":content,"author":author});
    if let Some(id) = context::field(activity, "/id") {
        entry["id"] = json!(id);
    }
    if let Some(at) = context::field(activity, "/timestamp") {
        entry["at"] = json!(at);
    }
    entry
}

/// A message the bot was not asked to answer, as conversation context. `None` when it carries
/// no text, quote or files.
fn passive_entry(activity: &Value) -> Option<Value> {
    let files = pending_files(activity);
    if !has_content(activity, &files) {
        return None;
    }
    let anonymous = context::field(activity, "/from/aadObjectId").is_none();
    let conversation_type = context::scope(activity).conversation_type;
    let mut entry = user_entry(activity, &conversation_type, anonymous, &files);
    entry["content"] = json!(context::cap(
        entry["content"].as_str().unwrap_or_default(),
        MAX_PASSIVE_BYTES
    ));
    Some(entry)
}

fn passive_key(activity_id: &str) -> String {
    format!("passive:{activity_id}")
}

async fn remember_passive(
    state: &AppState,
    c: &Connection,
    activity: &Value,
) -> Result<(), ApiError> {
    if !reads_messages(c) {
        return Ok(());
    }
    let conversation = string(activity, "/conversation/id")?;
    let key = passive_key(string(activity, "/id")?);
    let Some(entry) = passive_entry(activity) else {
        return Ok(());
    };
    update_history(state, &history_key(c, conversation), &c.id, |history| {
        record_passive(history, &key, &entry)
    })
    .await
    .map(drop)
}

fn record_passive(conversation: &mut Conversation, key: &str, entry: &Value) -> bool {
    if conversation.passive.iter().any(|known| known == key) {
        return false;
    }
    conversation.messages.push(entry.clone());
    trim_history(&mut conversation.messages);
    conversation.passive.push(key.into());
    keep_last(&mut conversation.passive, MAX_PASSIVE_KEYS);
    true
}

/// Applies a deleted or edited message to the history entry with its ID, if there is one.
async fn revise_history(
    state: &AppState,
    c: &Connection,
    activity: &Value,
    revision: Revision,
) -> Result<(), ApiError> {
    let conversation = string(activity, "/conversation/id")?;
    let id = string(activity, "/id")?;
    update_history(
        state,
        &history_key(c, conversation),
        &c.id,
        |history| match revision {
            Revision::Delete => record_delete(history, id),
            Revision::Edit => record_edit(history, id, activity),
        },
    )
    .await
    .map(drop)
}

/// Removes the entry and keeps its passive key, so a retried delivery does not restore it.
fn record_delete(conversation: &mut Conversation, id: &str) -> bool {
    let before = conversation.messages.len();
    conversation.messages.retain(|message| message["id"] != id);
    if conversation.messages.len() == before {
        return false;
    }
    let key = passive_key(id);
    if !conversation.passive.contains(&key) {
        conversation.passive.push(key);
        keep_last(&mut conversation.passive, MAX_PASSIVE_KEYS);
    }
    true
}

/// Rewrites the entry's content from the edited message, keeping its ID, author and time.
fn record_edit(conversation: &mut Conversation, id: &str, activity: &Value) -> bool {
    let passive = conversation.passive.contains(&passive_key(id));
    let Some(entry) = conversation
        .messages
        .iter_mut()
        .find(|message| message["id"] == id)
    else {
        return false;
    };
    let files = pending_files(activity);
    if !has_content(activity, &files) {
        return false;
    }
    let author = match entry["author"].as_str() {
        Some(author) => author.to_owned(),
        None => context::display_name(
            context::field(activity, "/from/name"),
            context::field(activity, "/from/aadObjectId").is_none(),
        ),
    };
    let conversation_type = context::scope(activity).conversation_type;
    let limit = if passive {
        MAX_PASSIVE_BYTES
    } else {
        MAX_MESSAGE_BYTES
    };
    let content = context::cap(
        &entry_content(activity, &conversation_type, &author, &files),
        limit,
    );
    if entry["content"] == content {
        return false;
    }
    entry["content"] = json!(content);
    true
}

fn timestamp(entry: &Value) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    entry["at"]
        .as_str()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
}

/// Persists the invoking message before the run starts, so replies attach after it. It goes
/// before any later message that arrived first; entries without a time count as earlier.
fn record_invocation(conversation: &mut Conversation, run: &str, entry: &Value) -> bool {
    if conversation.completed_runs.iter().any(|done| done == run) {
        return false;
    }
    let position = timestamp(entry)
        .and_then(|at| {
            conversation
                .messages
                .iter()
                .position(|message| timestamp(message).is_some_and(|other| other > at))
        })
        .unwrap_or(conversation.messages.len());
    conversation.messages.insert(position, entry.clone());
    trim_history(&mut conversation.messages);
    conversation.completed_runs.push(run.into());
    keep_last(&mut conversation.completed_runs, MAX_COMPLETED_RUNS);
    true
}

/// Applies `change` with compare-and-swap, retrying when another writer wins.
async fn update_history(
    state: &AppState,
    key: &str,
    connection_id: &str,
    mut change: impl FnMut(&mut Conversation) -> bool,
) -> Result<Conversation, ApiError> {
    let expires = now() + HISTORY_MS;
    for _ in 0..4 {
        let previous = store::get::<Conversation>(state, key).await?;
        let (mut conversation, revision) = previous.map(|(c, r)| (c, Some(r))).unwrap_or_default();
        if !change(&mut conversation) {
            return Ok(conversation);
        }
        let saved = if let Some(revision) = revision {
            store::update_until(state, key, &conversation, revision, expires).await?
        } else {
            store::insert(state, key, connection_id, &conversation, expires).await?
        };
        if saved {
            return Ok(conversation);
        }
    }
    Err(ApiError::conflict(
        "Teams conversation changed while it was being saved. Retry shortly.",
    ))
}

struct Invocation<'a> {
    chat_id: String,
    /// The persisted conversation, which already contains `entry`.
    history: Vec<Value>,
    entry: Value,
    files: &'a [FileEntry],
    local: Value,
    global: Value,
    teams: Value,
    run: &'a str,
    sub: String,
    name: String,
}

fn object_or_empty(value: Value) -> Value {
    if value.is_object() { value } else { json!({}) }
}

/// The Chat Event payload. The invoking message is last, with downloaded files as media parts.
fn payload(invocation: Invocation) -> Value {
    let Invocation {
        chat_id,
        history,
        entry,
        files,
        local,
        global,
        teams,
        run,
        sub,
        name,
    } = invocation;
    let mut messages: Vec<Value> = history
        .into_iter()
        .filter(|message| entry["id"].is_null() || message["id"] != entry["id"])
        .collect();
    let mut last = entry;
    if let Some(parts) = files::content_parts(last["content"].as_str().unwrap_or_default(), files) {
        last["content"] = parts;
    }
    messages.push(last);
    let mut local = object_or_empty(local);
    local["teams"] = teams;
    let mut global = object_or_empty(global);
    global["teams"] = json!({"session_id": run});
    let attachments = files
        .iter()
        .filter_map(FileEntry::attachment)
        .collect::<Vec<_>>();
    json!({
        "chat_id": chat_id,
        "messages": messages,
        "local_session": local,
        "global_session": global,
        "user": {"sub": sub, "name": name, "bot": false},
        "actions": [],
        "tools": [],
        "attachments": attachments
    })
}

fn user_key(s: &Session) -> String {
    format!(
        "user:{}",
        hash(&[&s.connection_id, &s.client_id, &s.tenant_id, &s.user_id])
    )
}

fn keep_last<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
}

/// Keeps the newest `MAX_HISTORY` entries whose serialized list fits `MAX_HISTORY_BYTES`,
/// always keeping the newest entry.
fn trim_history(messages: &mut Vec<Value>) {
    keep_last(messages, MAX_HISTORY);
    let sizes = messages
        .iter()
        .map(|message| message.to_string().len() + 1)
        .collect::<Vec<_>>();
    let mut total = sizes.iter().sum::<usize>() + 1;
    let mut dropped = 0;
    while total > MAX_HISTORY_BYTES && dropped + 1 < sizes.len() {
        total -= sizes[dropped];
        dropped += 1;
    }
    messages.drain(..dropped);
}

/// History never stores media: a parts array keeps only its text.
fn stored_message(message: &Value) -> Value {
    let mut message = message.clone();
    if let Some(parts) = message["content"].as_array() {
        let text = parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        message["content"] = json!(text);
    }
    message
}

fn activity_url(
    session: &Session,
    message: Option<&str>,
    reply: bool,
) -> Result<reqwest::Url, ApiError> {
    let mut url = auth::service_url(&session.service_url)?;
    let mut path = url
        .path_segments_mut()
        .map_err(|_| ApiError::UNAUTHORIZED)?;
    path.pop_if_empty().extend([
        "v3",
        "conversations",
        &session.conversation_id,
        "activities",
    ]);
    if let Some(id) = message {
        path.push(id);
    } else if reply {
        path.push(&session.activity_id);
    }
    drop(path);
    Ok(url)
}

/// What a repeated request gets: the original message ID, permission to send again after a
/// definite rejection, a wait while the first attempt runs, or a refusal when the first
/// attempt may have reached Teams.
fn delivery_retry(existing: &Delivery, digest: &str, now: i64) -> Result<Option<String>, ApiError> {
    if existing.hash != digest {
        return Err(ApiError::unprocessable(
            "This Teams request ID was already used for a different message",
        ));
    }
    match existing.status.as_str() {
        "sent" => existing.message_id.clone().map(Some).ok_or_else(|| {
            ApiError::unprocessable("Teams accepted this message without returning its ID")
        }),
        "rejected" => Ok(None),
        "sending" if existing.lease_until > now => Err(ApiError::conflict(
            "This Teams message is still being delivered. Retry shortly.",
        )),
        _ => Err(ApiError::unprocessable(
            "Teams did not confirm an earlier attempt of this message. It may already be in the conversation, so it is not sent again.",
        )),
    }
}

fn teams_rejection(status: StatusCode, retry_after: Option<u64>) -> ApiError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ApiError::too_many_requests(format!(
            "Teams is throttling this bot. Retry after {} seconds.",
            retry_after.unwrap_or(1)
        ));
    }
    if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
        return ApiError::bad_gateway(format!("Teams returned HTTP {status}. Retry shortly."));
    }
    ApiError::unprocessable(format!(
        "Teams rejected the message (HTTP {status}). Check that the bot is installed in this conversation and its credentials are valid."
    ))
}

async fn settle(state: &AppState, id: &str, delivery: &Delivery, revision: i32, status: &str) {
    let settled = Delivery {
        status: status.into(),
        lease_until: 0,
        ..delivery.clone()
    };
    match store::update(state, id, &settled, revision).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(delivery = %id, status, "Teams delivery changed before it was settled")
        }
        Err(error) => {
            tracing::warn!(delivery = %id, status, %error, "Could not record the Teams delivery outcome")
        }
    }
}

/// Sends at most once per `key` and run. Deterministic failures are 4xx, 409 means the same
/// delivery is still running, and 429/5xx are transient.
async fn deliver(
    state: &AppState,
    c: &Connection,
    session: &Session,
    key: &str,
    body: Value,
    update: Option<&str>,
) -> Result<String, ApiError> {
    let encoded = body.to_string();
    if encoded.len() > MAX_MESSAGE_BYTES {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "Teams responses may contain at most 24000 bytes",
        ));
    }
    let url = activity_url(session, update, true)?;
    let id = format!("delivery:{}", hash(&[&session.run_id, key]));
    let sending = Delivery {
        status: "sending".into(),
        message_id: None,
        hash: hash(&[&encoded, update.unwrap_or_default()]),
        lease_until: now() + DELIVERY_LEASE_MS,
    };
    let mut revision = 0;
    if !store::insert(state, &id, &c.id, &sending, now() + DAY_MS).await? {
        let in_flight =
            || ApiError::conflict("This Teams message is still being delivered. Retry shortly.");
        let (existing, previous) = store::get::<Delivery>(state, &id)
            .await?
            .ok_or_else(in_flight)?;
        if let Some(message) = delivery_retry(&existing, &sending.hash, now())? {
            return Ok(message);
        }
        if !store::update(state, &id, &sending, previous).await? {
            return Err(in_flight());
        }
        revision = previous + 1;
    }
    let token = match auth::bot_token(state, c).await {
        Ok(token) => token,
        Err(error) => {
            settle(state, &id, &sending, revision, "rejected").await;
            return Err(error);
        }
    };
    let method = if update.is_some() {
        reqwest::Method::PUT
    } else {
        reqwest::Method::POST
    };
    let response = match client()?
        .request(method, url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_connect() => {
            settle(state, &id, &sending, revision, "rejected").await;
            return Err(ApiError::bad_gateway(
                "Teams could not be reached. Retry shortly.",
            ));
        }
        Err(_) => {
            settle(state, &id, &sending, revision, "uncertain").await;
            return Err(ApiError::unprocessable(
                "Teams did not confirm this message in time. It may already be in the conversation, so it is not sent again.",
            ));
        }
    };
    let status = response.status();
    if !status.is_success() {
        settle(state, &id, &sending, revision, "rejected").await;
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            auth::forget_bot_token(c);
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        return Err(teams_rejection(status, retry_after));
    }
    let message = response
        .bytes()
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["id"].as_str().map(str::to_owned))
        .or_else(|| update.map(str::to_owned));
    let sent = Delivery {
        status: "sent".into(),
        message_id: message.clone(),
        lease_until: 0,
        ..sending
    };
    match store::update(state, &id, &sent, revision).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(delivery = %id, "Teams delivery changed after the message was sent")
        }
        Err(error) => {
            tracing::warn!(delivery = %id, %error, "Could not record a sent Teams message")
        }
    }
    message.ok_or_else(|| {
        ApiError::unprocessable("Teams accepted this message without returning its ID")
    })
}

pub(super) async fn session_for_run(
    state: &AppState,
    app_id: &str,
    run_id: &str,
) -> Result<Option<(Session, Connection, ExecutionRunRecord)>, ApiError> {
    let Some((session, _)) = store::get::<Session>(state, &format!("run:{}", run_id)).await? else {
        return Ok(None);
    };
    if session.app_id != app_id {
        return Err(ApiError::FORBIDDEN);
    }
    let run = get_state_store(state)
        .await?
        .get_run_for_app(run_id, app_id)
        .await
        .map_err(|_| ApiError::internal("Cannot verify Teams execution"))?
        .ok_or(ApiError::FORBIDDEN)?;
    if run.event_id.as_deref() != Some(&session.event_id)
        || run.expires_at.is_some_and(|expiry| expiry <= now())
    {
        return Err(ApiError::FORBIDDEN);
    }
    if run.shadow_of_run_id.is_some()
        || run.regression_run_id.is_some()
        || matches!(
            run.run_variant,
            crate::execution::state::RunVariant::Shadow
                | crate::execution::state::RunVariant::Regression
        )
    {
        return Err(ApiError::FORBIDDEN);
    }
    let (c, _) = store::connection(state, &session.connection_id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    live(state, &c).await?;
    if c.customer_tenant_id != session.tenant_id || c.client_id != session.client_id {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(Some((session, c, run)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamsSendRequest {
    /// Run ID from the Teams event's session.
    session_id: String,
    /// Idempotency key of 1–128 characters. Repeating it returns the first message's ID.
    request_id: String,
    /// Markdown text.
    #[serde(default)]
    text: Option<String>,
    /// Adaptive Card JSON.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    card: Option<Value>,
    /// Replace this earlier message instead of sending a new one.
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TeamsSendResponse {
    message_id: String,
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/teams/send",
    operation_id = "send_teams_message",
    tag = "execution",
    description = "Send or update a message in the Teams conversation that started this run. Executor only; retry only 409, 429 and 5xx responses.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = TeamsSendRequest,
    responses(
        (status = 200, description = "Delivered. A repeated request ID returns the original message ID.", body = TeamsSendResponse),
        (status = 400, description = "Invalid request, text or card"),
        (status = 403, description = "The caller is not this app's running Teams execution"),
        (status = 409, description = "The same request is still being delivered"),
        (status = 413, description = "The message exceeds 24000 bytes"),
        (status = 422, description = "Teams rejected the message, or an earlier attempt may already have been delivered"),
        (status = 429, description = "Teams is throttling the bot"),
        (status = 502, description = "Teams or Microsoft authentication failed transiently")
    ),
    security(("executor_jwt" = []))
)]
pub async fn send(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app): Path<String>,
    Json(input): Json<TeamsSendRequest>,
) -> Result<Json<TeamsSendResponse>, ApiError> {
    let AppUser::Executor(claims) = user else {
        return Err(ApiError::FORBIDDEN);
    };
    if claims.app_id != app || claims.run_id != input.session_id {
        return Err(ApiError::FORBIDDEN);
    }
    if input.request_id.is_empty() || input.request_id.len() > 128 {
        return Err(ApiError::bad_request(
            "Provide a Teams request ID of 1–128 characters",
        ));
    }
    let (session, c, run) = session_for_run(&state, &claims.app_id, &claims.run_id)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if run.status.is_terminal() {
        return Err(ApiError::forbidden("This Teams execution has ended"));
    }
    let text = input.text.filter(|v| !v.trim().is_empty());
    let mut body = json!({"type":"message","textFormat":"markdown"});
    if let Some(text) = &text {
        body["text"] = json!(text);
    }
    if let Some(card) = input.card {
        if card["type"] != "AdaptiveCard" {
            return Err(ApiError::bad_request("Provide an Adaptive Card"));
        }
        body["attachments"] =
            json!([{"contentType":"application/vnd.microsoft.card.adaptive","content":card}]);
    }
    if body.get("text").is_none() && body.get("attachments").is_none() {
        return Err(ApiError::bad_request(
            "Provide message text or an Adaptive Card",
        ));
    }
    let update = input.message_id.as_deref().filter(|s| !s.is_empty());
    let key = format!("node:{}", input.request_id);
    let message_id = deliver(&state, &c, &session, &key, body, update).await?;
    if update.is_none()
        && let Some(text) = &text
        && let Err(error) = remember(&state, &session, &key, text, None).await
    {
        tracing::warn!(run_id = %session.run_id, %error, "Could not add a Teams reply to the conversation history");
    }
    Ok(Json(TeamsSendResponse { message_id }))
}

pub(crate) async fn output(
    state: &AppState,
    claims: &ExecutionClaims,
    events: &[ExecutionEventInput],
) -> Result<(), ApiError> {
    if claims.event_id.is_none()
        || !is_teams_run(&claims.run_id)
        || !events
            .iter()
            .any(|e| matches!(e.event_type.as_str(), "chat_out" | "interaction_request"))
    {
        return Ok(());
    }
    let Some((session, c, run)) = session_for_run(state, &claims.app_id, &claims.run_id).await?
    else {
        return Ok(());
    };
    for event in events {
        match event.event_type.as_str() {
            "interaction_request" => {
                let request: InteractionRequest = serde_json::from_value(event.payload.clone())
                    .map_err(|_| ApiError::bad_request("Invalid Teams interaction request"))?;
                let channel = request.channel.as_ref().ok_or_else(|| {
                    ApiError::bad_request("Teams interactions require a reply channel")
                })?;
                if channel.channel_id != claims.run_id
                    || channel.request_id.as_deref() != Some(&request.id)
                    || request.expires_at <= (now() / 1000) as u64
                    || request.run_id.as_deref() != Some(&claims.run_id)
                    || request.app_id.as_deref() != Some(&claims.app_id)
                {
                    return Err(ApiError::FORBIDDEN);
                }
                let id = hash(&[&c.id, &claims.run_id, &request.id]);
                let action = Action {
                    session: session.clone(),
                    request: request.clone(),
                    allowed_responders: if c.allowed_responders.is_empty() {
                        vec![session.user_id.clone()]
                    } else {
                        c.allowed_responders.clone()
                    },
                    state: "pending".into(),
                    lease_until: 0,
                    answer: None,
                    responder: None,
                };
                store::insert(
                    state,
                    &format!("action:{id}"),
                    &c.id,
                    &action,
                    action_expiry(request.expires_at, run.expires_at, now()),
                )
                .await?;
                let card = cards::render(&request, &id)?;
                deliver(
                    state,
                    &c,
                    &session,
                    &format!("interaction:{}", request.id),
                    json!({"type":"message","attachments":[{"contentType":"application/vnd.microsoft.card.adaptive","content":card}]}),
                    None,
                )
                .await?;
            }
            "chat_out" => {
                let text = response_text(&event.payload);
                if !text.is_empty() {
                    deliver(
                        state,
                        &c,
                        &session,
                        "chat_out",
                        json!({"type":"message","text":text,"textFormat":"markdown"}),
                        None,
                    )
                    .await?;
                }
                remember(state, &session, "chat_out", &text, Some(&event.payload)).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// The executor chooses an Interaction's expiry; approval state never outlives the run.
fn action_expiry(requested_secs: u64, run_expires_at: Option<i64>, now: i64) -> i64 {
    i64::try_from(requested_secs)
        .unwrap_or(i64::MAX)
        .saturating_mul(1000)
        .min(run_expires_at.unwrap_or(i64::MAX))
        .min(now + DAY_MS)
}

fn response_text(payload: &Value) -> String {
    let message = &payload["response"]["choices"][0]["message"];
    if let Some(text) = message["content"].as_str().filter(|s| !s.is_empty()) {
        return text.into();
    }
    message["content_parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Adds one reply to the history. The run's user message is normally persisted at dispatch;
/// runs dispatched before that come with it here. `entry` makes retried replies no-ops.
/// Returns whether anything changed.
fn record_reply(
    conversation: &mut Conversation,
    run: &str,
    entry: &str,
    user_message: Option<&Value>,
    text: &str,
    local: Option<&Value>,
) -> bool {
    if conversation
        .recorded
        .iter()
        .any(|recorded| recorded == entry)
    {
        return false;
    }
    if !conversation.completed_runs.iter().any(|done| done == run) {
        if let Some(message) = user_message {
            conversation.messages.push(stored_message(message));
        }
        conversation.completed_runs.push(run.into());
        keep_last(&mut conversation.completed_runs, MAX_COMPLETED_RUNS);
    }
    if !text.is_empty() {
        conversation
            .messages
            .push(json!({"role":"assistant","content":text}));
    }
    trim_history(&mut conversation.messages);
    if let Some(local) = local {
        conversation.local = local.clone();
    }
    conversation.last_run = run.into();
    conversation.recorded.push(entry.into());
    keep_last(&mut conversation.recorded, MAX_RECORDED_REPLIES);
    true
}

/// Records a reply sent by `chat_out` or the send endpoint. Only `chat_out` carries the
/// sessions the workflow returned, so only it updates them.
async fn remember(
    state: &AppState,
    s: &Session,
    key: &str,
    text: &str,
    sessions: Option<&Value>,
) -> Result<(), ApiError> {
    let input = store::get::<Value>(state, &format!("input:{}", s.run_id))
        .await?
        .map(|v| v.0)
        .unwrap_or_default();
    let local = sessions.map(|payload| {
        let mut local = payload
            .get("local_session")
            .filter(|value| value.is_object())
            .unwrap_or(&input["local_session"])
            .clone();
        if let Some(local) = local.as_object_mut() {
            local.remove("teams");
        }
        local
    });
    let user_message = input["messages"]
        .as_array()
        .and_then(|messages| messages.last());
    let entry = hash(&[&s.run_id, key]);
    update_history(state, &s.history_key, &s.connection_id, |conversation| {
        record_reply(
            conversation,
            &s.run_id,
            &entry,
            user_message,
            text,
            local.as_ref(),
        )
    })
    .await?;
    let expires = now() + HISTORY_MS;
    if let Some(global) = sessions
        .and_then(|payload| payload.get("global_session"))
        .filter(|v| v.is_object())
    {
        let mut global = global.clone();
        if let Some(map) = global.as_object_mut() {
            map.remove("teams");
        }
        let key = user_key(s);
        for attempt in 0..4 {
            let previous = store::get::<UserSession>(state, &key).await?;
            let (mut session, revision) = previous.map(|(s, r)| (s, Some(r))).unwrap_or_default();
            if session.completed_runs.contains(&s.run_id) {
                break;
            }
            session.value = global.clone();
            session.completed_runs.push(s.run_id.clone());
            keep_last(&mut session.completed_runs, MAX_COMPLETED_RUNS);
            let saved = if let Some(revision) = revision {
                store::update_until(state, &key, &session, revision, expires).await?
            } else {
                store::insert(state, &key, &s.connection_id, &session, expires).await?
            };
            if saved {
                break;
            }
            if attempt == 3 {
                return Err(ApiError::conflict(
                    "Teams user session changed. Retry saving this response.",
                ));
            }
        }
    }
    Ok(())
}

/// Anonymous meeting guests have no Entra identity, so they can never answer a request.
fn responder(activity: &Value) -> Result<String, ApiError> {
    let oid = context::field(activity, "/from/aadObjectId").ok_or_else(|| {
        ApiError::forbidden(
            "Sign in with a Microsoft work or school account to answer this request.",
        )
    })?;
    super::guid(oid)
}

async fn respond(
    state: &AppState,
    c: &Connection,
    activity: &Value,
    data: &Value,
) -> Result<&'static str, ApiError> {
    let id = data["flow_like_action"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(ApiError::FORBIDDEN)?;
    let key = format!("action:{id}");
    let (mut action, revision) = store::get::<Action>(state, &key)
        .await?
        .ok_or_else(|| ApiError::gone("This request has expired"))?;
    let sender = responder(activity)?;
    if action.session.connection_id != c.id
        || action.session.tenant_id != c.customer_tenant_id
        || action.session.client_id != c.client_id
        || string(activity, "/conversation/id")? != action.session.conversation_id
        || !action.allowed_responders.contains(&sender)
        || (if c.allowed_responders.is_empty() {
            sender != action.session.user_id
        } else {
            !c.allowed_responders.contains(&sender)
        })
    {
        return Err(ApiError::forbidden(
            "This approval is assigned to another person or conversation",
        ));
    }
    if action.state == "responded" {
        return Ok("This request was already answered.");
    }
    if action.lease_until > now() {
        return Err(ApiError::conflict(
            "This answer is being processed. Try again shortly.",
        ));
    }
    let answer = cards::response(&action.request, data)?;
    if action
        .answer
        .as_ref()
        .is_some_and(|previous| previous != &answer)
        || action
            .responder
            .as_ref()
            .is_some_and(|previous| previous != &sender)
    {
        return Err(ApiError::conflict(
            "Another answer has already been submitted",
        ));
    }
    let run = get_state_store(state)
        .await?
        .get_run_for_app(&action.session.run_id, &c.app_id)
        .await
        .map_err(|_| ApiError::internal("Cannot verify the waiting execution"))?
        .ok_or(ApiError::NOT_FOUND)?;
    if run.status.is_terminal() || run.expires_at.is_some_and(|expiry| expiry <= now()) {
        return Err(ApiError::gone("The workflow is no longer waiting"));
    }
    action.state = "submitting".into();
    action.lease_until = now() + 30000;
    action.answer = Some(answer.clone());
    action.responder = Some(sender);
    if !store::update(state, &key, &action, revision).await? {
        return Err(ApiError::conflict("Another answer is being processed"));
    }
    let handle = action.request.channel.as_ref().ok_or(ApiError::FORBIDDEN)?;
    let descriptor = match &handle.transport {
        ChannelClientDescriptor::Http { .. } => &handle.transport,
        _ => handle.fallback.as_ref().ok_or(ApiError::FORBIDDEN)?,
    };
    let ChannelClientDescriptor::Http { token, .. } = descriptor else {
        return Err(ApiError::FORBIDDEN);
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}")
            .parse()
            .map_err(|_| ApiError::FORBIDDEN)?,
    );
    let body = ChannelPush {
        channel_id: handle.channel_id.clone(),
        request_id: Some(action.request.id.clone()),
        kind: ChannelPushKind::Reply,
        value: answer,
    };
    let result = crate::routes::channel::push_channel(
        State(state.clone()),
        headers,
        Path(handle.channel_id.clone()),
        Json(body),
    )
    .await;
    action.lease_until = 0;
    let outcome = match result {
        Ok(Json(result))
            if result.accepted
                || result.message.as_deref() == Some("This request was already answered") =>
        {
            action.state = "responded".into();
            Ok("Your response was submitted.")
        }
        Ok(_) => Err(ApiError::gone("The workflow is no longer waiting")),
        Err(error) => Err(error),
    };
    // A rejected or refused push stored nothing, so another answer may follow. A server
    // error leaves the push's outcome unknown and keeps the submitted answer binding.
    if outcome
        .as_ref()
        .is_err_and(|error| error.status().is_client_error())
    {
        action.state = "pending".into();
        action.answer = None;
        action.responder = None;
    }
    store::update(state, &key, &action, revision + 1).await?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(status: &str, message_id: Option<&str>, lease_until: i64) -> Delivery {
        Delivery {
            status: status.into(),
            message_id: message_id.map(Into::into),
            hash: "digest".into(),
            lease_until,
        }
    }

    fn connection() -> Connection {
        serde_json::from_value(json!({
            "id":"c", "app_id":"app", "event_id":"event", "mode":"customer_teams",
            "name":"Bot", "description":"", "customer_tenant_id":"tenant", "home_tenant_id":"home",
            "client_id":"bot-id", "secret":"s", "graph_object_id":null, "azure_resource_id":null,
            "secret_key_id":null, "secret_expires_at":null, "status":"ready", "allowed_responders":[]
        }))
        .unwrap()
    }

    #[test]
    fn identities_are_scoped_and_unambiguous() {
        assert_ne!(hash(&["ab", "c"]), hash(&["a", "bc"]));
        assert_ne!(
            hash(&["tenant-a", "message"]),
            hash(&["tenant-b", "message"])
        );
    }

    #[test]
    fn only_activity_hashes_are_teams_runs() {
        assert!(is_teams_run(&hash(&["connection", "activity"])));
        assert!(!is_teams_run("clx7q2k3b0000abcdefghijkl"));
        assert!(!is_teams_run(&"g".repeat(64)));
        assert!(!is_teams_run(""));
    }

    #[test]
    fn retried_messages_dispatch_once_and_recover_failed_dispatches() {
        let at = |status: &str, lease_until| Dispatch {
            status: status.into(),
            lease_until,
        };
        assert_eq!(dispatch_claim(&at("dispatched", 0), 10), Claim::Done);
        assert_eq!(dispatch_claim(&at("dispatching", 11), 10), Claim::InFlight);
        assert_eq!(dispatch_claim(&at("dispatching", 10), 10), Claim::Retry);
        assert_eq!(dispatch_claim(&at("failed", 0), 10), Claim::Retry);
    }

    #[test]
    fn repeated_deliveries_follow_executor_retry_semantics() {
        assert_eq!(
            delivery_retry(&delivery("sent", Some("message"), 0), "digest", 10).unwrap(),
            Some("message".into())
        );
        assert_eq!(
            delivery_retry(&delivery("rejected", None, 0), "digest", 10).unwrap(),
            None
        );
        let status = |result: Result<Option<String>, ApiError>| result.unwrap_err().status();
        assert_eq!(
            status(delivery_retry(&delivery("sending", None, 11), "digest", 10)),
            StatusCode::CONFLICT
        );
        for uncertain in [
            delivery("sending", None, 10),
            delivery("uncertain", None, 0),
        ] {
            assert_eq!(
                status(delivery_retry(&uncertain, "digest", 10)),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        assert_eq!(
            status(delivery_retry(&delivery("sent", None, 0), "digest", 10)),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status(delivery_retry(&delivery("sent", Some("m"), 0), "other", 10)),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn teams_rejections_separate_transient_from_deterministic_failures() {
        for (teams, api) in [
            (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS),
            (StatusCode::INTERNAL_SERVER_ERROR, StatusCode::BAD_GATEWAY),
            (StatusCode::SERVICE_UNAVAILABLE, StatusCode::BAD_GATEWAY),
            (StatusCode::REQUEST_TIMEOUT, StatusCode::BAD_GATEWAY),
            (StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY),
            (StatusCode::FORBIDDEN, StatusCode::UNPROCESSABLE_ENTITY),
            (StatusCode::NOT_FOUND, StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            assert_eq!(teams_rejection(teams, None).status(), api, "{teams}");
        }
        assert!(
            teams_rejection(StatusCode::TOO_MANY_REQUESTS, Some(7))
                .public_message()
                .is_some_and(|message| message.contains("7 seconds"))
        );
    }

    #[test]
    fn interaction_state_never_outlives_the_run_or_a_day() {
        let now = 1_000_000;
        assert_eq!(action_expiry(1_100, None, now), 1_100_000);
        assert_eq!(action_expiry(u64::MAX, None, now), now + DAY_MS);
        assert_eq!(action_expiry(u64::MAX, Some(now + 5), now), now + 5);
    }

    #[test]
    fn replies_record_the_user_message_once_and_ignore_retries() {
        let mut conversation = Conversation::default();
        let user = json!({"role":"user","content":"hi"});
        let local = json!({"topic":"billing"});
        assert!(record_reply(
            &mut conversation,
            "run",
            "node:1",
            Some(&user),
            "first",
            None
        ));
        assert!(!record_reply(
            &mut conversation,
            "run",
            "node:1",
            Some(&user),
            "first",
            None
        ));
        assert!(record_reply(
            &mut conversation,
            "run",
            "chat_out",
            Some(&user),
            "final",
            Some(&local)
        ));
        assert_eq!(
            conversation.messages,
            vec![
                user,
                json!({"role":"assistant","content":"first"}),
                json!({"role":"assistant","content":"final"}),
            ]
        );
        assert_eq!(conversation.local, local);
        assert_eq!(conversation.completed_runs, vec!["run".to_string()]);
    }

    #[test]
    fn history_keeps_only_the_latest_messages() {
        let mut conversation = Conversation::default();
        for index in 0..30 {
            let user = json!({"role":"user","content":index});
            record_reply(
                &mut conversation,
                &format!("run-{index}"),
                &format!("entry-{index}"),
                Some(&user),
                "reply",
                None,
            );
        }
        assert_eq!(conversation.messages.len(), MAX_HISTORY);
        assert_eq!(
            conversation.messages.last(),
            Some(&json!({"role":"assistant","content":"reply"}))
        );
    }

    const BOT: &str = "28:bot";
    const OID: &str = "9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c";

    fn message(conversation_type: &str, text: &str) -> Value {
        json!({
            "type": "message",
            "id": "1727",
            "timestamp": "2026-09-28T09:00:00Z",
            "text": text,
            "serviceUrl": "https://smba.trafficmanager.net/emea/",
            "from": {"id": "29:felix", "aadObjectId": OID, "name": "Felix Schultz"},
            "recipient": {"id": BOT, "name": "Flow Bot"},
            "conversation": {"id": "19:c@thread.v2", "conversationType": conversation_type},
            "entities": [{"type":"mention","text":"<at>Flow Bot</at>","mentioned":{"id":BOT}}]
        })
    }

    fn unmentioned(conversation_type: &str, text: &str) -> Value {
        let mut activity = message(conversation_type, text);
        activity["entities"] = json!([]);
        activity
    }

    fn quoting(mut activity: Value) -> Value {
        activity["entities"].as_array_mut().unwrap().push(
            json!({"type":"quotedReply","quotedReply":{"senderName":"Anna","preview":"preview"}}),
        );
        activity
    }

    #[test]
    fn only_addressed_messages_with_content_dispatch() {
        let gate = |activity: &Value| gate(activity, true).unwrap();
        assert_eq!(gate(&message("personal", "hello")), Gate::Dispatch);
        assert_eq!(gate(&unmentioned("personal", "hello")), Gate::Dispatch);
        assert_eq!(
            gate(&message("channel", "<at>Flow Bot</at> hi")),
            Gate::Dispatch
        );
        assert_eq!(gate(&unmentioned("groupChat", "hi all")), Gate::Passive);
        assert_eq!(gate(&unmentioned("channel", "hi all")), Gate::Passive);
        let mut image = message("personal", "");
        image["attachments"] = json!([{"contentType":"image/*","contentUrl":"https://smba.trafficmanager.net/emea/v3/attachments/a/views/original"}]);
        assert_eq!(gate(&image), Gate::Dispatch);
        image["attachments"] = json!([{"contentType":"text/html","content":"<p></p>"}]);
        assert_eq!(gate(&image), Gate::Ignore);
    }

    #[test]
    fn bare_mentions_and_quotes_dispatch_but_empty_personal_messages_do_not() {
        let gate = |activity: &Value| gate(activity, false).unwrap();
        assert_eq!(
            gate(&message("groupChat", "<at>Flow Bot</at> ")),
            Gate::Dispatch
        );
        assert_eq!(
            gate(&message("channel", "<at>Flow Bot</at>")),
            Gate::Dispatch
        );
        assert_eq!(
            gate(&message("personal", "<at>Flow Bot</at> ")),
            Gate::Ignore
        );
        assert_eq!(gate(&unmentioned("personal", " ")), Gate::Ignore);
        assert_eq!(gate(&quoting(unmentioned("personal", ""))), Gate::Dispatch);
        assert_eq!(
            gate(&quoting(unmentioned(
                "personal",
                "<quoted messageId=\"1\"/>"
            ))),
            Gate::Dispatch
        );
        let mut blank_quote = unmentioned("personal", "");
        blank_quote["entities"] =
            json!([{"type":"quotedReply","quotedReply":{"senderName":"Anna","preview":" "}}]);
        assert_eq!(gate(&blank_quote), Gate::Ignore);
    }

    #[test]
    fn bare_mentions_and_quotes_become_readable_history() {
        let bare = message("groupChat", "<at>Flow Bot</at>");
        assert_eq!(
            user_entry(&bare, "groupChat", false, &[])["content"],
            "Felix Schultz: @Flow Bot"
        );
        assert_eq!(
            user_entry(&quoting(bare), "groupChat", false, &[])["content"],
            "Felix Schultz: > Anna: preview\n\n@Flow Bot"
        );
        assert_eq!(
            user_entry(
                &quoting(unmentioned("personal", "")),
                "personal",
                false,
                &[]
            )["content"],
            "> Anna: preview"
        );
        let mut named = message("channel", "<at>Flow Bot</at>");
        named["entities"][0]["mentioned"]["name"] = json!("Flow\u{200b} Assistant");
        assert_eq!(
            user_entry(&named, "channel", false, &[])["content"],
            "Felix Schultz: @Flow Assistant"
        );
    }

    #[test]
    fn other_peoples_messages_need_the_read_permission_but_changes_always_apply() {
        let deleted = json!({"type":"messageDelete","id":"1727","channelData":{"eventType":"softDeleteMessage"}});
        let edited = json!({"type":"messageUpdate","id":"1727","text":"new","channelData":{"eventType":"editMessage"}});
        let passive = unmentioned("channel", "hi all");
        assert_eq!(gate(&passive, true).unwrap(), Gate::Passive);
        assert_eq!(gate(&passive, false).unwrap(), Gate::Ignore);
        for (activity, revision) in [
            (deleted.clone(), Gate::Revise(Revision::Delete)),
            (edited.clone(), Gate::Revise(Revision::Edit)),
        ] {
            assert_eq!(gate(&activity, true).unwrap(), revision);
            assert_eq!(gate(&activity, false).unwrap(), revision);
        }
        let undeleted = json!({"type":"messageUpdate","id":"1727","channelData":{"eventType":"undeleteMessage"}});
        assert_eq!(gate(&undeleted, true).unwrap(), Gate::Ignore);
        let untyped_delete = json!({"type":"messageDelete","id":"1727"});
        assert_eq!(
            gate(&untyped_delete, true).unwrap(),
            Gate::Revise(Revision::Delete)
        );
        let mut c = connection();
        assert!(!reads_messages(&c));
        c.permissions = vec![
            TeamsPermission::MeetingDetails,
            TeamsPermission::ReadMessages,
        ];
        assert!(reads_messages(&c));
    }

    #[test]
    fn bots_and_other_activities_never_dispatch() {
        let gate = |activity: &Value| gate(activity, true).unwrap();
        let mut bot = message("personal", "hello");
        bot["from"] = json!({"id":"28:other"});
        assert_eq!(gate(&bot), Gate::Ignore);
        bot["from"] = json!({"id":"29:x","aadObjectId":OID,"role":"bot"});
        assert_eq!(gate(&bot), Gate::Ignore);
        let mut own = message("personal", "hello");
        own["from"] = json!({"id":BOT});
        assert_eq!(gate(&own), Gate::Ignore);
        let update = json!({"type":"conversationUpdate","channelData":{"team":{"id":"19:team"}}});
        assert_eq!(gate(&update), Gate::ForgetTeam("19:team".into()));
        let install = json!({"type":"installationUpdate","channelData":{"team":{"id":"19:team"}}});
        assert_eq!(gate(&install), Gate::ForgetTeam("19:team".into()));
        assert_eq!(gate(&json!({"type":"conversationUpdate"})), Gate::Ignore);
        assert_eq!(
            gate(&json!({"type":"typing","channelData":{"team":{"id":"19:team"}}})),
            Gate::Ignore
        );
    }

    #[test]
    fn oversized_text_is_rejected_only_when_addressed() {
        let long = "x".repeat(MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            gate(&message("personal", &long), true)
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            gate(&unmentioned("channel", &long), true).unwrap(),
            Gate::Passive
        );
        let entry = passive_entry(&unmentioned("channel", &long)).unwrap();
        let content = entry["content"].as_str().unwrap();
        assert!(content.len() <= MAX_PASSIVE_BYTES);
        assert!(content.starts_with("Felix Schultz: xxx"));
        assert!(content.ends_with('…'));
    }

    #[test]
    fn anonymous_guests_get_a_stable_pseudonym_that_is_never_a_guid() {
        let (user, anonymous) = identity("c", &message("groupChat", "hi")).unwrap();
        assert_eq!((user.as_str(), anonymous), (OID, false));
        let mut guest = message("groupChat", "hi");
        guest["from"] = json!({"id":"29:guest","name":"Visitor"});
        let (first, anonymous) = identity("c", &guest).unwrap();
        assert!(anonymous);
        assert_eq!(first, format!("{ANONYMOUS}{}", hash(&["c", "29:guest"])));
        assert!(uuid::Uuid::parse_str(&first).is_err());
        assert_eq!(identity("c", &guest).unwrap().0, first);
        assert_ne!(identity("other", &guest).unwrap().0, first);
        guest["from"]["aadObjectId"] = json!("not-a-guid");
        assert_eq!(
            identity("c", &guest).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
        let error = responder(&json!({"from":{"id":"29:guest"}})).unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
        assert!(
            error
                .public_message()
                .is_some_and(|message| message.contains("work or school account"))
        );
        assert_eq!(responder(&message("personal", "")).unwrap(), OID);
    }

    #[test]
    fn group_history_names_the_author_and_personal_history_does_not() {
        let mut activity = message("groupChat", "<at>Flow Bot</at> what did <at>Anna</at> say?");
        activity["entities"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"quotedReply","quotedReply":{"senderName":"Anna","preview":"Ship it\nFriday"}}));
        let image = FileEntry {
            name: "a.png".into(),
            mime: "image/png".into(),
            url: Some("https://store/a.png".into()),
            downloadable: true,
            ..FileEntry::default()
        };
        assert_eq!(
            user_entry(&activity, "groupChat", false, &[image]),
            json!({
                "role": "user",
                "content": "Felix Schultz: > Anna: Ship it Friday\n\nwhat did @Anna say?\n[image: a.png]",
                "author": "Felix Schultz",
                "id": "1727",
                "at": "2026-09-28T09:00:00Z"
            })
        );
        assert_eq!(
            user_entry(&message("personal", "hello"), "personal", false, &[])["content"],
            "hello"
        );
        let mut guest = unmentioned("channel", "hi");
        guest["from"] = json!({"id":"29:guest"});
        let entry = passive_entry(&guest).unwrap();
        assert_eq!(entry["content"], "Guest: hi");
        assert_eq!(entry["author"], "Guest");
        let mut file_only = unmentioned("channel", "");
        file_only["attachments"] = json!([{"contentType":"reference","name":"Plan.docx","contentUrl":"https://contoso.sharepoint.com/Plan.docx"}]);
        assert_eq!(
            passive_entry(&file_only).unwrap()["content"],
            "Felix Schultz: [file: Plan.docx]"
        );
        assert_eq!(passive_entry(&unmentioned("channel", " ")), None);
    }

    #[test]
    fn passive_messages_are_recorded_once() {
        let mut conversation = Conversation::default();
        let entry = json!({"role":"user","content":"Anna: hi","id":"1"});
        assert!(record_passive(&mut conversation, "passive:1", &entry));
        assert!(!record_passive(&mut conversation, "passive:1", &entry));
        assert!(record_passive(
            &mut conversation,
            "passive:2",
            &json!({"role":"user","content":"Anna: again","id":"2"})
        ));
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.passive, vec!["passive:1", "passive:2"]);
        assert!(conversation.recorded.is_empty());
        assert!(conversation.completed_runs.is_empty());
    }

    #[test]
    fn busy_channels_do_not_evict_reply_keys() {
        let mut conversation = Conversation::default();
        assert!(record_reply(
            &mut conversation,
            "run",
            "reply",
            None,
            "answer",
            None
        ));
        for index in 0..MAX_PASSIVE_KEYS + 100 {
            let entry = json!({"role":"user","content":"Anna: hi","id":index.to_string()});
            assert!(record_passive(
                &mut conversation,
                &passive_key(&index.to_string()),
                &entry
            ));
        }
        assert_eq!(conversation.passive.len(), MAX_PASSIVE_KEYS);
        assert_eq!(conversation.passive[0], passive_key("100"));
        assert_eq!(conversation.recorded, vec!["reply"]);
        assert!(!record_reply(
            &mut conversation,
            "run",
            "reply",
            None,
            "answer",
            None
        ));
    }

    #[test]
    fn conversations_saved_before_the_passive_list_still_load() {
        let conversation: Conversation = serde_json::from_value(json!({
            "messages": [{"role":"user","content":"hi"}],
            "local": {},
            "last_run": "run",
            "completed_runs": ["run"],
            "recorded": ["passive:1", "reply"]
        }))
        .unwrap();
        assert!(conversation.passive.is_empty());
        assert_eq!(conversation.recorded.len(), 2);
    }

    fn history(entries: &[(&str, &str)]) -> Conversation {
        let mut conversation = Conversation::default();
        for (id, content) in entries {
            record_passive(
                &mut conversation,
                &passive_key(id),
                &json!({"role":"user","content":content,"author":"Anna","id":id,"at":"2026-09-28T09:00:00Z"}),
            );
        }
        conversation
    }

    #[test]
    fn deleted_messages_leave_the_history_for_good() {
        let mut conversation = history(&[("1", "Anna: first"), ("2", "Anna: second")]);
        assert!(record_delete(&mut conversation, "1"));
        assert_eq!(conversation.messages.len(), 1);
        assert_eq!(conversation.messages[0]["id"], "2");
        assert!(!record_delete(&mut conversation, "1"));
        assert!(!record_passive(
            &mut conversation,
            &passive_key("1"),
            &json!({"role":"user","content":"Anna: first","id":"1"})
        ));
        assert_eq!(conversation.messages.len(), 1);

        let mut invoked = Conversation::default();
        record_invocation(
            &mut invoked,
            "run",
            &json!({"role":"user","content":"Felix: hi","id":"9"}),
        );
        assert!(record_delete(&mut invoked, "9"));
        assert!(invoked.messages.is_empty());
        assert_eq!(invoked.passive, vec![passive_key("9")]);
        assert!(!record_delete(&mut Conversation::default(), "9"));
    }

    fn edit(conversation_type: &str, id: &str, text: &str) -> Value {
        let mut activity = unmentioned(conversation_type, text);
        activity["type"] = json!("messageUpdate");
        activity["id"] = json!(id);
        activity["from"]["name"] = json!("Anna Renamed");
        activity["timestamp"] = json!("2026-09-28T10:00:00Z");
        activity["channelData"] = json!({"eventType":"editMessage"});
        activity
    }

    #[test]
    fn edited_messages_are_rewritten_in_place() {
        let mut conversation = history(&[("1", "Anna: first"), ("2", "Anna: second")]);
        assert!(record_edit(
            &mut conversation,
            "1",
            &quoting(edit("channel", "1", "first, fixed"))
        ));
        assert_eq!(
            conversation.messages[0],
            json!({"role":"user","content":"Anna: > Anna: preview\n\nfirst, fixed","author":"Anna","id":"1","at":"2026-09-28T09:00:00Z"})
        );
        assert_eq!(conversation.messages[1]["content"], "Anna: second");
        assert!(!record_edit(
            &mut conversation,
            "1",
            &quoting(edit("channel", "1", "first, fixed"))
        ));
        assert!(!record_edit(
            &mut conversation,
            "3",
            &edit("channel", "3", "x")
        ));
        assert!(!record_edit(
            &mut conversation,
            "2",
            &edit("channel", "2", " ")
        ));

        let long = "y".repeat(MAX_MESSAGE_BYTES);
        assert!(record_edit(
            &mut conversation,
            "2",
            &edit("channel", "2", &long)
        ));
        let content = conversation.messages[1]["content"].as_str().unwrap();
        assert!(content.len() <= MAX_PASSIVE_BYTES && content.ends_with('…'));

        let mut invoked = Conversation::default();
        record_invocation(
            &mut invoked,
            "run",
            &json!({"role":"user","content":"hi","author":"Felix","id":"9"}),
        );
        assert!(record_edit(
            &mut invoked,
            "9",
            &edit("personal", "9", &long)
        ));
        assert_eq!(invoked.messages[0]["content"], long);
        assert_eq!(invoked.messages[0]["author"], "Felix");
    }

    #[test]
    fn the_invoking_message_is_stored_in_time_order() {
        let mut conversation = history(&[("0", "Anna: before")]);
        conversation.messages[0]["at"] = json!("2026-09-28T08:59:00Z");
        record_passive(
            &mut conversation,
            &passive_key("2"),
            &json!({"role":"user","content":"Anna: after","id":"2","at":"2026-09-28T11:00:05.000+02:00"}),
        );
        conversation
            .messages
            .push(json!({"role":"assistant","content":"reply"}));
        let entry =
            json!({"role":"user","content":"Felix: hi","id":"1","at":"2026-09-28T09:00:00Z"});
        assert!(record_invocation(&mut conversation, "run", &entry));
        let ids = conversation
            .messages
            .iter()
            .map(|message| message["id"].as_str().unwrap_or("reply"))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["0", "1", "2", "reply"]);

        let untimed = json!({"role":"user","content":"Felix: again","id":"3"});
        assert!(record_invocation(&mut conversation, "later", &untimed));
        assert_eq!(conversation.messages.last(), Some(&untimed));

        let payload = payload(Invocation {
            chat_id: "chat".into(),
            history: conversation.messages.clone(),
            entry: entry.clone(),
            files: &[],
            local: Value::Null,
            global: Value::Null,
            teams: json!({"session_id":"run"}),
            run: "run",
            sub: "tenant:user".into(),
            name: "Felix".into(),
        });
        assert_eq!(payload["messages"].as_array().unwrap().last(), Some(&entry));
    }

    #[test]
    fn health_is_rewritten_at_most_once_a_minute_per_bot_identity() {
        let c = connection();
        let now = 10 * HEALTH_REFRESH_MS;
        let health = |client: &str, tenant: &str, at: i64| json!({"client_id":client,"tenant_id":tenant,"at":at});
        assert!(recent_activity(
            &health("bot-id", "tenant", now - 1_000),
            &c,
            now
        ));
        assert!(!recent_activity(
            &health("bot-id", "tenant", now - HEALTH_REFRESH_MS),
            &c,
            now
        ));
        assert!(!recent_activity(
            &health("other-bot", "tenant", now),
            &c,
            now
        ));
        assert!(!recent_activity(
            &health("bot-id", "other-tenant", now),
            &c,
            now
        ));
        assert!(!recent_activity(&json!({}), &c, now));
    }

    #[tokio::test(start_paused = true)]
    async fn lookups_stop_at_the_shared_deadline() {
        let c = connection();
        let hung = std::future::pending::<Result<(), MicrosoftError>>;
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        assert_eq!(best_effort(&c, deadline, "slow", hung()).await, None);
        assert_eq!(Instant::now() - start, Duration::from_secs(1));
        let start = Instant::now();
        let far = start + Duration::from_secs(60);
        assert_eq!(best_effort(&c, far, "slow", hung()).await, None);
        assert_eq!(Instant::now() - start, LOOKUP_TIMEOUT);
        assert_eq!(
            best_effort(&c, far, "quick", async { Ok(7) }).await,
            Some(7)
        );
        assert_eq!(
            best_effort::<()>(&c, far, "failed", async { Err(MicrosoftError::NotFound) }).await,
            None
        );
    }

    #[test]
    fn the_invoking_message_is_persisted_once_and_replies_do_not_repeat_it() {
        let mut conversation = Conversation::default();
        let entry = json!({"role":"user","content":"Felix: hi","id":"1727"});
        assert!(record_invocation(&mut conversation, "run", &entry));
        assert!(!record_invocation(&mut conversation, "run", &entry));
        record_passive(
            &mut conversation,
            "passive:1728",
            &json!({"role":"user","content":"Anna: meanwhile","id":"1728"}),
        );
        let with_parts = json!({"role":"user","content":[{"type":"text","text":"Felix: hi"},{"type":"image_url","image_url":{"url":"u"}}]});
        assert!(record_reply(
            &mut conversation,
            "run",
            "chat_out",
            Some(&with_parts),
            "answer",
            None
        ));
        assert_eq!(
            conversation.messages,
            vec![
                entry,
                json!({"role":"user","content":"Anna: meanwhile","id":"1728"}),
                json!({"role":"assistant","content":"answer"}),
            ]
        );
        let mut legacy = Conversation::default();
        record_reply(&mut legacy, "old", "chat_out", Some(&with_parts), "a", None);
        assert_eq!(
            legacy.messages[0],
            json!({"role":"user","content":"Felix: hi"})
        );
    }

    #[test]
    fn history_is_trimmed_by_count_and_size_keeping_the_newest() {
        let mut messages = (0..40)
            .map(|index| json!({"role":"user","content":index.to_string()}))
            .collect::<Vec<_>>();
        trim_history(&mut messages);
        assert_eq!(messages.len(), MAX_HISTORY);
        assert_eq!(messages[0]["content"], "10");
        let big = "x".repeat(60_000);
        let mut messages = (0..5)
            .map(|index| json!({"role":"user","content":format!("{index}{big}")}))
            .collect::<Vec<_>>();
        trim_history(&mut messages);
        assert_eq!(messages.len(), 3);
        assert!(messages[0]["content"].as_str().unwrap().starts_with('2'));
        assert!(serde_json::to_vec(&messages).unwrap().len() <= MAX_HISTORY_BYTES);
        let mut huge = vec![json!({"role":"user","content":"y".repeat(MAX_HISTORY_BYTES)})];
        trim_history(&mut huge);
        assert_eq!(huge.len(), 1);
    }

    #[test]
    fn payloads_end_with_the_invoking_message_and_its_media() {
        let entry = json!({"role":"user","content":"Felix: look\n[image: a.png]\n[file: Plan.docx]","author":"Felix","id":"1727"});
        let files = [
            FileEntry {
                name: "a.png".into(),
                mime: "image/png".into(),
                size: Some(3),
                url: Some("https://store/a.png?sig=1".into()),
                path: Some("tmp/a.png".into()),
                downloadable: true,
                ..FileEntry::default()
            },
            FileEntry {
                name: "Plan.docx".into(),
                mime: "application/octet-stream".into(),
                link: Some("https://contoso.sharepoint.com/Plan.docx".into()),
                error: Some("not downloadable".into()),
                ..FileEntry::default()
            },
        ];
        let history = vec![
            json!({"role":"user","content":"Anna: earlier","id":"1700"}),
            entry.clone(),
            json!({"role":"user","content":"Anna: meanwhile","id":"1728"}),
        ];
        let payload = payload(Invocation {
            chat_id: "chat".into(),
            history,
            entry: entry.clone(),
            files: &files,
            local: json!({"topic":"billing"}),
            global: Value::Null,
            teams: json!({"session_id":"run"}),
            run: "run",
            sub: format!("tenant:{ANONYMOUS}abc"),
            name: "Guest".into(),
        });
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["id"], "1700");
        assert_eq!(messages[1]["id"], "1728");
        assert_eq!(messages[2]["id"], "1727");
        assert_eq!(
            messages[2]["content"],
            json!([
                {"type":"text","text":"Felix: look\n[image: a.png]\n[file: Plan.docx]"},
                {"type":"image_url","image_url":{"url":"https://store/a.png?sig=1","media_type":"image/png"}}
            ])
        );
        assert_eq!(
            payload["attachments"],
            json!([{"url":"https://store/a.png?sig=1","name":"a.png","type":"image/png","size":3}])
        );
        assert_eq!(
            payload["local_session"],
            json!({"topic":"billing","teams":{"session_id":"run"}})
        );
        assert_eq!(
            payload["global_session"],
            json!({"teams":{"session_id":"run"}})
        );
        assert_eq!(
            payload["user"],
            json!({"sub":"tenant:anonymous:abc","name":"Guest","bot":false})
        );
        assert_eq!(payload["chat_id"], "chat");
        assert_eq!(payload["actions"], json!([]));
        assert_eq!(payload["tools"], json!([]));

        let plain = super::payload(Invocation {
            chat_id: "chat".into(),
            history: vec![entry.clone()],
            entry: entry.clone(),
            files: &files[1..],
            local: json!("not an object"),
            global: json!({"plan":"pro","teams":{"stale":true}}),
            teams: json!({"session_id":"run"}),
            run: "run",
            sub: "tenant:user".into(),
            name: "Felix".into(),
        });
        assert_eq!(plain["messages"], json!([entry]));
        assert_eq!(plain["attachments"], json!([]));
        assert_eq!(
            plain["local_session"],
            json!({"teams":{"session_id":"run"}})
        );
        assert_eq!(
            plain["global_session"],
            json!({"plan":"pro","teams":{"session_id":"run"}})
        );
    }
}
