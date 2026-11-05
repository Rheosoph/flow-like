//! Standing grants between a person's bots: a grant from A to B lets A ask B without asking the
//! person again. It points one way, the same as the desktop's local grants, and it goes when
//! either bot goes. Both bots must be the caller's and live; every query is scoped to the caller
//! in SQL.

use super::{lock_bot, owned_bot};
use crate::{
    audit, entity::bot_permission, error::ApiError, middleware::jwt::AppUser,
    routes::inbox::now_millis, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::create_id;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, sea_query::OnConflict,
};
use serde::Serialize;
use utoipa::ToSchema;

pub(crate) const BOT_PERMISSION_SELF: &str = "BOT_PERMISSION_SELF";

/// One bot allowed to ask another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotPermission {
    pub id: String,
    /// The bot that may ask.
    pub from_bot_id: String,
    /// The bot that may be asked.
    pub to_bot_id: String,
    pub granted_at: String,
    /// The person who granted it.
    pub granted_by: String,
}

impl From<bot_permission::Model> for BotPermission {
    fn from(row: bot_permission::Model) -> Self {
        Self {
            id: row.id,
            from_bot_id: row.from_bot_id,
            to_bot_id: row.to_bot_id,
            granted_at: row.granted_at.to_rfc3339(),
            granted_by: row.granted_by,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotPermissionList {
    /// Bots this bot may ask, oldest grant first.
    pub granted: Vec<BotPermission>,
    /// Bots that may ask this bot, oldest grant first.
    pub received: Vec<BotPermission>,
}

fn pair(owner_id: &str, from_bot_id: &str, to_bot_id: &str) -> Condition {
    Condition::all()
        .add(bot_permission::Column::OwnerId.eq(owner_id))
        .add(bot_permission::Column::FromBotId.eq(from_bot_id))
        .add(bot_permission::Column::ToBotId.eq(to_bot_id))
}

pub(crate) async fn list_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<BotPermissionList, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let rows = bot_permission::Entity::find()
        .filter(bot_permission::Column::OwnerId.eq(owner_id))
        .filter(
            Condition::any()
                .add(bot_permission::Column::FromBotId.eq(bot_id))
                .add(bot_permission::Column::ToBotId.eq(bot_id)),
        )
        .order_by_asc(bot_permission::Column::GrantedAt)
        .order_by_asc(bot_permission::Column::Id)
        .all(db)
        .await?;
    let mut list = BotPermissionList::default();
    for row in rows {
        if row.from_bot_id == bot_id {
            list.granted.push(row.into());
        } else {
            list.received.push(row.into());
        }
    }
    Ok(list)
}

/// Lets `from_bot_id` ask `to_bot_id`; granting again returns the first grant. The flag says
/// whether this call created it.
pub(crate) async fn grant_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    from_bot_id: &str,
    to_bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(BotPermission, bool), ApiError> {
    if from_bot_id == to_bot_id {
        return Err(ApiError::coded(
            StatusCode::BAD_REQUEST,
            BOT_PERMISSION_SELF,
            "A bot doesn't need permission to ask itself",
        ));
    }
    owned_bot(db, owner_id, from_bot_id).await?;
    owned_bot(db, owner_id, to_bot_id).await?;
    let created = bot_permission::Entity::insert(bot_permission::ActiveModel {
        id: Set(format!("grant_{}", create_id())),
        owner_id: Set(owner_id.to_string()),
        from_bot_id: Set(from_bot_id.to_string()),
        to_bot_id: Set(to_bot_id.to_string()),
        granted_at: Set(now),
        granted_by: Set(owner_id.to_string()),
    })
    .on_conflict(
        OnConflict::columns([
            bot_permission::Column::FromBotId,
            bot_permission::Column::ToBotId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?
        > 0;
    let row = bot_permission::Entity::find()
        .filter(pair(owner_id, from_bot_id, to_bot_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Bot not found"))?;
    Ok((row.into(), created))
}

/// Takes the grant back; `false` when there was none.
pub(crate) async fn revoke_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    from_bot_id: &str,
    to_bot_id: &str,
) -> Result<bool, ApiError> {
    owned_bot(db, owner_id, from_bot_id).await?;
    owned_bot(db, owner_id, to_bot_id).await?;
    let removed = bot_permission::Entity::delete_many()
        .filter(pair(owner_id, from_bot_id, to_bot_id))
        .exec(db)
        .await?;
    Ok(removed.rows_affected > 0)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/permissions",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bots this bot may ask without asking you, and the bots that may ask it.", body = BotPermissionList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/permissions", skip(state, user))]
pub async fn list_bot_permissions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotPermissionList>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(list_in(&state.db, &sub, &bot_id).await?))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/permissions/{to_bot_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "The bot that may ask; yours"),
        ("to_bot_id" = String, Path, description = "The bot that may be asked; yours")
    ),
    responses(
        (status = 200, description = "The first bot may ask the second without asking you. Granting again changes nothing and returns the first grant.", body = BotPermission),
        (status = 400, description = "BOT_PERMISSION_SELF: a bot doesn't need permission to ask itself"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "One of the bots isn't yours or was deleted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/permissions/{to_bot_id}", skip(state, user))]
pub async fn grant_bot_permission(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((from_bot_id, to_bot_id)): Path<(String, String)>,
) -> Result<Json<BotPermission>, ApiError> {
    let sub = user.sub()?;
    let (grant, created) = state
        .transaction(|txn| {
            let (sub, from, to) = (sub.clone(), from_bot_id.clone(), to_bot_id.clone());
            Box::pin(async move {
                lock_pair(txn, &from, &to).await?;
                grant_in(txn, &sub, &from, &to, now_millis()).await
            })
        })
        .await?;
    if created {
        audit!(
            state,
            user,
            "bot.permission.grant",
            "Bot",
            from_bot_id,
            serde_json::json!({ "to_bot_id": to_bot_id })
        );
    }
    Ok(Json(grant))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/permissions/{to_bot_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "The bot that was allowed to ask; yours"),
        ("to_bot_id" = String, Path, description = "The bot that was allowed to be asked; yours")
    ),
    responses(
        (status = 204, description = "The first bot has to ask you again before it asks the second (or never had permission)"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "One of the bots isn't yours or was deleted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /bots/{bot_id}/permissions/{to_bot_id}",
    skip(state, user)
)]
pub async fn revoke_bot_permission(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((from_bot_id, to_bot_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let removed = state
        .transaction(|txn| {
            let (sub, from, to) = (sub.clone(), from_bot_id.clone(), to_bot_id.clone());
            Box::pin(async move {
                lock_pair(txn, &from, &to).await?;
                revoke_in(txn, &sub, &from, &to).await
            })
        })
        .await?;
    if removed {
        audit!(
            state,
            user,
            "bot.permission.revoke",
            "Bot",
            from_bot_id,
            serde_json::json!({ "to_bot_id": to_bot_id })
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn lock_pair(
    txn: &sea_orm::DatabaseTransaction,
    from_bot_id: &str,
    to_bot_id: &str,
) -> Result<(), sea_orm::DbErr> {
    let mut ids = [from_bot_id, to_bot_id];
    ids.sort_unstable();
    for id in ids {
        lock_bot(txn, id).await?;
    }
    Ok(())
}
