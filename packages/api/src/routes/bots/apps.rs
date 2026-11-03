//! Apps a bot may use under `selected_apps`. The owner attaches any app they are a member of;
//! the app is not asked. The bot never holds more than the owner's current role there.

use super::{lock_bot, owned_bot, workspace::refuse_workspaces};
use crate::{
    audit_branch,
    entity::{bot_app, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, JoinType,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationDef, RelationTrait,
    sea_query::{Expr, ExprTrait, IntoCondition, OnConflict},
};
use serde::Serialize;
use utoipa::ToSchema;

/// Apps one bot may use, so deleting a bot stays one small transaction.
pub(super) const MAX_APPS_PER_BOT: u64 = 200;
pub(super) const BOT_APP_LIMIT_REACHED: &str = "BOT_APP_LIMIT_REACHED";

/// An app the bot may use.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct BotApp {
    pub app_id: String,
    pub added_at: DateTime<FixedOffset>,
    /// The owner's current role in the app, which bounds what the bot may do there; null once
    /// the owner is no longer a member.
    pub owner_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotAppList {
    /// Oldest attachment first.
    pub apps: Vec<BotApp>,
}

/// `BotApp` → the owner's `Membership` in the same app.
fn owner_membership(owner_id: &str) -> RelationDef {
    let owner_id = owner_id.to_string();
    bot_app::Entity::belongs_to(membership::Entity)
        .from(bot_app::Column::AppId)
        .to(membership::Column::AppId)
        .on_condition(move |_, membership_table| {
            Expr::col((membership_table, membership::Column::UserId))
                .eq(owner_id.clone())
                .into_condition()
        })
        .into()
}

/// The bot's apps with the owner's role in each, in one query.
pub(super) async fn attached_apps<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<BotApp>, DbErr> {
    let rows = bot_app::Entity::find()
        .select_only()
        .column(bot_app::Column::AppId)
        .column(bot_app::Column::AddedAt)
        .column(role::Column::Name)
        .join(JoinType::LeftJoin, owner_membership(owner_id))
        .join(JoinType::LeftJoin, membership::Relation::Role.def())
        .filter(bot_app::Column::BotId.eq(bot_id))
        .order_by_asc(bot_app::Column::AddedAt)
        .order_by_asc(bot_app::Column::AppId)
        .into_tuple::<(String, DateTime<FixedOffset>, Option<String>)>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(app_id, added_at, owner_role)| BotApp {
            app_id,
            added_at,
            owner_role,
        })
        .collect())
}

/// The bot's apps the owner is still a member of, oldest attachment first.
pub(super) async fn member_app_ids<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<String>, DbErr> {
    bot_app::Entity::find()
        .select_only()
        .column(bot_app::Column::AppId)
        .join(JoinType::InnerJoin, owner_membership(owner_id))
        .filter(bot_app::Column::BotId.eq(bot_id))
        .order_by_asc(bot_app::Column::AddedAt)
        .order_by_asc(bot_app::Column::AppId)
        .into_tuple::<String>()
        .all(db)
        .await
}

async fn require_membership<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    app_id: &str,
) -> Result<(), ApiError> {
    let membership = membership::Entity::find()
        .select_only()
        .column(membership::Column::Id)
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::UserId.eq(owner_id))
        .into_tuple::<String>()
        .one(txn)
        .await?;
    membership.map(|_| ()).ok_or_else(|| {
        ApiError::forbidden(format!(
            "You are not a member of app '{app_id}', so your bot can't use it"
        ))
    })
}

async fn attach(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
    app_id: &str,
) -> Result<bool, ApiError> {
    lock_bot(txn, bot_id).await?;
    attach_in(txn, owner_id, bot_id, app_id).await
}

/// Attaches the app unless it already is; true when this call attached it. A bot's workspace is
/// never attached: its own is always in reach and another bot's is not shared. The caller holds
/// the bot's lock, so the [`MAX_APPS_PER_BOT`] count is exact.
pub(super) async fn attach_in<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    app_id: &str,
) -> Result<bool, ApiError> {
    owned_bot(txn, owner_id, bot_id).await?;
    require_membership(txn, owner_id, app_id).await?;
    refuse_workspaces(txn, [app_id]).await?;
    let attached = bot_app::Entity::find_by_id((bot_id.to_string(), app_id.to_string()))
        .one(txn)
        .await?
        .is_some();
    if attached {
        return Ok(false);
    }
    let count = bot_app::Entity::find()
        .filter(bot_app::Column::BotId.eq(bot_id))
        .count(txn)
        .await?;
    if count >= MAX_APPS_PER_BOT {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_APP_LIMIT_REACHED,
            format!(
                "Bot '{bot_id}' already uses {MAX_APPS_PER_BOT} apps; detach one before attaching '{app_id}'"
            ),
        ));
    }
    let inserted = bot_app::Entity::insert(bot_app::ActiveModel {
        bot_id: Set(bot_id.to_string()),
        app_id: Set(app_id.to_string()),
        added_at: Set(chrono::Utc::now().fixed_offset()),
    })
    .on_conflict(
        OnConflict::columns([bot_app::Column::BotId, bot_app::Column::AppId])
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    Ok(inserted > 0)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/apps",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The apps you attached to the bot, each with your current role there", body = BotAppList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/apps", skip(state, user))]
pub async fn list_bot_apps(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotAppList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    Ok(Json(BotAppList {
        apps: attached_apps(&state.db, &sub, &bot_id).await?,
    }))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/apps/{app_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("app_id" = String, Path, description = "An app you are a member of")
    ),
    responses(
        (status = 204, description = "The bot may use the app with your role there; attaching twice changes nothing"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "You are not a member of the app, or the caller is not a person"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_APP_LIMIT_REACHED: the bot already uses 200 apps; BOT_WORKSPACE_LOCKED: the app is a bot's workspace (a bot always reaches its own)")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/apps/{app_id}", skip(state, user))]
pub async fn attach_bot_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, app_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let attached = state
        .transaction(|txn| {
            let (sub, bot_id, app_id) = (sub.clone(), bot_id.clone(), app_id.clone());
            Box::pin(async move { attach(txn, &sub, &bot_id, &app_id).await })
        })
        .await?;
    if attached {
        audit_branch!(state, user, app_id, "bot.app.attach", "Bot", bot_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/apps/{app_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("app_id" = String, Path, description = "An app attached to the bot")
    ),
    responses(
        (status = 204, description = "The bot no longer uses the app (or never did)"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/apps/{app_id}", skip(state, user))]
pub async fn detach_bot_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, app_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let removed = bot_app::Entity::delete_many()
        .filter(bot_app::Column::BotId.eq(&bot_id))
        .filter(bot_app::Column::AppId.eq(&app_id))
        .exec(&state.db)
        .await?;
    if removed.rows_affected > 0 {
        audit_branch!(state, user, app_id, "bot.app.detach", "Bot", bot_id);
    }
    Ok(StatusCode::NO_CONTENT)
}
