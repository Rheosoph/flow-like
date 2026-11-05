//! Monthly app, bot and room budgets. Owner budgets narrow an administrator's ceiling.

use crate::{
    ensure_permission,
    entity::{app, bot, conversation, conversation_member},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    quota::QuotaAmounts,
    scoped_usage::{self, BudgetScope, MonthlyUsageLimits},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    #[sea_orm(from_alias = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
    calls: i64,
    #[sea_orm(from_alias = "inputTokens")]
    input_tokens: i64,
    #[sea_orm(from_alias = "outputTokens")]
    output_tokens: i64,
    #[sea_orm(from_alias = "embeddingTokens")]
    embedding_tokens: i64,
    #[sea_orm(from_alias = "costMicroDollars")]
    cost_micro_dollars: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBudgetView {
    scope: String,
    subject_id: String,
    group_id: Option<String>,
    name: Option<String>,
    archived: bool,
    owner_limits: MonthlyUsageLimits,
    admin_limits: Option<MonthlyUsageLimits>,
    default_limits: MonthlyUsageLimits,
    effective_limits: MonthlyUsageLimits,
    period_start: String,
    period_end: String,
    used: QuotaAmounts,
    reserved: QuotaAmounts,
    input_tokens: i64,
    output_tokens: i64,
    embedding_tokens: i64,
    models: Vec<ModelUsage>,
    groups: Vec<UsageBudgetView>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnterpriseDefaults {
    apps: MonthlyUsageLimits,
    bots: MonthlyUsageLimits,
}

#[derive(Default, Deserialize)]
pub struct BudgetListQuery {
    limit: Option<u64>,
    offset: Option<u64>,
}

#[derive(Serialize)]
pub struct BotBudgetList {
    items: Vec<UsageBudgetView>,
    total: u64,
}

async fn budget_view(
    state: &AppState,
    scope: BudgetScope,
    name: Option<String>,
) -> Result<UsageBudgetView, ApiError> {
    let policy = scoped_usage::get_policy(&state.db, &scope).await?;
    let defaults = scoped_usage::default_limits(&state.db, &scope).await?;
    let effective = scoped_usage::effective_limits(&policy, &defaults);
    let (used, reserved) = scoped_usage::current_usage(&state.db, &scope).await?;
    let (start, end) = crate::quota::monthly_period(Utc::now(), None);
    // The identifiers below are fixed server constants; only values come from the request.
    let column = if scope.scope == "app" {
        "appId"
    } else {
        "botId"
    };
    let mut filter = format!(r#""{column}"=$1 AND "startedAt">=$2 AND "startedAt"<$3"#);
    let mut values = vec![
        scope.subject_id.clone().into(),
        start.fixed_offset().into(),
        end.fixed_offset().into(),
    ];
    if let Some(group_id) = &scope.group_id {
        filter.push_str(r#" AND "conversationId"=$4"#);
        values.push(group_id.clone().into());
    }
    let models = ModelUsage::find_by_statement(Statement::from_sql_and_values(
        state.db.get_database_backend(),
        format!(
            r#"SELECT "modelId",provider,COUNT(*)::bigint AS calls,
            COALESCE(SUM("inputTokens"),0)::bigint AS "inputTokens",
            COALESCE(SUM("outputTokens"),0)::bigint AS "outputTokens",
            COALESCE(SUM("embeddingTokens"),0)::bigint AS "embeddingTokens",
            COALESCE(SUM("costMicroDollars"),0)::bigint AS "costMicroDollars"
            FROM "UsageInvocation" WHERE {filter} GROUP BY "modelId",provider
            ORDER BY "costMicroDollars" DESC, "modelId",provider"#
        ),
        values,
    ))
    .all(&state.db)
    .await?;
    Ok(UsageBudgetView {
        scope: scope.scope,
        subject_id: scope.subject_id,
        group_id: scope.group_id,
        name,
        archived: false,
        owner_limits: policy.owner_limits,
        admin_limits: policy.admin_limits,
        default_limits: defaults,
        effective_limits: effective,
        period_start: start.to_rfc3339(),
        period_end: end.to_rfc3339(),
        used,
        reserved,
        input_tokens: models
            .iter()
            .map(|row| row.input_tokens)
            .fold(0i64, i64::saturating_add),
        output_tokens: models
            .iter()
            .map(|row| row.output_tokens)
            .fold(0i64, i64::saturating_add),
        embedding_tokens: models
            .iter()
            .map(|row| row.embedding_tokens)
            .fold(0i64, i64::saturating_add),
        models,
        groups: vec![],
    })
}

async fn bot_view(state: &AppState, bot: bot::Model) -> Result<UsageBudgetView, ApiError> {
    let mut view = budget_view(state, BudgetScope::bot(&bot.id), bot.name).await?;
    view.archived = bot.deleted_at.is_some();
    // Retain attribution after the bot leaves a group. Membership also exposes a new,
    // unused group so its owner can set a budget before the first model call.
    let memberships = conversation_member::Entity::find()
        .filter(conversation_member::Column::PrincipalId.eq(&bot.id))
        .all(&state.db)
        .await?;
    let mut groups: BTreeSet<String> = memberships
        .into_iter()
        .map(|row| row.conversation_id)
        .collect();
    for row in state.db.query_all_raw(Statement::from_sql_and_values(
        state.db.get_database_backend(),
        r#"SELECT DISTINCT "groupId" FROM "UsageBudgetPeriod" WHERE scope='bot_group' AND "subjectId"=$1"#,
        vec![bot.id.clone().into()],
    )).await? {
        if let Some(id) = row.try_get::<Option<String>>("", "groupId")? { groups.insert(id); }
    }
    for group in groups {
        let conversation = conversation::Entity::find_by_id(&group)
            .one(&state.db)
            .await?;
        if conversation.as_ref().is_some_and(|row| row.kind != "GROUP") {
            continue;
        }
        let archived = view.archived || conversation.is_none();
        let mut group_view = budget_view(
            state,
            BudgetScope::bot_group(&bot.id, &group),
            conversation.and_then(|row| row.title),
        )
        .await?;
        group_view.archived = archived;
        view.groups.push(group_view);
    }
    Ok(view)
}

async fn existing_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)
}

async fn room_access<C: ConnectionTrait>(
    db: &C,
    actor: &str,
    bot_id: &str,
    group_id: &str,
    write: bool,
) -> Result<(), ApiError> {
    require_group(db, group_id).await?;
    let bot = existing_bot(db, bot_id).await?;
    if write || bot.owner_id == actor {
        super::bots::owned_bot(db, actor, bot_id).await?;
        // Owners can still review their bot's history after leaving a room.
        conversation_member::Entity::find_by_id((group_id.to_owned(), bot_id.to_owned()))
            .one(db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
    } else {
        super::inbox::active_membership(db, group_id, actor).await?;
        super::inbox::active_membership(db, group_id, bot_id).await?;
    }
    Ok(())
}

async fn require_group<C: ConnectionTrait>(db: &C, group_id: &str) -> Result<(), ApiError> {
    conversation::Entity::find_by_id(group_id)
        .filter(conversation::Column::Kind.eq("GROUP"))
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(())
}

pub async fn get_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    let bot = super::bots::owned_bot(&state.db, &user.sub()?, &bot_id).await?;
    Ok(Json(bot_view(&state, bot).await?))
}

pub async fn put_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(limits): Json<MonthlyUsageLimits>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    let actor = user.sub()?;
    let bot = super::bots::owned_bot(&state.db, &actor, &bot_id).await?;
    scoped_usage::save_policy(
        &state,
        &BudgetScope::bot(&bot_id),
        Some(limits),
        false,
        &actor,
    )
    .await?;
    Ok(Json(bot_view(&state, bot).await?))
}

pub async fn get_room(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, group_id)): Path<(String, String)>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    room_access(&state.db, &user.sub()?, &bot_id, &group_id, false).await?;
    let name = conversation::Entity::find_by_id(&group_id)
        .one(&state.db)
        .await?
        .and_then(|row| row.title);
    Ok(Json(
        budget_view(&state, BudgetScope::bot_group(&bot_id, &group_id), name).await?,
    ))
}

