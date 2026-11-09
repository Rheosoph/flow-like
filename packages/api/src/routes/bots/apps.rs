//! Apps a bot may use under `selected_apps`. The owner attaches any app they are a member of;
//! the app is not asked. The bot never holds more than the owner's current role there.

use super::{lock_bot, owned_bot, workspace::refuse_workspaces};
use crate::middleware::jwt::user_app_permission_uncached;
use crate::permission::role_permission::RolePermissions;
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
use flow_like_types::bot_restrictions::{BotPermission, BotRestrictions, BotScopes};
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
    /// What you let the bot do in this app, in every chat; only you see it.
    #[schema(value_type = Object)]
    pub restrictions: BotRestrictions,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotAppList {
    /// Oldest attachment first.
    pub apps: Vec<BotApp>,
}

/// A new attachment: the owner's full role, no scopes.
pub(super) fn unrestricted_row(
    bot_id: &str,
    app_id: &str,
    added_at: DateTime<FixedOffset>,
) -> bot_app::ActiveModel {
    bot_app::ActiveModel {
        bot_id: Set(bot_id.to_string()),
        app_id: Set(app_id.to_string()),
        added_at: Set(added_at),
        permission_mask: Set(None),
        scopes: Set(serde_json::json!({})),
    }
}

/// The stored mask and scopes as restrictions; scopes that no longer parse restrict nothing.
pub(super) fn restrictions_of(mask: Option<i64>, scopes: serde_json::Value) -> BotRestrictions {
    let scopes: BotScopes = serde_json::from_value(scopes).unwrap_or_default();
    BotRestrictions::from_bits(mask, scopes)
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
pub(crate) async fn attached_apps<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<BotApp>, DbErr> {
    let rows = bot_app::Entity::find()
        .select_only()
        .column(bot_app::Column::AppId)
        .column(bot_app::Column::AddedAt)
        .column(bot_app::Column::PermissionMask)
        .column(bot_app::Column::Scopes)
        .column(role::Column::Name)
        .join(JoinType::LeftJoin, owner_membership(owner_id))
        .join(JoinType::LeftJoin, membership::Relation::Role.def())
        .filter(bot_app::Column::BotId.eq(bot_id))
        .order_by_asc(bot_app::Column::AddedAt)
        .order_by_asc(bot_app::Column::AppId)
        .into_tuple::<(
            String,
            DateTime<FixedOffset>,
            Option<i64>,
            serde_json::Value,
            Option<String>,
        )>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(app_id, added_at, mask, scopes, owner_role)| BotApp {
            app_id,
            added_at,
            owner_role,
            restrictions: restrictions_of(mask, scopes),
        })
        .collect())
}

/// An attached app with its stored restrictions, as a duplicate takes it over.
pub(super) type MemberApp = (String, Option<i64>, serde_json::Value);

