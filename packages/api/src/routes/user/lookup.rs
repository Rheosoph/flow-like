use crate::{
    entity::{membership, user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::user::sign_avatar,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::hub::Lookup;
use flow_like_types::Value;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UserLookupResponse {
    id: String,
    email: Option<String>,
    username: Option<String>,
    preferred_username: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
    additional_information: Option<Value>,
    description: Option<String>,
    created_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UserBatchLookupBody {
    pub user_ids: Vec<String>,
}

impl UserLookupResponse {
    pub async fn parse(user: user::Model, lookup_config: Lookup, state: &AppState) -> Self {
        let avatar_url = match (lookup_config.avatar, user.avatar.as_ref()) {
            (true, Some(avatar_id)) => match sign_avatar(&user.id, avatar_id, state).await {
                Ok(url) => Some(url),
                Err(err) => {
                    tracing::error!("Failed to sign avatar URL: {:?}", err);
                    None
                }
            },
            _ => None,
        };

        UserLookupResponse {
            id: user.id,
            email: lookup_config.email.then_some(user.email).flatten(),
            username: lookup_config.username.then_some(user.username).flatten(),
            preferred_username: lookup_config
                .preferred_username
                .then_some(user.preferred_username)
                .flatten(),
            name: lookup_config.name.then_some(user.name).flatten(),
            avatar_url,
            additional_information: lookup_config
                .additional_information
                .then_some(user.additional_information)
                .flatten(),
            description: lookup_config
                .description
                .then_some(user.description)
                .flatten(),
            created_at: lookup_config.created_at.then_some(user.created_at),
        }
    }
}

#[utoipa::path(
    get,
    path = "/user/lookup/{sub}",
    tag = "user",
    params(
        ("sub" = String, Path, description = "User ID to look up")
    ),
    responses(
        (status = 200, description = "User found", body = UserLookupResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/lookup/{sub}", skip(state, user))]
pub async fn user_lookup(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(sub): Path<String>,
) -> Result<Json<UserLookupResponse>, ApiError> {
    user.executor_scoped_sub()?;
    let sub = scoped_lookup_id(&state, &user, sub).await?;
    let lookup_config = state.platform_config.lookup.clone();
    let found_user = user::Entity::find()
        .filter(user::Column::Id.eq(&sub))
        .one(&state.db)
        .await?;

    if let Some(user_info) = found_user {
        let response = UserLookupResponse::parse(user_info, lookup_config, &state).await;
        return Ok(Json(response));
    }

    Err(ApiError::NOT_FOUND)
}

#[utoipa::path(
    post,
    path = "/user/lookup",
    tag = "user",
    request_body = UserBatchLookupBody,
    responses(
        (status = 200, description = "Users found for the requested IDs", body = Vec<UserLookupResponse>),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /user/lookup (batch)", skip(state, user, body))]
pub async fn user_batch_lookup(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<UserBatchLookupBody>,
) -> Result<Json<Vec<UserLookupResponse>>, ApiError> {
    user.executor_scoped_sub()?;
    let lookup_config = state.platform_config.lookup.clone();
    let ids = body
        .user_ids
        .into_iter()
        .filter(|id| !id.trim().is_empty())
        .take(100)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    if ids.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let ids = scoped_lookup_ids(&state, &user, ids).await?;
    if ids.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let found_users = user::Entity::find()
        .filter(user::Column::Id.is_in(ids))
        .limit(100)
        .all(&state.db)
        .await?;

    let mut responses = Vec::with_capacity(found_users.len());
    for user_info in found_users {
        responses.push(UserLookupResponse::parse(user_info, lookup_config.clone(), &state).await);
    }

    Ok(Json(responses))
}

async fn scoped_lookup_id(
    state: &AppState,
    user: &AppUser,
    sub: String,
) -> Result<String, ApiError> {
    if let AppUser::Executor(executor) = user {
        ensure_executor_lookup_permission(state, user, &executor.app_id).await?;
        let membership = membership::Entity::find()
            .filter(membership::Column::AppId.eq(&executor.app_id))
            .filter(membership::Column::UserId.eq(&sub))
            .one(&state.db)
            .await?;

        if membership.is_none() {
            return Err(ApiError::NOT_FOUND);
        }
    }

    Ok(sub)
}

async fn scoped_lookup_ids(
    state: &AppState,
    user: &AppUser,
    ids: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    if let AppUser::Executor(executor) = user {
        ensure_executor_lookup_permission(state, user, &executor.app_id).await?;
        let ids = membership::Entity::find()
            .filter(membership::Column::AppId.eq(&executor.app_id))
            .filter(membership::Column::UserId.is_in(ids))
            .limit(100)
            .all(&state.db)
            .await?
            .into_iter()
            .map(|membership| membership.user_id)
            .collect();

        return Ok(ids);
    }

    Ok(ids)
}

async fn ensure_executor_lookup_permission(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
) -> Result<(), ApiError> {
    let permission = user.execution_app_permission(app_id, state).await?;
    if !permission.has_permission(RolePermissions::ReadTeam) {
        return Err(ApiError::FORBIDDEN);
    }

    Ok(())
}

#[utoipa::path(
    get,
    path = "/user/search/{query}",
    tag = "user",
    params(
        ("query" = String, Path, description = "Search query (username, email, or name)")
    ),
    responses(
        (status = 200, description = "Users matching the search query", body = Vec<UserLookupResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No users found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/search/{query}", skip(state, user))]
pub async fn user_search(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(query): Path<String>,
) -> Result<Json<Vec<UserLookupResponse>>, ApiError> {
    user.sub()?;
    let lookup_config = state.platform_config.lookup.clone();

    // First try exact matches
    let exact_matches = user::Entity::find()
        .filter(
            user::Column::Id
                .eq(&query)
                .or(user::Column::Email.eq(&query))
                .or(user::Column::Username.eq(&query)),
        )
        .all(&state.db)
        .await?;

    if !exact_matches.is_empty() {
        let mut responses: Vec<UserLookupResponse> = Vec::with_capacity(exact_matches.len());

        for user_info in exact_matches {
            let response =
                UserLookupResponse::parse(user_info, lookup_config.clone(), &state).await;
            responses.push(response);
        }

        return Ok(Json(responses));
    }

    // If no exact matches, try fuzzy search
    let fuzzy_query = format!("%{}%", query);
    let fuzzy_matches = user::Entity::find()
        .filter(
            user::Column::Username
                .like(&fuzzy_query)
                .or(user::Column::Name.like(&fuzzy_query))
                .or(user::Column::Email.like(&fuzzy_query)),
        )
        .limit(10)
        .all(&state.db)
        .await?;

    if fuzzy_matches.is_empty() {
        return Err(ApiError::NOT_FOUND);
    }

    let mut responses: Vec<UserLookupResponse> = Vec::with_capacity(fuzzy_matches.len());

    for user_info in fuzzy_matches {
        let response = UserLookupResponse::parse(user_info, lookup_config.clone(), &state).await;
        responses.push(response);
    }

    Ok(Json(responses))
}