pub async fn put_room(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, group_id)): Path<(String, String)>,
    Json(limits): Json<MonthlyUsageLimits>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    room_access(&state.db, &user.sub()?, &bot_id, &group_id, true).await?;
    scoped_usage::save_policy(
        &state,
        &BudgetScope::bot_group(&bot_id, &group_id),
        Some(limits),
        false,
        &user.sub()?,
    )
    .await?;
    get_room(State(state), Extension(user), Path((bot_id, group_id))).await
}

pub async fn get_defaults(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<EnterpriseDefaults>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    Ok(Json(EnterpriseDefaults {
        apps: scoped_usage::default_limits(&state.db, &BudgetScope::app("")).await?,
        bots: scoped_usage::default_limits(&state.db, &BudgetScope::bot("")).await?,
    }))
}

pub async fn put_defaults(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(defaults): Json<EnterpriseDefaults>,
) -> Result<Json<EnterpriseDefaults>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    scoped_usage::save_defaults(
        &state,
        defaults.apps.clone(),
        defaults.bots.clone(),
        &user.sub()?,
    )
    .await?;
    Ok(Json(defaults))
}

pub async fn list_bots(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<BudgetListQuery>,
) -> Result<Json<BotBudgetList>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let (start, _) = crate::quota::monthly_period(Utc::now(), None);
    let select = bot::Entity::find().filter(Condition::any()
        .add(bot::Column::DeletedAt.is_null())
        .add(sea_orm::sea_query::Expr::cust_with_values(
            r#""id" IN (SELECT "subjectId" FROM "UsageBudgetPeriod" WHERE scope='bot' AND "periodStart"=$1)"#,
            [sea_orm::Value::from(start.timestamp_millis())],
        )));
    let total = select.clone().count(&state.db).await?;
    let bots = select
        .order_by_asc(bot::Column::Id)
        .offset(query.offset.unwrap_or(0))
        .limit(query.limit.unwrap_or(25).clamp(1, 100))
        .all(&state.db)
        .await?;
    let mut items = Vec::with_capacity(bots.len());
    for bot in bots {
        items.push(bot_view(&state, bot).await?);
    }
    Ok(Json(BotBudgetList { items, total }))
}

