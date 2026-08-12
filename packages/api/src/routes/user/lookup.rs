use crate::{
    entity::{membership, sea_orm_active_enums::UserStatus, user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::user::{
        identity::{RankableUser, SearchTerm, escape_like_pattern, is_idp_handle, score_candidate},
        sign_avatar,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::hub::Lookup;
use flow_like_types::Value;
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect,
    sea_query::{Expr, LikeExpr, extension::postgres::PgExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

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
#[tracing::instrument(name = "GET /user/lookup/{sub}", skip_all)]
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

/// Executor tokens belong to a running flow, so they see the app's members and
/// nobody else — the same boundary `scoped_lookup_ids` enforces for lookups.
async fn scope_search_candidates(
    state: &AppState,
    user: &AppUser,
    candidates: Vec<user::Model>,
) -> Result<Vec<user::Model>, ApiError> {
    let AppUser::Executor(executor) = user else {
        return Ok(candidates);
    };

    ensure_executor_lookup_permission(state, user, &executor.app_id).await?;
    if candidates.is_empty() {
        return Ok(candidates);
    }

    let members = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&executor.app_id))
        .filter(
            membership::Column::UserId
                .is_in(candidates.iter().map(|candidate| candidate.id.clone())),
        )
        .limit(MAX_CANDIDATE_POOL)
        .all(&state.db)
        .await?
        .into_iter()
        .map(|membership| membership.user_id)
        .collect::<HashSet<_>>();

    Ok(candidates
        .into_iter()
        .filter(|candidate| members.contains(&candidate.id))
        .collect())
}

const DEFAULT_SEARCH_LIMIT: u64 = 10;
const MAX_SEARCH_LIMIT: u64 = 25;
/// Ranking only sees what the database returns, so the candidate pool is wider
/// than the response — otherwise an arbitrary unordered slice decides the winners.
const CANDIDATE_POOL_FACTOR: u64 = 8;
const MAX_CANDIDATE_POOL: u64 = 200;

#[derive(Debug, Deserialize, IntoParams)]
pub struct UserSearchQuery {
    #[serde(default)]
    pub limit: Option<u64>,
}

/// `ILIKE` with no wildcards is exactly case-insensitive equality.
fn ilike_eq(column: user::Column, value: &str) -> sea_orm::sea_query::SimpleExpr {
    Expr::col(column).ilike(LikeExpr::new(escape_like_pattern(value)).escape('\\'))
}

fn ilike_contains(column: user::Column, pattern: &str) -> sea_orm::sea_query::SimpleExpr {
    Expr::col(column).ilike(LikeExpr::new(pattern).escape('\\'))
}

#[utoipa::path(
    get,
    path = "/user/search/{query}",
    tag = "user",
    params(
        ("query" = String, Path, description = "Name, handle, email or user ID to search for"),
        UserSearchQuery
    ),
    responses(
        (status = 200, description = "Users matching the search query, best match first", body = Vec<UserLookupResponse>),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/search/{query}", skip_all)]
pub async fn user_search(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(query): Path<String>,
    Query(params): Query<UserSearchQuery>,
) -> Result<Json<Vec<UserLookupResponse>>, ApiError> {
    user.executor_scoped_sub()?;
    let lookup_config = state.platform_config.lookup.clone();
    let limit = params
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Json(Vec::new()));
    }

    // Pasting an id or a full email address should resolve even when it is shorter
    // than the substring-search floor.
    let exact_matches = user::Entity::find()
        .filter(
            Condition::any()
                .add(user::Column::Id.eq(trimmed))
                .add(ilike_eq(user::Column::Email, trimmed))
                .add(ilike_eq(user::Column::Username, trimmed))
                .add(ilike_eq(user::Column::PreferredUsername, trimmed)),
        )
        .filter(user::Column::Status.ne(UserStatus::Banned))
        .limit(MAX_SEARCH_LIMIT)
        .all(&state.db)
        .await?;

    // Pasting an id or address that already resolved needs no substring scan; typing
    // a name still gets one, so near-matches keep showing up alongside an exact hit.
    let resolved_identifier =
        !exact_matches.is_empty() && (trimmed.contains('@') || is_idp_handle(trimmed));

    // A one-character term matches most of the table, so it is not worth scanning for.
    let term = SearchTerm::parse(trimmed);
    let fuzzy_matches = match &term {
        Some(_) if resolved_identifier => Vec::new(),
        Some(term) => {
            let pattern = term.like_pattern();
            let pool = (limit * CANDIDATE_POOL_FACTOR).min(MAX_CANDIDATE_POOL);
            user::Entity::find()
                .filter(
                    Condition::any()
                        .add(ilike_contains(user::Column::Name, &pattern))
                        .add(ilike_contains(user::Column::PreferredUsername, &pattern))
                        .add(ilike_contains(user::Column::Email, &pattern))
                        .add(ilike_contains(user::Column::Username, &pattern)),
                )
                .filter(user::Column::Status.ne(UserStatus::Banned))
                .limit(pool)
                .all(&state.db)
                .await?
        }
        None => Vec::new(),
    };

    let mut seen = HashSet::with_capacity(exact_matches.len() + fuzzy_matches.len());
    let mut candidates: Vec<user::Model> = Vec::with_capacity(seen.capacity());
    for candidate in exact_matches.into_iter().chain(fuzzy_matches) {
        if seen.insert(candidate.id.clone()) {
            candidates.push(candidate);
        }
    }

    let mut candidates = scope_search_candidates(&state, &user, candidates).await?;
    if candidates.is_empty() {
        return Ok(Json(Vec::new()));
    }

    // Without a term the exact pass already decided the set; ranking is a no-op.
    if let Some(term) = &term {
        let mut ranked = candidates
            .into_iter()
            .map(|candidate| {
                let score = score_candidate(
                    &RankableUser {
                        id: &candidate.id,
                        name: candidate.name.as_deref(),
                        preferred_username: candidate.preferred_username.as_deref(),
                        username: candidate.username.as_deref(),
                        email: candidate.email.as_deref(),
                        has_avatar: candidate.avatar.is_some(),
                    },
                    term,
                );
                (score, candidate)
            })
            .collect::<Vec<_>>();

        // Ties break on id so paging over a stable dataset stays stable.
        ranked.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.id.cmp(&right.id))
        });

        candidates = ranked
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect::<Vec<_>>();
    }

    candidates.truncate(limit as usize);

    // Each response signs an avatar URL against the object store; serially that is
    // one round trip per result.
    let responses = futures::future::join_all(
        candidates
            .into_iter()
            .map(|candidate| UserLookupResponse::parse(candidate, lookup_config.clone(), &state)),
    )
    .await;

    Ok(Json(responses))
}
