//! Scheduled calls use separate participants, chat and peer rooms. Guest session credentials
//! are read only here and never become account or inbox credentials.

pub mod model;
mod store;
#[cfg(test)]
mod tests;

use crate::{
    backend_jwt,
    db::coordination::coordinate,
    entity::{meeting, user},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{active_membership, now_millis, presence},
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use model::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_meeting))
        .route("/{id}/preview", post(preview))
        .route("/{id}/invite", get(current_invite).post(rotate_invite))
        .route("/{id}/join", post(join))
        .route("/{id}/guest", post(guest))
        .route("/{id}/session", get(session))
        .route("/{id}/realtime", post(realtime))
        .route("/{id}/messages", post(send))
        .route("/{id}/leave", post(leave))
        .route("/{id}/end", post(end))
        .route("/{id}/participants/{participant_id}/admit", post(admit))
        .route("/{id}/participants/{participant_id}/remove", post(remove))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
}

async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

fn person(user: &AppUser) -> Result<String, ApiError> {
    if !matches!(user, AppUser::OpenID(_) | AppUser::PAT(_)) {
        return Err(ApiError::UNAUTHORIZED);
    }
    Ok(user.sub()?)
}

fn credential(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-flow-meeting-session")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() <= 180)
        .map(str::to_string)
}

fn required_credential(headers: &HeaderMap) -> Result<String, ApiError> {
    credential(headers).ok_or(ApiError::UNAUTHORIZED)
}

async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MeetingList>, ApiError> {
    let sub = person(&user)?;
    active_membership(&state.db, &query.conversation_id, &sub).await?;
    let now = now_millis();
    let rows = meeting::Entity::find()
        .filter(meeting::Column::ConversationId.eq(query.conversation_id))
        .order_by_desc(meeting::Column::StartsAt)
        .limit(100)
        .all(&state.db)
        .await?;
    Ok(Json(MeetingList {
        meetings: rows.iter().map(|r| Meeting::of(r, now)).collect(),
    }))
}

async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<CreateMeeting>,
) -> Result<Json<CreatedMeeting>, ApiError> {
    let sub = person(&user)?;
    let row = state
        .transaction(|txn| {
            let (sub, request) = (sub.clone(), request.clone());
            Box::pin(async move {
                coordinate(txn, "meeting-create", &[&request.conversation_id]).await?;
                store::create_in(txn, &sub, &request, now_millis()).await
            })
        })
        .await?;
    Ok(Json(CreatedMeeting {
        invite_token: store::invite_token(&state.encryption_key, &row),
        meeting: Meeting::of(&row, now_millis()),
    }))
}

async fn get_meeting(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Meeting>, ApiError> {
    let sub = person(&user)?;
    let row = store::find(&state.db, &id).await?;
    store::member(&state.db, &row, &sub).await?;
    Ok(Json(Meeting::of(&row, now_millis())))
}

async fn preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<MeetingPreview>, ApiError> {
    let row = store::find(&state.db, &id).await?;
    let invited = request
        .invite_token
        .as_deref()
        .is_some_and(|token| store::valid_invite(&state.encryption_key, &row, token));
    if !invited {
        let sub = person(&user).map_err(|_| ApiError::NOT_FOUND)?;
        store::member(&state.db, &row, &sub).await?;
    }
    Ok(Json(Meeting::of(&row, now_millis()).into()))
}

async fn current_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<InviteToken>, ApiError> {
    let sub = person(&user).ok();
    let token = credential(&headers);
    let row = store::find(&state.db, &id).await?;
    store::host_in(&state.db, &row, sub.as_deref(), token.as_deref()).await?;
    Ok(Json(InviteToken {
        invite_token: store::invite_token(&state.encryption_key, &row),
    }))
}

async fn rotate_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<InviteToken>, ApiError> {
    let sub = person(&user).ok();
    let token = credential(&headers);
    let row = state
        .transaction(|txn| {
            let (id, sub, token) = (id.clone(), sub.clone(), token.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                store::host_in(txn, &row, sub.as_deref(), token.as_deref()).await?;
                let mut row = row.into_active_model();
                row.invite_nonce = Set(store::random_secret()?);
                Ok::<_, ApiError>(row.update(txn).await?)
            })
        })
        .await?;
    Ok(Json(InviteToken {
        invite_token: store::invite_token(&state.encryption_key, &row),
    }))
}

async fn join(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Joined>, ApiError> {
    let sub = person(&user)?;
    let name = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .and_then(|u| u.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Member".to_string());
    let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    let joined = state
        .transaction(|txn| {
            let (id, sub, name) = (id.clone(), sub.clone(), name.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                store::member(txn, &row, &sub).await?;
                store::reap_in(txn, &row, now_millis()).await?;
                let row = store::find(txn, &id).await?;
                store::join_in(txn, &row, Some(&sub), &name, now_millis()).await
            })
        })
        .await?;
    Ok(Json(joined))
}

async fn guest(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<GuestJoin>,
) -> Result<Json<Joined>, ApiError> {
    let key = state.encryption_key;
    let joined = state
        .transaction(|txn| {
            let (id, request) = (id.clone(), request.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                if !store::valid_invite(&key, &row, &request.invite_token) {
                    return Err(ApiError::NOT_FOUND);
                }
                store::reap_in(txn, &row, now_millis()).await?;
                let row = store::find(txn, &id).await?;
                store::join_in(txn, &row, None, &request.display_name, now_millis()).await
            })
        })
        .await?;
    Ok(Json(joined))
}