pub async fn put_admin_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(limits): Json<Option<MonthlyUsageLimits>>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let bot = existing_bot(&state.db, &bot_id).await?;
    scoped_usage::save_policy(
        &state,
        &BudgetScope::bot(&bot_id),
        limits,
        true,
        &user.sub()?,
    )
    .await?;
    Ok(Json(bot_view(&state, bot).await?))
}

pub async fn put_admin_room(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, group_id)): Path<(String, String)>,
    Json(limits): Json<Option<MonthlyUsageLimits>>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    existing_bot(&state.db, &bot_id).await?;
    require_group(&state.db, &group_id).await?;
    conversation_member::Entity::find_by_id((group_id.clone(), bot_id.clone()))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    scoped_usage::save_policy(
        &state,
        &BudgetScope::bot_group(&bot_id, &group_id),
        limits,
        true,
        &user.sub()?,
    )
    .await?;
    Ok(Json(
        budget_view(&state, BudgetScope::bot_group(&bot_id, &group_id), None).await?,
    ))
}

pub async fn get_admin_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(Json(
        budget_view(&state, BudgetScope::app(&app_id), None).await?,
    ))
}

pub async fn put_admin_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(limits): Json<Option<MonthlyUsageLimits>>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    scoped_usage::save_policy(
        &state,
        &BudgetScope::app(&app_id),
        limits,
        true,
        &user.sub()?,
    )
    .await?;
    get_admin_app(State(state), Extension(user), Path(app_id)).await
}

