use crate::{
    ensure_permission,
    entity::{membership, sea_orm_active_enums::UserStatus, user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::user::{
        contacts::invitation_candidates,
        identity::{
            EXACT_MATCH_BONUS, MAX_SEARCH_LEN, PREFIX_MATCH_BONUS, RankableUser,
            SUBSTRING_MATCH_BONUS, SearchTerm, TOKEN_MATCH_PENALTY, WEIGHT_EMAIL, WEIGHT_ID,
            WEIGHT_NAME, WEIGHT_PREFERRED_USERNAME, WEIGHT_USERNAME, WORD_PREFIX_MATCH_BONUS,
            escape_like_pattern, humanize_email_local_part, is_exact_identifier_match,
            is_idp_handle, normalize_search_query, sanitize_display_name, score_candidate,
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
use flow_like_types::{Value, tokio::try_join};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    sea_query::{Alias, Expr, ExprTrait, Func, Order, SimpleExpr, extension::postgres::PgBinOper},
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
    created_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// An exact identifier match, even when lookup settings hide that identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    exact_match: Option<bool>,
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
            exact_match: None,
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

#[derive(Debug, Deserialize, IntoParams)]
pub struct UserSearchQuery {
    #[serde(default)]
    pub limit: Option<u64>,
    /// Exclude people who cannot be invited to this project. Requires Admin.
    #[serde(default)]
    pub app_id: Option<String>,
}

/// `ILIKE` with no wildcards is exactly case-insensitive equality.
fn ilike_eq(column: user::Column, value: &str) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    Expr::col(column).ilike(escape_like_pattern(value))
}

fn ilike_contains(column: user::Column, pattern: &str) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    // PostgreSQL uses backslash as the default LIKE escape. A plain bound string
    // also avoids SeaQuery wrapping `pattern ESCAPE ...` in invalid parentheses.
    Expr::col(column).ilike(pattern)
}

fn exact_identity_condition(value: &str) -> Condition {
    Condition::any()
        .add(ilike_eq(user::Column::Id, value))
        .add(ilike_eq(user::Column::Email, value))
        .add(ilike_eq(user::Column::Username, value))
        .add(ilike_eq(user::Column::PreferredUsername, value))
}

fn exact_identity_score(value: &str) -> SimpleExpr {
    Expr::case(exact_identity_condition(value), 1)
        .finally(0)
        .into()
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

fn word_prefix_pattern(value: &str) -> String {
    let mut escaped = String::from("(^|[^[:alnum:]])");
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn best_field_score(needle: &str) -> SimpleExpr {
    let escaped = escape_like_pattern(needle);
    let prefix = format!("{escaped}%");
    let substring = format!("%{escaped}%");
    let word_prefix = word_prefix_pattern(needle);
    let fields = [
        (user::Column::Name, WEIGHT_NAME),
        (user::Column::PreferredUsername, WEIGHT_PREFERRED_USERNAME),
        (user::Column::Email, WEIGHT_EMAIL),
        (user::Column::Id, WEIGHT_ID),
        (user::Column::Username, WEIGHT_USERNAME),
    ];

    Func::cust(Alias::new("GREATEST"))
        .args(fields.map(|(column, weight)| -> SimpleExpr {
            Expr::case(ilike_eq(column, needle), EXACT_MATCH_BONUS + weight)
                .case(ilike_contains(column, &prefix), PREFIX_MATCH_BONUS + weight)
                .case(
                    Expr::col(column).binary(PgBinOper::RegexCaseInsensitive, word_prefix.clone()),
                    WORD_PREFIX_MATCH_BONUS + weight,
                )
                .case(
                    ilike_contains(column, &substring),
                    SUBSTRING_MATCH_BONUS + weight,
                )
                .finally(0)
                .into()
        }))
        .into()
}

/// Mirrors `score_candidate` before LIMIT. Fetching a wider unordered pool still
/// loses exact names and strong prefixes once enough weaker rows match.
fn candidate_score(term: &SearchTerm) -> SimpleExpr {
    let phrase = best_field_score(&term.lower);
    let best = if term.tokens.is_empty() {
        phrase
    } else {
        let weakest: SimpleExpr = Func::cust(Alias::new("LEAST"))
            .args(term.tokens.iter().map(|token| best_field_score(token)))
            .into();
        Func::cust(Alias::new("GREATEST"))
            .args([phrase, weakest.sub(TOKEN_MATCH_PENALTY)])
            .into()
    };

    best.add(Expr::case(user::Column::Name.is_not_null(), 6).finally(0))
        .add(Expr::case(user::Column::PreferredUsername.is_not_null(), 4).finally(0))
        .add(Expr::case(user::Column::Avatar.is_not_null(), 2).finally(0))
}

fn ranked_candidates(
    query: sea_orm::Select<user::Entity>,
    term: &SearchTerm,
    limit: u64,
) -> sea_orm::Select<user::Entity> {
    query
        .order_by(exact_identity_score(&term.lower), Order::Desc)
        .order_by(candidate_score(term), Order::Desc)
        .order_by_asc(user::Column::Id)
        .limit(limit)
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
    let caller_id = user.executor_scoped_sub()?;
    let lookup_config = state.platform_config.lookup.clone();
    let limit = params
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);

    let invitation_scope = if let Some(app_id) = params.app_id.as_deref() {
        ensure_permission!(user, app_id, &state, RolePermissions::Admin);
        Some(invitation_candidates(app_id, &caller_id))
    } else {
        None
    };

    let normalized = normalize_search_query(&query);
    let trimmed = normalized.as_str();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_SEARCH_LEN {
        return Ok(Json(Vec::new()));
    }

    let scope = executor_search_scope(&state, &user).await?;
    let scoped = |mut query: sea_orm::Select<user::Entity>| {
        if let Some(constraint) = &scope {
            query = query.filter(constraint.clone());
        }
        if let Some(constraint) = &invitation_scope {
            query = query.filter(constraint.clone());
        }
        query.filter(user::Column::Status.ne(UserStatus::Banned))
    };

    // A one-character term only gets exact lookup, never a directory-wide scan.
    let term = SearchTerm::parse(trimmed);

    // Pasting an id or a full email address should resolve even when it is shorter
    // than the substring-search floor.
    let exact_query = scoped(
        user::Entity::find().filter(
            Condition::any()
                .add(exact_identity_condition(trimmed))
                .add(ilike_eq(user::Column::Name, trimmed)),
        ),
    );
    let exact_query = match &term {
        Some(term) => ranked_candidates(exact_query, term, limit),
        None => exact_query
            .order_by(exact_identity_score(trimmed), Order::Desc)
            .order_by_asc(user::Column::Id)
            .limit(limit),
    };
    let fuzzy_query = term.as_ref().map(|term| {
        ranked_candidates(
            scoped(user::Entity::find().filter(search_condition(term))),
            term,
            limit,
        )
    });

    // Pasting an id or address that already resolved needs no substring scan; typing
    // a name still gets one, so near-matches keep showing up alongside an exact hit.
    // Only an identifier-shaped term can skip the scan, so every other term runs both
    // passes at once instead of waiting on a result that cannot change the decision.
    let looks_like_identifier = trimmed.contains('@') || is_idp_handle(trimmed);

    let (exact_matches, fuzzy_matches) = match fuzzy_query {
        Some(fuzzy_query) if !looks_like_identifier => {
            try_join!(exact_query.all(&state.db), fuzzy_query.all(&state.db))?
        }
        Some(fuzzy_query) => {
            let exact_matches = exact_query.all(&state.db).await?;
            let fuzzy_matches = if exact_matches.is_empty() {
                fuzzy_query.all(&state.db).await?
            } else {
                Vec::new()
            };
            (exact_matches, fuzzy_matches)
        }
        None => (exact_query.all(&state.db).await?, Vec::new()),
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
                let rankable = RankableUser {
                    id: &candidate.id,
                    name: candidate.name.as_deref(),
                    preferred_username: candidate.preferred_username.as_deref(),
                    username: candidate.username.as_deref(),
                    email: candidate.email.as_deref(),
                    has_avatar: candidate.avatar.is_some(),
                };
                let score = score_candidate(&rankable, term);
                let exact_identifier = is_exact_identifier_match(&rankable, &term.lower);
                ((exact_identifier, score), candidate)
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
    let normalized_lower = trimmed.to_lowercase();
    let responses = futures::future::join_all(candidates.into_iter().map(|candidate| {
        let exact_match = is_exact_identifier_match(
            &RankableUser {
                id: &candidate.id,
                email: candidate.email.as_deref(),
                username: candidate.username.as_deref(),
                preferred_username: candidate.preferred_username.as_deref(),
                ..Default::default()
            },
            &normalized_lower,
        );
        let lookup_config = lookup_config.clone();
        let state = &state;
        async move {
            let mut response = UserLookupResponse::parse(candidate, lookup_config, state).await;
            response.exact_match = Some(exact_match);
            response
        }
    }))
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

    #[test]
    fn ranking_precedes_the_limit_with_a_stable_tie_breaker() {
        let term = SearchTerm::parse("Felix Schultz").unwrap();
        let sql = ranked_candidates(
            user::Entity::find().filter(search_condition(&term)),
            &term,
            10,
        )
        .build(DbBackend::Postgres)
        .to_string();

        assert!(sql.contains("ORDER BY (CASE WHEN"));
        assert!(sql.contains("GREATEST("));
        assert!(sql.contains("LEAST("));
        assert!(sql.find("ORDER BY") < sql.find("LIMIT"));
        assert!(sql.ends_with(r#"DESC, "User"."id" ASC LIMIT 10"#));
    }

    #[test]
    fn regex_ranking_treats_punctuation_as_literal_text() {
        assert_eq!(
            word_prefix_pattern("a+b.c[0]"),
            r"(^|[^[:alnum:]])a\+b\.c\[0\]"
        );
        assert_eq!(word_prefix_pattern(r"a\b$"), r"(^|[^[:alnum:]])a\\b\$");
    }

    #[test]
    fn invitation_exclusions_are_inside_the_ranked_query() {
        let term = SearchTerm::parse("felix").unwrap();
        let sql = ranked_candidates(
            user::Entity::find()
                .filter(search_condition(&term))
                .filter(invitation_candidates("target", "caller"))
                .filter(user::Column::Status.ne(UserStatus::Banned)),
            &term,
            10,
        )
        .build(DbBackend::Postgres)
        .to_string();

        assert_eq!(sql.matches("NOT IN (SELECT").count(), 2);
        assert!(sql.contains(r#""User"."id" <> 'caller'"#));
        assert!(sql.contains(r#""User"."status" <> 'BANNED'"#));
        assert!(sql.find("Invitation") < sql.find("ORDER BY"));
    }

    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_USER_SEARCH_TEST_DATABASE_URL pointing to an empty disposable PostgreSQL database"]
    async fn database_ranking_keeps_late_exact_matches_and_excludes_existing_invitees() {
        use sea_orm::{ConnectionTrait, Database, TransactionTrait};

        let url = std::env::var("FLOW_LIKE_USER_SEARCH_TEST_DATABASE_URL").unwrap();
        let db = Database::connect(url).await.unwrap();
        let txn = db.begin().await.unwrap();
        txn.execute_unprepared(
            r#"
            CREATE TABLE public."User" (
                id text PRIMARY KEY, name text, email text, username text,
                "preferredUsername" text, avatar text, status text NOT NULL
            );
            CREATE TABLE public."Membership" ("userId" text NOT NULL, "appId" text NOT NULL);
            CREATE TABLE public."Invitation" ("userId" text NOT NULL, "appId" text NOT NULL);
            INSERT INTO public."User" (id, name, status)
                SELECT 'noise-' || n, 'Unfelixlike ' || n, 'Active' FROM generate_series(1, 2000) n;
            INSERT INTO public."User" (id, name, status) VALUES
                ('exact', 'Felix', 'Active'),
                ('prefix', 'Felix Schultz', 'Active'),
                ('word', 'Dr. Felix Schultz', 'Active'),
                ('member', 'Felix', 'Active'),
                ('invitee', 'Felix', 'Active'),
                ('caller', 'Felix', 'Active'),
                ('banned', 'Felix', 'BANNED');
            INSERT INTO public."Membership" VALUES ('member', 'target');
            INSERT INTO public."Invitation" VALUES ('invitee', 'target');
            INSERT INTO public."User" (id, name, status)
                SELECT 'same-name-' || n, 'person-key', 'Active' FROM generate_series(1, 50) n;
            INSERT INTO public."User" (id, name, status) VALUES
                ('person-key', 'New Person', 'Active'),
                ('literal', 'A+B.[100%]', 'Active'),
                ('wildcard-noise', 'AB.100000', 'Active');
        "#,
        )
        .await
        .unwrap();

        let term = SearchTerm::parse("felix").unwrap();
        let ids = ranked_candidates(
            user::Entity::find()
                .filter(search_condition(&term))
                .filter(invitation_candidates("target", "caller"))
                .filter(user::Column::Status.ne(UserStatus::Banned)),
            &term,
            3,
        )
        .select_only()
        .column(user::Column::Id)
        .into_tuple::<String>()
        .all(&txn)
        .await
        .unwrap();
        assert_eq!(ids, ["exact", "prefix", "word"]);

        let identifier = SearchTerm::parse("person-key").unwrap();
        let ids = ranked_candidates(
            user::Entity::find().filter(
                Condition::any()
                    .add(exact_identity_condition(&identifier.raw))
                    .add(ilike_eq(user::Column::Name, &identifier.raw)),
            ),
            &identifier,
            1,
        )
        .select_only()
        .column(user::Column::Id)
        .into_tuple::<String>()
        .all(&txn)
        .await
        .unwrap();
        assert_eq!(ids, ["person-key"]);

        let literal = SearchTerm::parse("A+B.[100%]").unwrap();
        let ids = ranked_candidates(
            user::Entity::find().filter(search_condition(&literal)),
            &literal,
            10,
        )
        .select_only()
        .column(user::Column::Id)
        .into_tuple::<String>()
        .all(&txn)
        .await
        .unwrap();
        assert_eq!(ids, ["literal"]);

        // Database and merge ranking must agree before each query is capped.
        for query in [
            "felix",
            "Felix Schultz",
            "Schultz Felix",
            "@Felix",
            "Felix.",
        ] {
            let term = SearchTerm::parse(query).unwrap();
            let scores = user::Entity::find()
                .filter(search_condition(&term))
                .filter(user::Column::Id.is_in(["exact", "prefix", "word"]))
                .select_only()
                .column(user::Column::Id)
                .column(user::Column::Name)
                .expr(candidate_score(&term))
                .into_tuple::<(String, String, i32)>()
                .all(&txn)
                .await
                .unwrap();
            assert!(!scores.is_empty());
            for (id, name, database_score) in scores {
                let candidate = RankableUser {
                    id: &id,
                    name: Some(&name),
                    ..Default::default()
                };
                assert_eq!(
                    database_score,
                    score_candidate(&candidate, &term),
                    "{query}: {name}"
                );
            }
        }
        txn.rollback().await.unwrap();
    }
}
