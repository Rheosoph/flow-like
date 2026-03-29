//! List and search users for admin

use crate::entity::sea_orm_active_enums::{UserStatus, UserTier};
use crate::entity::user;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use sea_orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListUsersQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserRecord {
    pub id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub status: String,
    pub tier: String,
    pub permission: i64,
    pub total_size: i64,
    pub total_llm_price: i64,
    pub total_embedding_price: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListUsersResponse {
    pub users: Vec<AdminUserRecord>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

fn user_status_to_str(s: &UserStatus) -> String {
    match s {
        UserStatus::Active => "ACTIVE".to_string(),
        UserStatus::Inactive => "INACTIVE".to_string(),
        UserStatus::Banned => "BANNED".to_string(),
    }
}

fn user_tier_to_str(t: &UserTier) -> String {
    match t {
        UserTier::Free => "FREE".to_string(),
        UserTier::Premium => "PREMIUM".to_string(),
        UserTier::Pro => "PRO".to_string(),
        UserTier::Enterprise => "ENTERPRISE".to_string(),
    }
}

#[utoipa::path(
    get,
    path = "/admin/users",
    tag = "admin",
    params(ListUsersQuery),
    responses(
        (status = 200, description = "List of users", body = ListUsersResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List and search all users. Requires Admin permission."
)]
pub async fn list_users(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<ListUsersResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(25).min(100);

    let mut select = user::Entity::find();

    if let Some(q) = &query.query {
        if !q.is_empty() {
            let pattern = format!("%{}%", q);
            select = select.filter(
                user::Column::Email
                    .like(&pattern)
                    .or(user::Column::Username.like(&pattern))
                    .or(user::Column::PreferredUsername.like(&pattern))
                    .or(user::Column::Name.like(&pattern))
                    .or(user::Column::Id.like(&pattern)),
            );
        }
    }

    if let Some(status) = &query.status {
        let s = match status.to_uppercase().as_str() {
            "ACTIVE" => Some(UserStatus::Active),
            "INACTIVE" => Some(UserStatus::Inactive),
            "BANNED" => Some(UserStatus::Banned),
            _ => None,
        };
        if let Some(s) = s {
            select = select.filter(user::Column::Status.eq(s));
        }
    }

    if let Some(tier) = &query.tier {
        let t = match tier.to_uppercase().as_str() {
            "FREE" => Some(UserTier::Free),
            "PREMIUM" => Some(UserTier::Premium),
            "PRO" => Some(UserTier::Pro),
            "ENTERPRISE" => Some(UserTier::Enterprise),
            _ => None,
        };
        if let Some(t) = t {
            select = select.filter(user::Column::Tier.eq(t));
        }
    }

    let total = select.clone().count(&state.db).await?;

    let records = select
        .order_by(user::Column::CreatedAt, Order::Desc)
        .offset(offset)
        .limit(limit)
        .all(&state.db)
        .await?;

    let users = records
        .into_iter()
        .map(|u| AdminUserRecord {
            id: u.id,
            email: u.email,
            username: u.username,
            preferred_username: u.preferred_username,
            name: u.name,
            avatar: u.avatar,
            status: user_status_to_str(&u.status),
            tier: user_tier_to_str(&u.tier),
            permission: u.permission,
            total_size: u.total_size,
            total_llm_price: u.total_llm_price,
            total_embedding_price: u.total_embedding_price,
            created_at: u.created_at.to_string(),
            updated_at: u.updated_at.to_string(),
        })
        .collect();

    Ok(Json(ListUsersResponse {
        users,
        total,
        offset,
        limit,
    }))
}