pub async fn get_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.sub()?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    Ok(Json(
        budget_view(&state, BudgetScope::app(&app_id), None).await?,
    ))
}

pub async fn put_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(limits): Json<MonthlyUsageLimits>,
) -> Result<Json<UsageBudgetView>, ApiError> {
    user.sub()?;
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    scoped_usage::save_policy(
        &state,
        &BudgetScope::app(&app_id),
        Some(limits),
        false,
        &sub.sub()?,
    )
    .await?;
    Ok(Json(
        budget_view(&state, BudgetScope::app(&app_id), None).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::inbox::fixture;

    async fn membership(db: &sea_orm::DatabaseConnection, principal: &str, status: &str) {
        db.execute_unprepared(r#"INSERT INTO "Conversation" (id,kind,encryption,"createdBy","lastActivityAt","createdAt","updatedAt")
            VALUES ('room','GROUP','PLAIN','owner','2026-11-01T00:00:00+00:00','2026-11-01T00:00:00+00:00','2026-11-01T00:00:00+00:00')
            ON CONFLICT (id) DO NOTHING"#).await.unwrap();
        db.execute_raw(Statement::from_sql_and_values(db.get_database_backend(),
            r#"INSERT INTO "ConversationMember" ("conversationId","principalId","principalKind","joinedAt",status)
               VALUES ('room',$1,'USER','2026-11-01T00:00:00+00:00',$2)"#,
            vec![principal.into(), status.into()],
        )).await.unwrap();
    }

    #[tokio::test]
    async fn room_usage_is_readable_by_members_but_only_editable_by_the_bot_owner() {
        let db = fixture::database().await;
        fixture::insert_bot(&db, "bot_a", "owner", "SERVER").await;
        membership(&db, "bot_a", "ACTIVE").await;
        membership(&db, "member", "ACTIVE").await;
        membership(&db, "invitee", "PENDING").await;
        assert!(
            room_access(&db, "owner", "bot_a", "room", true)
                .await
                .is_ok()
        );
        assert!(
            room_access(&db, "member", "bot_a", "room", false)
                .await
                .is_ok()
        );
        assert!(
            room_access(&db, "member", "bot_a", "room", true)
                .await
                .is_err()
        );
        assert!(
            room_access(&db, "invitee", "bot_a", "room", false)
                .await
                .is_err()
        );
        assert!(
            room_access(&db, "stranger", "bot_a", "room", false)
                .await
                .is_err()
        );
        assert!(
            room_access(&db, "owner", "bot_a", "other-room", true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn direct_chats_cannot_receive_a_separate_group_budget() {
        let db = fixture::database().await;
        fixture::insert_bot(&db, "bot_a", "owner", "SERVER").await;
        membership(&db, "bot_a", "ACTIVE").await;
        db.execute_unprepared(r#"UPDATE "Conversation" SET kind='DIRECT' WHERE id='room'"#)
            .await
            .unwrap();
        assert!(
            room_access(&db, "owner", "bot_a", "room", true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn leaving_a_group_keeps_owner_history_without_exposing_it_to_former_members() {
        let db = fixture::database().await;
        fixture::insert_bot(&db, "bot_a", "owner", "SERVER").await;
        membership(&db, "bot_a", "ACTIVE").await;
        membership(&db, "member", "ACTIVE").await;
        db.execute_unprepared(r#"UPDATE "ConversationMember" SET "leftAt"='2026-11-02T00:00:00+00:00' WHERE "principalId"='bot_a'"#).await.unwrap();
        assert!(
            room_access(&db, "owner", "bot_a", "room", false)
                .await
                .is_ok()
        );
        assert!(
            room_access(&db, "member", "bot_a", "room", false)
                .await
                .is_err()
        );
        fixture::delete_bot(&db, "bot_a").await;
        assert!(
            room_access(&db, "owner", "bot_a", "room", false)
                .await
                .is_err()
        );
    }
}
