use super::{
    INBOX_KEY_EXISTS, INBOX_KEY_LIMIT,
    model::{InboxKey, InboxKeyList, KeyOwnerQuery, KeysQuery, RegisterInboxKeyRequest},
    now_millis,
};
use crate::{
    audit, db::coordination::coordinate, entity::inbox_key, error::ApiError,
    middleware::jwt::AppUser, routes::bots::owned_bot, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, sea_query::Expr,
};
use std::collections::BTreeSet;

const KEY_ID_MAX_CHARS: usize = 128;
const ALGORITHM_MAX_CHARS: usize = 64;
const PUBLIC_KEY_MAX_CHARS: usize = 4_096;
/// Active keys per principal, so a key lookup stays small.
pub(super) const MAX_ACTIVE_KEYS: u64 = 20;
/// Principals per key lookup.
pub(super) const MAX_KEY_PRINCIPALS: usize = 50;

/// A key to register, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewKey {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
}

fn token(
    field: &str,
    value: &str,
    max_chars: usize,
    punctuation: &[u8],
) -> Result<String, ApiError> {
    let valid = (1..=max_chars).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || punctuation.contains(&byte));
    if !valid {
        let shown: String = value.chars().take(80).collect();
        return Err(ApiError::bad_request(format!(
            "{field} '{shown}' must be 1 to {max_chars} characters of letters, digits and {}",
            String::from_utf8_lossy(punctuation)
        )));
    }
    Ok(value.to_string())
}

pub(super) fn validate_key(request: &RegisterInboxKeyRequest) -> Result<NewKey, ApiError> {
    let public_key = request.public_key.trim();
    if public_key.is_empty()
        || public_key.len() > PUBLIC_KEY_MAX_CHARS
        || !public_key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(ApiError::bad_request(format!(
            "public_key must be 1 to {PUBLIC_KEY_MAX_CHARS} printable ASCII characters without spaces; got {} characters",
            public_key.chars().count()
        )));
    }
    Ok(NewKey {
        key_id: token("key_id", request.key_id.trim(), KEY_ID_MAX_CHARS, b"._:-")?,
        algorithm: token(
            "algorithm",
            request.algorithm.trim(),
            ALGORITHM_MAX_CHARS,
            b"._-",
        )?,
        public_key: public_key.to_string(),
    })
}

/// The principal ids of a key lookup: trimmed, deduplicated, at most [`MAX_KEY_PRINCIPALS`].
pub(super) fn requested_principals(principals: Option<&str>) -> Result<Vec<String>, ApiError> {
    let ids: BTreeSet<String> = principals
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if ids.len() > MAX_KEY_PRINCIPALS {
        return Err(ApiError::bad_request(format!(
            "Ask for at most {MAX_KEY_PRINCIPALS} principals at once; got {}",
            ids.len()
        )));
    }
    Ok(ids.into_iter().collect())
}

pub(super) async fn active_keys<C: ConnectionTrait>(
    db: &C,
    principals: &[String],
) -> Result<Vec<InboxKey>, ApiError> {
    if principals.is_empty() {
        return Ok(Vec::new());
    }
    Ok(inbox_key::Entity::find()
        .filter(inbox_key::Column::PrincipalId.is_in(principals.iter().cloned()))
        .filter(inbox_key::Column::RevokedAt.is_null())
        .order_by_asc(inbox_key::Column::PrincipalId)
        .order_by_asc(inbox_key::Column::CreatedAt)
        .order_by_asc(inbox_key::Column::KeyId)
        .all(db)
        .await?
        .into_iter()
        .map(InboxKey::from)
        .collect())
}

/// The stored key when `key` repeats it; other material under its id, or a revoked key, is a
/// conflict.
fn same_key(existing: inbox_key::Model, key: &NewKey) -> Result<inbox_key::Model, ApiError> {
    let same = existing.revoked_at.is_none()
        && existing.algorithm == key.algorithm
        && existing.public_key == key.public_key;
    if same {
        return Ok(existing);
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_KEY_EXISTS,
        format!(
            "Key '{}' of '{}' is already registered with other material or revoked; register a new key_id",
            key.key_id, existing.principal_id
        ),
    ))
}

/// Registers `key` for `principal`. Keys never change: the same key again returns the stored
/// one, different material under a used `key_id` is a conflict.
pub(super) async fn register_in<C: ConnectionTrait>(
    db: &C,
    principal: &str,
    key: &NewKey,
    now: DateTime<FixedOffset>,
) -> Result<inbox_key::Model, ApiError> {
    if let Some(existing) =
        inbox_key::Entity::find_by_id((principal.to_string(), key.key_id.clone()))
            .one(db)
            .await?
    {
        return same_key(existing, key);
    }
    let active = inbox_key::Entity::find()
        .filter(inbox_key::Column::PrincipalId.eq(principal))
        .filter(inbox_key::Column::RevokedAt.is_null())
        .count(db)
        .await?;
    if active >= MAX_ACTIVE_KEYS {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_KEY_LIMIT,
            format!("'{principal}' already has {MAX_ACTIVE_KEYS} active keys"),
        ));
    }
    Ok(inbox_key::ActiveModel {
        principal_id: Set(principal.to_string()),
        key_id: Set(key.key_id.clone()),
        algorithm: Set(key.algorithm.clone()),
        public_key: Set(key.public_key.clone()),
        created_at: Set(now),
        revoked_at: Set(None),
    }
    .insert(db)
    .await?)
}