/// The bot's apps the owner is still a member of, oldest attachment first.
pub(super) async fn member_apps<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<MemberApp>, DbErr> {
    bot_app::Entity::find()
        .select_only()
        .column(bot_app::Column::AppId)
        .column(bot_app::Column::PermissionMask)
        .column(bot_app::Column::Scopes)
        .join(JoinType::InnerJoin, owner_membership(owner_id))
        .filter(bot_app::Column::BotId.eq(bot_id))
        .order_by_asc(bot_app::Column::AddedAt)
        .order_by_asc(bot_app::Column::AppId)
        .into_tuple::<MemberApp>()
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
    let inserted = bot_app::Entity::insert(unrestricted_row(
        bot_id,
        app_id,
        chrono::Utc::now().fixed_offset(),
    ))
    .on_conflict(
        OnConflict::columns([bot_app::Column::BotId, bot_app::Column::AppId])
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    Ok(inserted > 0)
}

/// Attaches an app the owner is a member of to their bot; true when this call attached it.
pub(crate) async fn attach_owned(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    app_id: &str,
) -> Result<bool, ApiError> {
    owned_bot(&state.db, owner_id, bot_id).await?;
    state
        .transaction(|txn| {
            let (owner_id, bot_id, app_id) =
                (owner_id.to_string(), bot_id.to_string(), app_id.to_string());
            Box::pin(async move { attach(txn, &owner_id, &bot_id, &app_id).await })
        })
        .await
}

/// Detaches an app from the owner's bot; true when it was attached.
pub(crate) async fn detach_owned(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    app_id: &str,
) -> Result<bool, ApiError> {
    owned_bot(&state.db, owner_id, bot_id).await?;
    let removed = bot_app::Entity::delete_many()
        .filter(bot_app::Column::BotId.eq(bot_id))
        .filter(bot_app::Column::AppId.eq(app_id))
        .exec(&state.db)
        .await?;
    Ok(removed.rows_affected > 0)
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
    let attached = attach_owned(&state, &sub, &bot_id, &app_id).await?;
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
    if detach_owned(&state, &sub, &bot_id, &app_id).await? {
        audit_branch!(state, user, app_id, "bot.app.detach", "Bot", bot_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's current role bounds what they can hand to a bot.
async fn refuse_beyond_owner_role(
    state: &AppState,
    owner_id: &str,
    app_id: &str,
    restrictions: &BotRestrictions,
) -> Result<(), ApiError> {
    let Some(requested) = restrictions.permissions.as_deref() else {
        return Ok(());
    };
    let held = user_app_permission_uncached(owner_id, app_id, state).await?;
    let beyond: Vec<String> = requested
        .iter()
        .filter(|permission| !held.has_permission(role_flag(**permission)))
        .map(|permission| format!("{permission:?}"))
        .collect();
    if beyond.is_empty() {
        return Ok(());
    }
    Err(ApiError::forbidden(format!(
        "Your role in app '{app_id}' doesn't include {}, so your bot can't be given it",
        beyond.join(", ")
    )))
}

fn role_flag(permission: BotPermission) -> RolePermissions {
    RolePermissions::from_bits_truncate(permission.bit())
}

/// Stores the restrictions of an app the owner's bot uses; any other owner's bot is not found.
pub(super) async fn store_restrictions<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    app_id: &str,
    restrictions: &BotRestrictions,
) -> Result<(), ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let scopes = serde_json::to_value(&restrictions.scopes)
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))?;
    let updated = bot_app::Entity::update_many()
        .col_expr(
            bot_app::Column::PermissionMask,
            Expr::value(restrictions.permission_bits()),
        )
        .col_expr(bot_app::Column::Scopes, Expr::value(scopes))
        .filter(bot_app::Column::BotId.eq(bot_id))
        .filter(bot_app::Column::AppId.eq(app_id))
        .exec(db)
        .await?;
    if updated.rows_affected == 0 {
        return Err(ApiError::not_found(format!(
            "Bot '{bot_id}' doesn't use app '{app_id}'"
        )));
    }
    Ok(())
}

async fn attached_row<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    app_id: &str,
) -> Result<bot_app::Model, ApiError> {
    bot_app::Entity::find_by_id((bot_id.to_string(), app_id.to_string()))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' doesn't use app '{app_id}'")))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/apps/{app_id}/restrictions",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("app_id" = String, Path, description = "An app attached to the bot")
    ),
    responses(
        (status = 200, description = "What you let the bot do in this app. It applies in every chat, yours included.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id, or it doesn't use the app")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /bots/{bot_id}/apps/{app_id}/restrictions",
    skip(state, user)
)]
pub async fn get_bot_app_restrictions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, app_id)): Path<(String, String)>,
) -> Result<Json<BotRestrictions>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let row = attached_row(&state.db, &bot_id, &app_id).await?;
    Ok(Json(restrictions_of(row.permission_mask, row.scopes)))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/apps/{app_id}/restrictions",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("app_id" = String, Path, description = "An app attached to the bot")
    ),
    request_body(content = Object, description = "Permissions the bot may hold (omit for your whole role) and scopes: folders, tables, events and boards it may reach"),
    responses(
        (status = 200, description = "The stored restrictions; they apply in every chat, yours included, and can only narrow your own role", body = Object),
        (status = 400, description = "A folder leaves the app, or a list is blank or too long"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Your role in the app doesn't include a permission you asked to give the bot"),
        (status = 404, description = "You have no bot with this id, or it doesn't use the app")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /bots/{bot_id}/apps/{app_id}/restrictions",
    skip(state, user, restrictions)
)]
pub async fn set_bot_app_restrictions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, app_id)): Path<(String, String)>,
    Json(restrictions): Json<BotRestrictions>,
) -> Result<Json<BotRestrictions>, ApiError> {
    let sub = user.sub()?;
    restrictions.validate().map_err(ApiError::bad_request)?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    refuse_beyond_owner_role(&state, &sub, &app_id, &restrictions).await?;
    store_restrictions(&state.db, &sub, &bot_id, &app_id, &restrictions).await?;
    audit_branch!(state, user, app_id, "bot.app.restrict", "Bot", bot_id);
    Ok(Json(restrictions))
}
