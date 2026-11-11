use crate::{
    ensure_permission,
    entity::{bot, membership, sea_orm_active_enums::UserStatus, user},
    error::ApiError,
    middleware::jwt::{AppUser, ExecutorUse},
    permission::role_permission::RolePermissions,
    routes::user::{
        contacts::invitation_candidates,
        identity::{
            EXACT_MATCH_BONUS, MAX_SEARCH_LEN, PREFIX_MATCH_BONUS, RankableUser,
            SUBSTRING_MATCH_BONUS, SearchTerm, TOKEN_MATCH_PENALTY, WEIGHT_EMAIL, WEIGHT_ID,
            WEIGHT_NAME, WEIGHT_PREFERRED_USERNAME, WEIGHT_USERNAME, WORD_PREFIX_MATCH_BONUS,
            escape_like_pattern, is_email_like, is_exact_identifier_match, is_idp_handle,
            normalize_search_query, public_display_name, public_handle, score_candidate,
            shared_email,
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
use flow_like_types::{Value, bot::BotSystemKind, tokio::try_join};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    availability: Option<flow_like_types::user_availability::PublicAvailability>,
    description: Option<String>,
    created_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// An exact identifier match, even when lookup settings hide that identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    exact_match: Option<bool>,
    /// Set when the id is a bot's sub. A bot acts for its owner and has no email,
    /// username or avatar URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bot: Option<BotLookup>,
}

/// The bot behind a looked-up sub.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct BotLookup {
    /// The person the bot acts for.
    pub owner_id: String,
    /// `general` (Assistant) or `flux` for built-in bots, null otherwise. A built-in bot
    /// without a stored name shows its localized default name.
    #[schema(value_type = Option<String>, example = "general")]
    pub system: Option<BotSystemKind>,
    /// The look from the bot builder: an object with a string `kind`.
    #[schema(value_type = Object, example = json!({ "kind": "pip" }))]
    pub appearance: Value,
    /// The pinned text-to-speech Bit ID, so call participants can use the bot's voice.
    /// Provider configuration and credentials stay private. Deleted bots expose no voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_model: Option<String>,
    /// The bot was deleted; what it did still shows it.
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UserBatchLookupBody {
    pub user_ids: Vec<String>,
}

impl UserLookupResponse {
    /// The lookup entry of a bot sub, deleted bots included. The hub's lookup settings hide
    /// the description and creation time as they do for people; the name and `bot` stay.
    pub fn from_bot(row: bot::Model, lookup: &Lookup) -> Self {
        UserLookupResponse {
            id: row.id,
            email: None,
            username: None,
            preferred_username: None,
            name: row.name,
            avatar_url: None,
            additional_information: None,
            availability: None,
            description: lookup.description.then_some(row.description).flatten(),
            created_at: lookup.created_at.then_some(row.created_at),
            exact_match: None,
            bot: Some(BotLookup {
                owner_id: row.owner_id,
                system: row.system_key.as_deref().and_then(BotSystemKind::parse),
                appearance: row.appearance,
                tts_model: if row.deleted_at.is_none() {
                    row.tts_model
                } else {
                    None
                },
                deleted: row.deleted_at.is_some(),
            }),
        }
    }

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

        Self::project(user, &lookup_config, avatar_url)
    }

    /// The directory entry of `user`: only the fields `lookup` shares, never
    /// account, billing or permission data. `avatar_url` is the signed avatar.
    pub fn project(user: user::Model, lookup: &Lookup, avatar_url: Option<String>) -> Self {
        let availability = flow_like_types::user_availability::AvailabilityState::read(
            user.additional_information.as_ref(),
        )
        .current(chrono::Utc::now().timestamp_millis())
        .without_calendar_schedule();
        // Rows provisioned before display names were derived — and every row
        // `ensure_user_exists` creates — carry no name, and with the address hidden
        // there is then nothing left to render but the raw id. `derive_display_name`
        // already treats the email local part as the last rung of that ladder, so
        // reuse it here instead of showing a uuid where a person belongs. The domain
        // never leaves the server. The handle stays its own field, so it is not a
        // rung here.
        let name = lookup
            .name
            .then(|| public_display_name(user.name.as_deref(), None, user.email.as_deref()))
            .flatten();
        // Sign-ins that map the address into the handle, and directories whose login
        // name is the address, must not share it through a field the person did not
        // choose to share it in.
        let preferred_username = lookup
            .preferred_username
            .then(|| user.preferred_username.as_deref().and_then(public_handle))
            .flatten();
        let username = lookup
            .username
            .then_some(user.username)
            .flatten()
            .filter(|username| !is_email_like(username));

        UserLookupResponse {
            id: user.id,
            email: shared_email(lookup, user.share_email, user.email),
            username,
            preferred_username,
            name,
            avatar_url: lookup.avatar.then_some(avatar_url).flatten(),
            additional_information: lookup
                .additional_information
                .then_some(flow_like_types::user_availability::public_information(
                    user.additional_information,
                ))
                .flatten(),
            availability: Some(availability),
            description: lookup.description.then_some(user.description).flatten(),
            created_at: lookup.created_at.then_some(user.created_at),
            exact_match: None,
            bot: None,
        }
    }
}

#[utoipa::path(
    get,
    path = "/user/lookup/{sub}",
    tag = "user",
    params(
        ("sub" = String, Path, description = "User or bot ID to look up")
    ),
    responses(
        (status = 200, description = "The user, or the bot with its owner when the ID is a bot's", body = UserLookupResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No user or bot has this ID")
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

    if let Some(found_bot) = bot::Entity::find_by_id(sub).one(&state.db).await? {
        return Ok(Json(UserLookupResponse::from_bot(
            found_bot,
            &lookup_config,
        )));
    }

    Err(ApiError::NOT_FOUND)
}

#[utoipa::path(
    post,
    path = "/user/lookup",
    tag = "user",
    request_body = UserBatchLookupBody,
    responses(
        (status = 200, description = "Users and bots found for the requested IDs; IDs nobody has are left out", body = Vec<UserLookupResponse>),
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
        .filter(user::Column::Id.is_in(ids.iter().cloned()))
        .limit(100)
        .all(&state.db)
        .await?;
    let found_ids = found_users
        .iter()
        .map(|user_info| user_info.id.as_str())
        .collect::<HashSet<_>>();
    let misses = ids
        .iter()
        .filter(|id| !found_ids.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let found_bots = crate::routes::bots::bots_by_id(&state.db, misses).await?;

    let mut responses = Vec::with_capacity(found_users.len() + found_bots.len());
    for user_info in found_users {
        responses.push(UserLookupResponse::parse(user_info, lookup_config.clone(), &state).await);
    }
    responses.extend(
        found_bots
            .into_iter()
            .map(|row| UserLookupResponse::from_bot(row, &lookup_config)),
    );

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
    let permission = user
        .execution_app_permission(app_id, state, ExecutorUse::Read)
        .await?;
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

/// A name or handle that holds an address is shown as something else, so a part of
/// that address must not find it either.
fn shown_text_contains(column: user::Column, pattern: &str) -> Condition {
    Condition::all()
        .add(ilike_contains(column, pattern))
        .add(Expr::col(column).not_like("%@%"))
}

/// The address a result shows, as [`shared_email`] decides it: every member's where
/// the hub shows them, otherwise only those whose owner shares it. A hidden address
/// reads as NULL, so no part of it finds or ranks its account.
fn shown_email(lookup: &Lookup) -> SimpleExpr {
    let email = Expr::col(user::Column::Email);
    if lookup.email {
        return email.into();
    }
    Expr::case(user::Column::ShareEmail.eq(true), email).into()
}

/// The columns a typed term may be found in: what a result shows. An address or
/// login name that is hidden only resolves when it is pasted whole, through
/// `exact_identity_condition` — a substring match on it would let anyone list the
/// accounts behind a mail domain.
fn any_column_contains(pattern: &str, lookup: &Lookup) -> Condition {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    let mut condition = Condition::any()
        .add(shown_text_contains(user::Column::Name, pattern))
        .add(shown_text_contains(
            user::Column::PreferredUsername,
            pattern,
        ))
        .add(shown_email(lookup).ilike(pattern));
    if lookup.username {
        condition = condition.add(ilike_contains(user::Column::Username, pattern));
    }
    condition
}

/// Every token has to land somewhere, but not all of them in the same column.
/// Matching the phrase as one contiguous string is what made "Felix Schultz" miss
/// `name = 'Schultz, Felix'`.
fn search_condition(term: &SearchTerm, lookup: &Lookup) -> Condition {
    let patterns = term.token_patterns();
    if patterns.is_empty() {
        return any_column_contains(&term.like_pattern(), lookup);
    }

    patterns
        .iter()
        .fold(Condition::all(), |condition, pattern| {
            condition.add(any_column_contains(pattern, lookup))
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

/// What a result is ranked on, with the weights. A hidden address or login name
/// ranks nothing, or the order of the results would say whose it is.
fn ranked_fields(lookup: &Lookup) -> Vec<(SimpleExpr, i32)> {
    let column = |column: user::Column| -> SimpleExpr { Expr::col(column).into() };
    let mut fields = vec![
        (column(user::Column::Name), WEIGHT_NAME),
        (
            column(user::Column::PreferredUsername),
            WEIGHT_PREFERRED_USERNAME,
        ),
        (shown_email(lookup), WEIGHT_EMAIL),
        (column(user::Column::Id), WEIGHT_ID),
    ];
    if lookup.username {
        fields.push((column(user::Column::Username), WEIGHT_USERNAME));
    }
    fields
}

/// The Rust side of [`ranked_fields`].
fn ranked_candidate<'a>(candidate: &'a user::Model, lookup: &Lookup) -> RankableUser<'a> {
    let email_shown = lookup.email || candidate.share_email == Some(true);
    RankableUser {
        id: &candidate.id,
        name: candidate.name.as_deref(),
        preferred_username: candidate.preferred_username.as_deref(),
        username: candidate.username.as_deref().filter(|_| lookup.username),
        email: candidate.email.as_deref().filter(|_| email_shown),
        has_avatar: candidate.avatar.is_some(),
    }
}

/// Every identifier a pasted term may equal, whether a result shows it or not.
fn identifiers(candidate: &user::Model) -> RankableUser<'_> {
    RankableUser {
        id: &candidate.id,
        email: candidate.email.as_deref(),
        username: candidate.username.as_deref(),
        preferred_username: candidate.preferred_username.as_deref(),
        ..Default::default()
    }
}

fn best_field_score(needle: &str, lookup: &Lookup) -> SimpleExpr {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    let escaped = escape_like_pattern(needle);
    let prefix = format!("{escaped}%");
    let substring = format!("%{escaped}%");
    let word_prefix = word_prefix_pattern(needle);

    Func::cust(Alias::new("GREATEST"))
        .args(
            ranked_fields(lookup)
                .into_iter()
                .map(|(field, weight)| -> SimpleExpr {
                    Expr::case(
                        field.clone().ilike(escaped.as_str()),
                        EXACT_MATCH_BONUS + weight,
                    )
                    .case(
                        field.clone().ilike(prefix.as_str()),
                        PREFIX_MATCH_BONUS + weight,
                    )
                    .case(
                        field
                            .clone()
                            .binary(PgBinOper::RegexCaseInsensitive, word_prefix.clone()),
                        WORD_PREFIX_MATCH_BONUS + weight,
                    )
                    .case(
                        field.ilike(substring.as_str()),
                        SUBSTRING_MATCH_BONUS + weight,
                    )
                    .finally(0)
                    .into()
                }),
        )
        .into()
}

/// Mirrors `score_candidate` before LIMIT. Fetching a wider unordered pool still
/// loses exact names and strong prefixes once enough weaker rows match.
fn candidate_score(term: &SearchTerm, lookup: &Lookup) -> SimpleExpr {
    let phrase = best_field_score(&term.lower, lookup);
    let best = if term.tokens.is_empty() {
        phrase
    } else {
        let weakest: SimpleExpr = Func::cust(Alias::new("LEAST"))
            .args(
                term.tokens
                    .iter()
                    .map(|token| best_field_score(token, lookup)),
            )
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
    lookup: &Lookup,
    limit: u64,
) -> sea_orm::Select<user::Entity> {
    query
        .order_by(exact_identity_score(&term.lower), Order::Desc)
        .order_by(candidate_score(term, lookup), Order::Desc)
        .order_by_asc(user::Column::Id)
        .limit(limit)
}

#[utoipa::path(
    get,
    path = "/user/search/{query}",
    tag = "user",
    params(
        ("query" = String, Path, description = "Name or handle to search for, or a complete email address or user ID. Part of an address only finds people who show their email."),
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
        Some(term) => ranked_candidates(exact_query, term, &lookup_config, limit),
        None => exact_query
            .order_by(exact_identity_score(trimmed), Order::Desc)
            .order_by_asc(user::Column::Id)
            .limit(limit),
    };
    let fuzzy_query = term.as_ref().map(|term| {
        ranked_candidates(
            scoped(user::Entity::find().filter(search_condition(term, &lookup_config))),
            term,
            &lookup_config,
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
                let score = score_candidate(&ranked_candidate(&candidate, &lookup_config), term);
                let exact_identifier =
                    is_exact_identifier_match(&identifiers(&candidate), &term.lower);
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
        let exact_match = is_exact_identifier_match(&identifiers(&candidate), &normalized_lower);
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

    /// A hub that shows every member's address and login name.
    fn identifiers_shown() -> Lookup {
        Lookup {
            email: true,
            username: true,
            ..Lookup::default()
        }
    }

    /// The WHERE clause of a fuzzy search. The select list names every column, so
    /// only the condition says which ones a term is looked for in.
    fn fuzzy_sql(query: &str, lookup: &Lookup) -> String {
        let term = SearchTerm::parse(query).unwrap();
        let sql = user::Entity::find()
            .filter(search_condition(&term, lookup))
            .build(DbBackend::Postgres)
            .to_string();
        let (_, condition) = sql.split_once(" WHERE ").expect("a filtered query");
        condition.to_string()
    }

    #[test]
    fn a_single_token_matches_the_phrase_across_every_shown_column() {
        let sql = fuzzy_sql("felix", &identifiers_shown());
        assert_eq!(sql.matches("ILIKE").count(), 4);
        assert_eq!(sql.matches("'%felix%'").count(), 4);
    }

    /// True when `sql` compares the address, and only ever as
    /// `CASE WHEN "shareEmail" … THEN "email" END`: behind its owner's choice.
    fn address_only_when_shared(sql: &str) -> bool {
        let mentions = sql.match_indices(r#""email""#).collect::<Vec<_>>();
        !mentions.is_empty()
            && mentions.into_iter().all(|(at, _)| {
                let before = sql[..at].trim_end_matches(|c: char| c == '(' || c.is_whitespace());
                before.ends_with("THEN")
                    && before
                        .rfind("CASE WHEN")
                        .is_some_and(|case| before[case..].contains(r#""shareEmail""#))
            })
    }

    #[test]
    fn a_hidden_address_or_login_name_is_not_found_by_a_part_of_it() {
        let hidden = Lookup::default();
        assert!(!hidden.email && !hidden.username);
        for query in ["felix", "gmail.com", "felix.schultz@corp.de"] {
            let sql = fuzzy_sql(query, &hidden);
            assert!(address_only_when_shared(&sql), "{query}: {sql}");
            assert!(!sql.contains(r#""username""#), "{query}: {sql}");
        }

        // An address kept in the name or the handle is not shown, so it is not
        // searched: both columns only match values without an `@`.
        let sql = fuzzy_sql("gmail", &hidden);
        assert_eq!(sql.matches("ILIKE").count(), 3);
        assert_eq!(sql.matches("NOT LIKE '%@%'").count(), 2);
        assert!(sql.contains(r#""name" NOT LIKE '%@%'"#), "{sql}");
        assert!(
            sql.contains(r#""preferredUsername" NOT LIKE '%@%'"#),
            "{sql}"
        );

        let hub_wide = Lookup {
            email: true,
            ..Lookup::default()
        };
        let sql = fuzzy_sql("gmail", &hub_wide);
        assert!(sql.contains(r#""email" ILIKE '%gmail%'"#), "{sql}");
        assert!(!sql.contains("shareEmail"), "{sql}");
        assert!(!sql.contains(r#""username""#), "{sql}");
    }

    #[test]
    fn a_hidden_address_or_login_name_does_not_rank_a_result() {
        let term = SearchTerm::parse("felix").unwrap();
        let score = |lookup: &Lookup| {
            user::Entity::find()
                .select_only()
                .expr(candidate_score(&term, lookup))
                .build(DbBackend::Postgres)
                .to_string()
        };
        let hidden = score(&Lookup::default());
        assert!(address_only_when_shared(&hidden), "{hidden}");
        assert!(!hidden.contains(r#""username""#), "{hidden}");
        assert!(hidden.contains(r#""name""#), "{hidden}");
        assert!(hidden.contains(r#""id""#), "{hidden}");
        let shown = score(&identifiers_shown());
        assert!(shown.contains(r#""email""#), "{shown}");
        assert!(!shown.contains("shareEmail"), "{shown}");
        assert!(shown.contains(r#""username""#), "{shown}");

        let row = user::Model {
            name: Some("Someone".into()),
            preferred_username: None,
            email: Some("felix@example.com".into()),
            username: Some("felix".into()),
            ..account_row()
        };
        for not_shared in [None, Some(false)] {
            let row = user::Model {
                share_email: not_shared,
                ..row.clone()
            };
            assert_eq!(
                score_candidate(&ranked_candidate(&row, &Lookup::default()), &term),
                0
            );
        }
        let shared = user::Model {
            share_email: Some(true),
            ..row.clone()
        };
        assert!(score_candidate(&ranked_candidate(&shared, &Lookup::default()), &term) > 0);
        assert!(score_candidate(&ranked_candidate(&row, &identifiers_shown()), &term) > 0);
        // A pasted address still resolves the account it belongs to.
        assert!(is_exact_identifier_match(
            &identifiers(&row),
            "felix@example.com"
        ));
    }

    #[test]
    fn every_token_gets_its_own_column_group() {
        let sql = fuzzy_sql("Felix Schultz", &identifiers_shown());
        // Each token is ORed across the columns, and the groups are ANDed — a row
        // has to carry both halves, but not in the same column.
        assert_eq!(sql.matches("'%felix%'").count(), 4);
        assert_eq!(sql.matches("'%schultz%'").count(), 4);
        assert!(!sql.contains("'%Felix Schultz%'"));
    }

    #[test]
    fn a_typed_address_keeps_its_domain_whole() {
        let sql = fuzzy_sql("felix.schultz@corp.de", &identifiers_shown());
        assert_eq!(sql.matches("'%corp.de%'").count(), 4);
        // A bare `de` token would match most of the directory.
        assert!(!sql.contains("'%de%'"));
    }

    #[test]
    fn an_executor_search_is_scoped_before_the_candidate_cap() {
        let sql = user::Entity::find()
            .filter(search_condition(
                &SearchTerm::parse("felix").unwrap(),
                &Lookup::default(),
            ))
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

    fn account_row() -> user::Model {
        let created = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .fixed_offset();
        user::Model {
            id: "user-1".into(),
            stripe_id: Some("cus_1".into()),
            subscription_period_start: Some(created),
            subscription_period_end: Some(created),
            billing_period_anchor: Some(created),
            subscription_id: Some("sub_1".into()),
            subscription_event_created_at: Some(1),
            subscription_event_id: Some("evt_1".into()),
            subscription_sync_revision: Some(1),
            tracking_id: Some("tracking-1".into()),
            email: Some("person@example.com".into()),
            username: Some("person".into()),
            preferred_username: Some("person-handle".into()),
            name: Some("Person".into()),
            description: Some("About".into()),
            avatar: Some("avatar-1".into()),
            additional_information: Some(serde_json::json!({ "team": "ops" })),
            permission: 7,
            accepted_terms_version: Some("v1".into()),
            tutorial_completed: true,
            total_size: 1,
            total_llm_price: 2,
            total_embedding_price: 3,
            llm_price_tracking_month: Some("2026-10".into()),
            created_at: created,
            updated_at: created,
            dev_mode: true,
            status: UserStatus::Active,
            tier: crate::entity::sea_orm_active_enums::UserTier::Pro,
            share_email: None,
        }
    }

    fn projected_row(row: user::Model, lookup: &Lookup) -> serde_json::Map<String, Value> {
        let response =
            UserLookupResponse::project(row, lookup, Some("https://signed.example/avatar".into()));
        match serde_json::to_value(response).unwrap() {
            Value::Object(fields) => fields,
            other => panic!("a lookup response is an object, got {other}"),
        }
    }

    fn projected(lookup: &Lookup) -> serde_json::Map<String, Value> {
        projected_row(account_row(), lookup)
    }

    #[test]
    fn an_address_is_shown_only_when_the_person_or_the_hub_shares_it() {
        let hidden = Lookup::default();
        assert_eq!(projected(&hidden)["email"], Value::Null);

        let with_sharing = |share_email: bool| user::Model {
            share_email: Some(share_email),
            ..account_row()
        };
        let shared = projected_row(with_sharing(true), &hidden);
        assert_eq!(shared["email"], "person@example.com");
        // The choice itself is not part of the profile other people read.
        assert!(!shared.contains_key("share_email"));
        assert_eq!(
            projected_row(with_sharing(false), &hidden)["email"],
            Value::Null
        );

        let hub_wide = Lookup {
            email: true,
            ..Lookup::default()
        };
        assert_eq!(
            projected_row(with_sharing(false), &hub_wide)["email"],
            "person@example.com"
        );
    }

    #[test]
    fn an_address_kept_as_name_handle_or_login_name_is_never_shown() {
        let address = "felix.schultz@example.com";
        // What a sign-in that maps the address into the handle left behind.
        let row = user::Model {
            email: Some(address.into()),
            preferred_username: Some(address.into()),
            name: Some(address.into()),
            username: Some("google_110293847561029384756".into()),
            ..account_row()
        };
        let wire = projected_row(row.clone(), &Lookup::default());
        assert_eq!(wire["name"], "Felix Schultz");
        assert_eq!(wire["preferred_username"], Value::Null);
        assert_eq!(wire["email"], Value::Null);
        assert!(!Value::Object(wire).to_string().contains("example.com"));

        let login_is_address = user::Model {
            username: Some(address.into()),
            ..row
        };
        let login_names_shown = Lookup {
            username: true,
            ..Lookup::default()
        };
        let wire = projected_row(login_is_address, &login_names_shown);
        assert_eq!(wire["username"], Value::Null);
        assert!(!Value::Object(wire).to_string().contains("example.com"));
        assert_eq!(projected(&login_names_shown)["username"], "person");
    }

    #[test]
    fn the_directory_projection_carries_no_account_internals() {
        let profile_fields = [
            "id",
            "email",
            "username",
            "preferred_username",
            "name",
            "avatar_url",
            "additional_information",
            "availability",
            "description",
            "created_at",
        ];
        let everything = Lookup {
            email: true,
            name: true,
            username: true,
            preferred_username: true,
            avatar: true,
            additional_information: true,
            description: true,
            created_at: true,
        };

        let hidden = projected(&Lookup::default());
        let mut keys = hidden.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        let mut expected = profile_fields.to_vec();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(hidden["email"], Value::Null);
        assert_eq!(hidden["username"], Value::Null);
        assert_eq!(hidden["name"], "Person");
        assert_eq!(hidden["avatar_url"], "https://signed.example/avatar");

        let shared = projected(&everything);
        assert_eq!(shared.len(), profile_fields.len());
        assert_eq!(shared["email"], "person@example.com");
        assert_eq!(shared["username"], "person");

        let no_avatar = projected(&Lookup {
            avatar: false,
            ..everything
        });
        assert_eq!(no_avatar["avatar_url"], Value::Null);
    }

    #[test]
    fn availability_is_public_but_calendar_handles_and_preferences_stay_private() {
        let mut row = account_row();
        let until = chrono::Utc::now().timestamp_millis() + 60_000;
        row.additional_information = Some(
            serde_json::json!({"team":"ops","_flow_like_availability":{
                "status_override":{"status":"focus","source":"manual","until":until},
                "calendar_links":[{"id":"link","provider_id":"google","label":"Work","connection_handle":"fl_oauth_private"}],
                "preferences":{"allow_mentions":false,"allow_calls":true}
            }}),
        );
        let response = UserLookupResponse::project(
            row,
            &Lookup {
                additional_information: true,
                ..Lookup::default()
            },
            None,
        );
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(
            value["availability"],
            serde_json::json!({"status":"focus","source":"manual","until":until})
        );
        assert_eq!(
            value["additional_information"],
            serde_json::json!({"team":"ops"})
        );
        assert!(!value.to_string().contains("fl_oauth_private"));
        assert!(!value.to_string().contains("allow_mentions"));
    }

    fn bot_row() -> bot::Model {
        let created = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .fixed_offset();
        bot::Model {
            id: "bot_1".into(),
            owner_id: "user-1".into(),
            system_key: Some("flux".into()),
            preset_id: None,
            name: Some("Scout".into()),
            description: Some("Builds flows".into()),
            instructions: Some("private instructions".into()),
            appearance: serde_json::json!({ "kind": "flux" }),
            model_policy: serde_json::json!({}),
            classification: None,
            pii: false,
            capabilities: Default::default(),
            profile_id: None,
            location: "SERVER".into(),
            device_id: None,
            created_at: created,
            updated_at: created,
            deleted_at: Some(created),
            import_key: None,
            embedding_model: None,
            stt_model: None,
            tts_model: None,
            image_model: None,
            vision_model: None,
            subtask_model: None,
        }
    }

    #[test]
    fn a_bot_sub_looks_up_as_its_bot_without_personal_fields() {
        let row = bot::Model {
            name: None,
            ..bot_row()
        };
        let wire =
            serde_json::to_value(UserLookupResponse::from_bot(row, &Lookup::default())).unwrap();

        assert_eq!(wire["id"], "bot_1");
        assert_eq!(wire["name"], Value::Null);
        assert_eq!(wire["description"], "Builds flows");
        assert!(wire["created_at"].is_string());
        for field in [
            "email",
            "username",
            "preferred_username",
            "avatar_url",
            "additional_information",
        ] {
            assert_eq!(wire[field], Value::Null, "{field}");
        }
        assert_eq!(
            wire["bot"],
            serde_json::json!({
                "owner_id": "user-1",
                "system": "flux",
                "appearance": { "kind": "flux" },
                "deleted": true,
            })
        );
        assert!(!wire.to_string().contains("private instructions"));
    }

    #[test]
    fn a_live_bot_shares_only_its_pinned_voice_model() {
        let row = bot::Model {
            deleted_at: None,
            tts_model: Some("catalog-voice-bit".into()),
            stt_model: Some("private-transcription-bit".into()),
            model_policy: serde_json::json!({"api_key": "private-provider-key"}),
            ..bot_row()
        };
        let hidden = Lookup {
            description: false,
            created_at: false,
            ..Lookup::default()
        };
        let wire = serde_json::to_value(UserLookupResponse::from_bot(row, &hidden)).unwrap();
        assert_eq!(wire["bot"]["tts_model"], "catalog-voice-bit");
        assert_eq!(wire["bot"]["deleted"], false);
        assert_eq!(wire["description"], Value::Null);
        assert_eq!(wire["created_at"], Value::Null);
        for private in [
            "private instructions",
            "private-provider-key",
            "private-transcription-bit",
            "model_policy",
        ] {
            assert!(!wire.to_string().contains(private), "{private}");
        }
    }

    #[test]
    fn unpinned_and_deleted_bots_expose_no_voice_model() {
        for row in [
            bot::Model {
                deleted_at: None,
                tts_model: None,
                ..bot_row()
            },
            bot::Model {
                tts_model: Some("removed-voice-bit".into()),
                ..bot_row()
            },
        ] {
            let wire = serde_json::to_value(UserLookupResponse::from_bot(row, &Lookup::default()))
                .unwrap();
            assert!(wire["bot"].get("tts_model").is_none());
        }
        let older: BotLookup = serde_json::from_value(serde_json::json!({
            "owner_id": "user-1", "system": null, "appearance": {}, "deleted": false,
        }))
        .unwrap();
        assert!(older.tts_model.is_none());
    }

    #[test]
    fn a_bot_lookup_hides_what_the_lookup_settings_hide_for_people() {
        let hidden = Lookup {
            description: false,
            created_at: false,
            ..Lookup::default()
        };
        let wire = serde_json::to_value(UserLookupResponse::from_bot(bot_row(), &hidden)).unwrap();

        assert_eq!(wire["description"], Value::Null);
        assert_eq!(wire["created_at"], Value::Null);
        assert_eq!(wire["name"], "Scout");
        assert_eq!(wire["bot"]["owner_id"], "user-1");
        assert_eq!(wire["bot"]["deleted"], true);

        let nothing_shared = Lookup {
            email: false,
            name: false,
            username: false,
            preferred_username: false,
            avatar: false,
            additional_information: false,
            description: false,
            created_at: false,
        };
        let wire =
            serde_json::to_value(UserLookupResponse::from_bot(bot_row(), &nothing_shared)).unwrap();
        assert_eq!(wire["name"], "Scout");
        assert_eq!(wire["bot"]["system"], "flux");
    }

    #[test]
    fn tokenizing_does_not_lose_like_escaping() {
        let term = SearchTerm::parse("100% off").unwrap();
        assert_eq!(term.token_patterns(), [r"%100\%%", "%off%"]);
    }

    #[test]
    fn ranking_precedes_the_limit_with_a_stable_tie_breaker() {
        let term = SearchTerm::parse("Felix Schultz").unwrap();
        let lookup = Lookup::default();
        let sql = ranked_candidates(
            user::Entity::find().filter(search_condition(&term, &lookup)),
            &term,
            &lookup,
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
        let lookup = Lookup::default();
        let sql = ranked_candidates(
            user::Entity::find()
                .filter(search_condition(&term, &lookup))
                .filter(invitation_candidates("target", "caller"))
                .filter(user::Column::Status.ne(UserStatus::Banned)),
            &term,
            &lookup,
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
                "preferredUsername" text, avatar text, status text NOT NULL,
                "shareEmail" boolean
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

        let lookup = Lookup::default();
        let term = SearchTerm::parse("felix").unwrap();
        let ids = ranked_candidates(
            user::Entity::find()
                .filter(search_condition(&term, &lookup))
                .filter(invitation_candidates("target", "caller"))
                .filter(user::Column::Status.ne(UserStatus::Banned)),
            &term,
            &lookup,
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
            &lookup,
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
            user::Entity::find().filter(search_condition(&literal, &lookup)),
            &literal,
            &lookup,
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
                .filter(search_condition(&term, &lookup))
                .filter(user::Column::Id.is_in(["exact", "prefix", "word"]))
                .select_only()
                .column(user::Column::Id)
                .column(user::Column::Name)
                .expr(candidate_score(&term, &lookup))
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
