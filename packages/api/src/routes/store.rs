use crate::error::{ApiError, InternalError};
use crate::routes::app::groups::{GroupInfo, assemble_groups};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Json, Router, routing::get};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
pub struct DbStateResponse {
    pub rtt: u128,
}

pub fn routes() -> Router<AppState> {
    let router = Router::new();

    router
        .route("/", get(|| async { "ok" }))
        .route("/db", get(get_store_db))
        .route("/groups", get(list_public_groups))
        .route("/groups/{group_id}", get(get_public_group))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct StoreGroupsQuery {
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

/// Members shown for a public store group are limited to publicly-visible apps
/// so a private app is never leaked through a suite it belongs to.
async fn filter_public_members(
    state: &AppState,
    members: Vec<crate::entity::app_group_member::Model>,
) -> Result<Vec<crate::entity::app_group_member::Model>, ApiError> {
    use crate::entity::{app, sea_orm_active_enums::Visibility};
    if members.is_empty() {
        return Ok(members);
    }
    let app_ids: Vec<String> = members.iter().map(|m| m.app_id.clone()).collect();
    let public_apps: HashSet<String> = app::Entity::find()
        .filter(app::Column::Id.is_in(app_ids))
        .filter(
            app::Column::Visibility
                .eq(Visibility::Public)
                .or(app::Column::Visibility.eq(Visibility::PublicRequestAccess)),
        )
        .all(&state.db)
        .await?
        .into_iter()
        .map(|a| a.id)
        .collect();
    Ok(members
        .into_iter()
        .filter(|m| public_apps.contains(&m.app_id))
        .collect())
}

#[utoipa::path(
    get,
    path = "/store/groups",
    tag = "store",
    description = "List public app groups (\"suites\") for the store.",
    params(
        ("offset" = Option<u64>, Query, description = "Pagination offset"),
        ("limit" = Option<u64>, Query, description = "Page size (max 100)")
    ),
    responses((status = 200, description = "Public app groups", body = [GroupInfo]))
)]
pub async fn list_public_groups(
    State(state): State<AppState>,
    Query(query): Query<StoreGroupsQuery>,
) -> Result<Json<Vec<GroupInfo>>, ApiError> {
    use crate::entity::{
        app_group, app_group_member,
        sea_orm_active_enums::{AppGroupMemberStatus, Status, Visibility},
    };

    let limit = query.limit.unwrap_or(24).min(100);
    let offset = query.offset.unwrap_or(0);

    let groups = app_group::Entity::find()
        .filter(
            app_group::Column::Visibility
                .eq(Visibility::Public)
                .or(app_group::Column::Visibility.eq(Visibility::PublicRequestAccess)),
        )
        .filter(app_group::Column::Status.eq(Status::Active))
        .order_by_desc(app_group::Column::CreatedAt)
        .offset(offset)
        .limit(limit)
        .all(&state.db)
        .await?;

    let group_ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
    let members = if group_ids.is_empty() {
        vec![]
    } else {
        app_group_member::Entity::find()
            .filter(app_group_member::Column::GroupId.is_in(group_ids))
            .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
            .all(&state.db)
            .await?
    };
    let members = filter_public_members(&state, members).await?;

    Ok(Json(assemble_groups(&state, groups, members).await?))
}

#[utoipa::path(
    get,
    path = "/store/groups/{group_id}",
    tag = "store",
    description = "Get a public app group (suite) with its members.",
    params(("group_id" = String, Path, description = "Group ID")),
    responses(
        (status = 200, description = "Public group details", body = GroupInfo),
        (status = 404, description = "Group not found")
    )
)]
pub async fn get_public_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<GroupInfo>, ApiError> {
    use crate::entity::{
        app_group, app_group_member,
        sea_orm_active_enums::{AppGroupMemberStatus, Status, Visibility},
    };

    let group = app_group::Entity::find_by_id(&group_id)
        .filter(
            app_group::Column::Visibility
                .eq(Visibility::Public)
                .or(app_group::Column::Visibility.eq(Visibility::PublicRequestAccess)),
        )
        .filter(app_group::Column::Status.eq(Status::Active))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
        .order_by_asc(app_group_member::Column::Position)
        .all(&state.db)
        .await?;
    let members = filter_public_members(&state, members).await?;

    assemble_groups(&state, vec![group], members)
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or(ApiError::NOT_FOUND)
}

#[utoipa::path(
    get,
    path = "/store/db",
    tag = "store",
    responses(
        (status = 200, description = "Database connection status", body = DbStateResponse),
        (status = 500, description = "Database connection failed")
    )
)]
pub async fn get_store_db(
    State(state): State<AppState>,
) -> Result<Json<DbStateResponse>, InternalError> {
    let db = state.db.clone();
    let now = Instant::now();
    db.ping().await?;
    let elapsed = now.elapsed();
    let response = Json(DbStateResponse {
        rtt: elapsed.as_millis(),
    });
    Ok(response)
}