/// Revokes `key_id` of `principal`: it is no longer listed, no longer counts toward
/// [`MAX_ACTIVE_KEYS`], and its id stays taken. True when this call revoked it; revoking a
/// revoked key changes nothing.
pub(super) async fn revoke_in<C: ConnectionTrait>(
    db: &C,
    principal: &str,
    key_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let revoked = inbox_key::Entity::update_many()
        .col_expr(inbox_key::Column::RevokedAt, Expr::value(now))
        .filter(inbox_key::Column::PrincipalId.eq(principal))
        .filter(inbox_key::Column::KeyId.eq(key_id))
        .filter(inbox_key::Column::RevokedAt.is_null())
        .exec(db)
        .await?;
    if revoked.rows_affected > 0 {
        return Ok(true);
    }
    inbox_key::Entity::find_by_id((principal.to_string(), key_id.to_string()))
        .one(db)
        .await?
        .map(|_| false)
        .ok_or_else(|| ApiError::not_found(format!("Key '{key_id}' of '{principal}' not found")))
}

/// Whose keys the caller manages: their own, or with `principal_id` one of their live bots'.
async fn key_owner<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    principal_id: Option<&str>,
) -> Result<String, ApiError> {
    match principal_id.map(str::trim) {
        None | Some("") => Ok(sub.to_string()),
        Some(id) if id == sub => Ok(sub.to_string()),
        Some(bot_id) => Ok(owned_bot(db, sub, bot_id).await?.id),
    }
}

#[utoipa::path(
    get,
    path = "/inbox/keys",
    tag = "Inbox",
    params(KeysQuery),
    responses(
        (status = 200, description = "The active public keys of these people and bots, to seal envelopes to", body = InboxKeyList),
        (status = 400, description = "More than 50 principals"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read inbox keys, not API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/keys", skip(state, user))]
pub async fn list_keys(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<KeysQuery>,
) -> Result<Json<InboxKeyList>, ApiError> {
    user.sub()?;
    let principals = requested_principals(query.principals.as_deref())?;
    Ok(Json(InboxKeyList {
        keys: active_keys(&state.db, &principals).await?,
    }))
}

#[utoipa::path(
    put,
    path = "/inbox/keys",
    tag = "Inbox",
    request_body = RegisterInboxKeyRequest,
    responses(
        (status = 200, description = "The registered key; registering the same key again returns it", body = InboxKey),
        (status = 400, description = "The key id, algorithm or public key is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people register inbox keys, not API keys or running flows"),
        (status = 404, description = "principal_id is not one of your bots"),
        (status = 409, description = "INBOX_KEY_EXISTS: the key id is taken by other material; INBOX_KEY_LIMIT: 20 active keys already")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /inbox/keys", skip(state, user, body))]
pub async fn register_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<RegisterInboxKeyRequest>,
) -> Result<Json<InboxKey>, ApiError> {
    let sub = user.sub()?;
    let key = validate_key(&body)?;
    let principal = key_owner(&state.db, &sub, body.principal_id.as_deref()).await?;
    let stored = state
        .transaction(|txn| {
            let (principal, key) = (principal.clone(), key.clone());
            Box::pin(async move {
                coordinate(txn, "inbox-keys", &[&principal]).await?;
                register_in(txn, &principal, &key, now_millis()).await
            })
        })
        .await?;
    audit!(
        state,
        user,
        "inbox.key.register",
        "InboxKey",
        principal,
        serde_json::json!({ "key_id": stored.key_id, "algorithm": stored.algorithm })
    );
    Ok(Json(InboxKey::from(stored)))
}

#[utoipa::path(
    delete,
    path = "/inbox/keys/{key_id}",
    tag = "Inbox",
    params(
        ("key_id" = String, Path, description = "The key to revoke"),
        KeyOwnerQuery
    ),
    responses(
        (status = 204, description = "The key is revoked: nobody can seal to it any more, it no longer counts toward the 20 active keys, and its id can't be registered again. Revoking it again changes nothing."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people revoke inbox keys, not API keys or running flows"),
        (status = 404, description = "There is no key with this id, or principal_id is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /inbox/keys/{key_id}", skip(state, user))]
pub async fn revoke_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(key_id): Path<String>,
    Query(query): Query<KeyOwnerQuery>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let principal = key_owner(&state.db, &sub, query.principal_id.as_deref()).await?;
    let key_id = key_id.trim();
    if revoke_in(&state.db, &principal, key_id, now_millis()).await? {
        audit!(
            state,
            user,
            "inbox.key.revoke",
            "InboxKey",
            principal,
            serde_json::json!({ "key_id": key_id })
        );
    }
    Ok(StatusCode::NO_CONTENT)
}
