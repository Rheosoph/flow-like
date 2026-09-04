use crate::{
    ensure_permission,
    entity::{role, technical_user, user as user_entity},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub creator_user_id: Option<String>,
    pub creator_membership_id: Option<String>,
    pub creator_display_name: Option<String>,
    pub creator_email: Option<String>,
    pub role_id: Option<String>,
    pub role_name: Option<String>,
    pub role_permissions: Option<i64>,
    pub valid_until: Option<i64>,
    pub created_at: i64,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/api",
    tag = "api-keys",
    description = "List API keys for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "API keys", body = Vec<ApiKeyInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/api", skip(state, user))]
pub async fn get_api_keys(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<ApiKeyInfo>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let lookup_config = state.platform_config.lookup.clone();

    let technical_users = technical_user::Entity::find()
        .filter(technical_user::Column::AppId.eq(&app_id))
        .limit(1000)
        .all(&state.db)
        .await?;

    // Get all role IDs to fetch role info
    let role_ids: Vec<String> = technical_users
        .iter()
        .filter_map(|tu| tu.role_id.clone())
        .collect();

    let roles: std::collections::HashMap<String, role::Model> = if !role_ids.is_empty() {
        role::Entity::find()
            .filter(role::Column::Id.is_in(role_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    let creator_user_ids: Vec<String> = technical_users
        .iter()
        .filter_map(|tu| tu.creator_user_id.clone())
        .collect();

    let creators: std::collections::HashMap<String, user_entity::Model> =
        if !creator_user_ids.is_empty() {
            user_entity::Entity::find()
                .filter(user_entity::Column::Id.is_in(creator_user_ids))
                .all(&state.db)
                .await?
                .into_iter()
                .map(|u| (u.id.clone(), u))
                .collect()
        } else {
            std::collections::HashMap::new()
        };

    let api_keys = technical_users
        .into_iter()
        .map(|tu| {
            let role = tu.role_id.as_ref().and_then(|id| roles.get(id));
            let creator = tu.creator_user_id.as_ref().and_then(|id| creators.get(id));
            let creator_display_name = creator.and_then(|user| {
                if lookup_config.name
                    && let Some(name) = user.name.clone()
                {
                    return Some(name);
                }

                if lookup_config.preferred_username
                    && let Some(preferred_username) = user.preferred_username.clone()
                {
                    return Some(preferred_username);
                }

                if lookup_config.username
                    && let Some(username) = user.username.clone()
                {
                    return Some(username);
                }

                None
            });
            let creator_email = if lookup_config.email {
                creator.and_then(|user| user.email.clone())
            } else {
                None
            };
            ApiKeyInfo {
                id: tu.id,
                name: tu.name,
                description: tu.description,
                creator_user_id: tu.creator_user_id,
                creator_membership_id: tu.creator_membership_id,
                creator_display_name,
                creator_email,
                role_id: tu.role_id,
                role_name: role.map(|r| r.name.clone()),
                role_permissions: role.map(|r| r.permissions),
                valid_until: tu.valid_until.map(|dt| dt.timestamp()),
                created_at: tu.created_at.timestamp(),
            }
        })
        .collect();

    Ok(Json(api_keys))
}