async fn session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Session>, ApiError> {
    let token = required_credential(&headers)?;
    let snapshot = state
        .transaction(|txn| {
            let (id, token) = (id.clone(), token.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                let participant = store::session_in(txn, &row, &token).await?;
                let now = now_millis();
                let participant = store::heartbeat_in(txn, &row, participant, now).await?;
                let row = store::find(txn, &id).await?;
                store::reap_in(txn, &row, now).await?;
                let row = store::find(txn, &id).await?;
                store::snapshot(txn, &row, &participant, now).await
            })
        })
        .await?;
    Ok(Json(snapshot))
}

async fn realtime(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Realtime>, ApiError> {
    let token = required_credential(&headers)?;
    let row = store::find(&state.db, &id).await?;
    let participant = store::session_in(&state.db, &row, &token).await?;
    let now = now_millis();
    store::admitted(&row, &participant, now)?;
    let room_id = format!("meeting-{}-{}", row.id, row.room_epoch);
    let key_id = format!("meeting-v1-{}", row.room_epoch);
    let jti = flow_like_types::create_id();
    let mut claims = presence::claims_for(
        participant.id.clone(),
        Some(participant.display_name),
        &room_id,
        jti.clone(),
    );
    // A token never outlives the meeting. Clients poll admission and retire old rooms when the
    // epoch changes; signaling also disconnects expired tokens independently of the UI.
    claims.exp = (now.timestamp() + 90).min(row.ends_at.timestamp());
    let expires_at = claims.exp;
    if expires_at <= now.timestamp() {
        return Err(ApiError::gone("This meeting has ended"));
    }
    let issued = state
        .realtime_ice
        .issue_bounded(
            &format!("meeting:{}:{}:{}", row.id, row.room_epoch, participant.id),
            &jti,
            (expires_at - now.timestamp()) as u32,
            now.timestamp(),
        )
        .await
        .map_err(|_| ApiError::bad_gateway("Meeting connection servers are unavailable"))?;
    let (ice_servers, ice_expires_at) = issued
        .map(|i| (Some(i.ice_servers), Some(i.expires_at)))
        .unwrap_or((None, None));
    let jwt = backend_jwt::sign(&claims).map_err(|e| {
        ApiError::internal_error(flow_like_types::anyhow!(
            "Meeting realtime is not configured: {e}"
        ))
    })?;
    Ok(Json(Realtime {
        access: presence::ConversationRealtimeParams {
            jwt,
            encryption_key: presence::derive_room_key(&state.encryption_key, &room_id, &key_id),
            key_id,
            ice_servers,
            ice_expires_at,
        },
        room_id,
        expires_at,
        expires_in: expires_at - now.timestamp(),
        server_time: now,
        signaling_servers: state.platform_config.signaling.clone().unwrap_or_default(),
    }))
}

async fn send(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SendMessage>,
) -> Result<Json<Message>, ApiError> {
    let token = required_credential(&headers)?;
    let message = state
        .transaction(|txn| {
            let (id, token, body) = (id.clone(), token.clone(), request.body.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                let participant = store::session_in(txn, &row, &token).await?;
                store::send_in(txn, &row, &participant, &body, now_millis()).await
            })
        })
        .await?;
    Ok(Json(message))
}

async fn leave(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = required_credential(&headers)?;
    state
        .transaction(|txn| {
            let (id, token) = (id.clone(), token.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                let participant = store::session_in(txn, &row, &token).await?;
                store::leave_in(txn, &row, participant).await
            })
        })
        .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn end(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sub = person(&user).ok();
    let token = credential(&headers);
    state
        .transaction(|txn| {
            let (id, sub, token) = (id.clone(), sub.clone(), token.clone());
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                store::host_in(txn, &row, sub.as_deref(), token.as_deref()).await?;
                store::end_in(txn, &row, now_millis()).await
            })
        })
        .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn change_participant(
    state: &AppState,
    user: &AppUser,
    id: String,
    participant_id: String,
    headers: HeaderMap,
    accept: bool,
) -> Result<Json<Participant>, ApiError> {
    let sub = person(user).ok();
    let token = credential(&headers);
    let participant = state
        .transaction(|txn| {
            let (id, participant_id, sub, token) = (
                id.clone(),
                participant_id.clone(),
                sub.clone(),
                token.clone(),
            );
            Box::pin(async move {
                coordinate(txn, "meeting", &[&id]).await?;
                let row = store::find(txn, &id).await?;
                store::host_in(txn, &row, sub.as_deref(), token.as_deref()).await?;
                store::reap_in(txn, &row, now_millis()).await?;
                let row = store::find(txn, &id).await?;
                store::change_participant_in(txn, &row, &participant_id, accept, now_millis()).await
            })
        })
        .await?;
    Ok(Json(participant))
}

async fn admit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, participant_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Participant>, ApiError> {
    change_participant(&state, &user, id, participant_id, headers, true).await
}

async fn remove(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, participant_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Participant>, ApiError> {
    change_participant(&state, &user, id, participant_id, headers, false).await
}
