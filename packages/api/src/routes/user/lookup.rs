use crate::{
    entity::{membership, sea_orm_active_enums::UserStatus, user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::user::{
        identity::{
            RankableUser, SearchTerm, escape_like_pattern, humanize_email_local_part,
            is_idp_handle, sanitize_display_name, score_candidate,
        },
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
    ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect, QueryTrait,
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

        // Rows provisioned before display names were derived — and every row
        // `ensure_user_exists` creates — carry no name, and with `lookup.email` off
        // there is then nothing left to render but the raw id. `derive_display_name`
        // already treats the email local part as the last rung of that ladder, so
        // reuse it here instead of showing a uuid where a person belongs. The domain
        // never leaves the server, so this stays inside the `email` opt-out.
        let name = lookup_config
            .name
            .then(|| {
                user.name
                    .as_deref()
                    .and_then(sanitize_display_name)
                    .or_else(|| user.email.as_deref().and_then(humanize_email_local_part))
            })
            .flatten();

        UserLookupResponse {
            id: user.id,
            email: lookup_config.email.then_some(user.email).flatten(),
            username: lookup_config.username.then_some(user.username).flatten(),
            preferred_username: lookup_config
                .preferred_username
                .then_some(user.preferred_username)
                .flatten(),
            name,
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

/// The app-membership constraint as SQL, so it can join the candidate queries
/// instead of filtering what they already returned.
fn membership_scope(app_id: &str) -> sea_orm::sea_query::SimpleExpr {
    user::Column::Id.in_subquery(
        membership::Entity::find()
            .select_only()
            .column(membership::Column::UserId)
            .filter(membership::Column::AppId.eq(app_id))
            .into_query(),
    )
}

/// Executor tokens belong to a running flow, so they see the app's members and
/// nobody else — the same boundary `scoped_lookup_ids` enforces for lookups.
///
/// The constraint has to be part of the candidate queries rather than a pass over
/// their results: both of them are capped, and in a large directory the cap can
/// fill up with non-members long before the app's own members are reached, which
/// would answer "no such user" for a colleague sitting in the same app.
async fn executor_search_scope(
    state: &AppState,
    user: &AppUser,
) -> Result<Option<sea_orm::sea_query::SimpleExpr>, ApiError> {
    let AppUser::Executor(executor) = user else {
        return Ok(None);
    };

    ensure_executor_lookup_permission(state, user, &executor.app_id).await?;
    Ok(Some(membership_scope(&executor.app_id)))
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

/// The columns a human search term can legitimately land in. `username` is the
/// pool-internal handle, but a pasted one still has to resolve.
fn any_column_contains(pattern: &str) -> Condition {
    Condition::any()
        .add(ilike_contains(user::Column::Name, pattern))
        .add(ilike_contains(user::Column::PreferredUsername, pattern))
        .add(ilike_contains(user::Column::Email, pattern))
        .add(ilike_contains(user::Column::Username, pattern))
}

/// Every token has to land somewhere, but not all of them in the same column.
/// Matching the phrase as one contiguous string is what made "Felix Schultz" miss
/// `name = 'Schultz, Felix'` and `email = 'felix.schultz@…'` — the two places a
/// directory most often keeps a person.
fn search_condition(term: &SearchTerm) -> Condition {
    let patterns = term.token_patterns();
    if patterns.is_empty() {
        return any_column_contains(&term.like_pattern());
    }

    patterns
        .iter()
        .fold(Condition::all(), |condition, pattern| {
            condition.add(any_column_contains(pattern))
        })
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

    let scope = executor_search_scope(&state, &user).await?;
    let scoped = |query: sea_orm::Select<user::Entity>| match &scope {
        Some(constraint) => query.filter(constraint.clone()),
        None => query,
    };

    // Pasting an id or a full email address should resolve even when it is shorter
    // than the substring-search floor.
    let exact_matches = scoped(
        user::Entity::find()
            .filter(
                Condition::any()
                    .add(user::Column::Id.eq(trimmed))
                    .add(ilike_eq(user::Column::Email, trimmed))
                    .add(ilike_eq(user::Column::Username, trimmed))
                    .add(ilike_eq(user::Column::PreferredUsername, trimmed)),
            )
            .filter(user::Column::Status.ne(UserStatus::Banned)),
    )
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
            let pool = (limit * CANDIDATE_POOL_FACTOR).min(MAX_CANDIDATE_POOL);
            scoped(
                user::Entity::find()
                    .filter(search_condition(term))
                    .filter(user::Column::Status.ne(UserStatus::Banned)),
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::DbBackend;

    fn fuzzy_sql(query: &str) -> String {
        let term = SearchTerm::parse(query).unwrap();
        user::Entity::find()
            .filter(search_condition(&term))
            .build(DbBackend::Postgres)
            .to_string()
    }

    #[test]
    fn a_single_token_matches_the_phrase_across_every_column() {
        let sql = fuzzy_sql("felix");
        assert_eq!(sql.matches("ILIKE").count(), 4);
        assert_eq!(sql.matches("'%felix%'").count(), 4);
    }

    #[test]
    fn every_token_gets_its_own_column_group() {
        let sql = fuzzy_sql("Felix Schultz");
        // Each token is ORed across the columns, and the groups are ANDed — a row
        // has to carry both halves, but not in the same column.
        assert_eq!(sql.matches("'%felix%'").count(), 4);
        assert_eq!(sql.matches("'%schultz%'").count(), 4);
        assert!(!sql.contains("'%Felix Schultz%'"));
    }

    #[test]
    fn a_typed_address_keeps_its_domain_whole() {
        let sql = fuzzy_sql("felix.schultz@corp.de");
        assert_eq!(sql.matches("'%corp.de%'").count(), 4);
        // A bare `de` token would match most of the directory.
        assert!(!sql.contains("'%de%'"));
    }

    #[test]
    fn an_executor_search_is_scoped_before_the_candidate_cap() {
        let sql = user::Entity::find()
            .filter(search_condition(&SearchTerm::parse("felix").unwrap()))
            .filter(membership_scope("app-1"))
            .limit(200)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""Membership""#));
        assert!(sql.contains("'app-1'"));
        // The membership subquery has to sit inside the query the cap applies to,
        // or the cap decides the pool before scoping ever sees it.
        assert!(sql.find("Membership") < sql.find("LIMIT"));
    }

    #[test]
    fn tokenizing_does_not_lose_like_escaping() {
        let term = SearchTerm::parse("100% off").unwrap();
        assert_eq!(term.token_patterns(), [r"%100\%%", "%off%"]);
    }
}
