//! A bot's private calendar subscription. The owner turns a feed on, changes what it shows, gets a
//! new link or turns it off; calendar apps fetch the public `GET /calendar-feeds/{token}.ics`
//! without signing in, so the token in the link is the only secret. The row keeps the token's
//! hash for lookup and a sealed copy so the owner can see the link again. Servers only: a bot
//! that lives in a local store has no public URL.

use super::{lock_bot, memory::admin_for, owned_bot, secret_cipher, webhooks::WindowLimiter};
use crate::{
    bot_calendar_feed::{FeedLayers, FeedSource, etag_of, file_slug, render},
    entity::{bot, bot_calendar_feed, bot_task},
    error::ApiError,
    middleware::jwt::{AppUser, ClientIp},
    routes::invite::api_url,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use flow_like_bot_memory::host::FactView;
use flow_like_types::bot_task::{STORED_CRON, STORED_EVENT, STORED_ONCE};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use utoipa::ToSchema;

const SECRET_NAME: &str = "calendar_feed";
const TOKEN_BYTES: usize = 32;
const TOKEN_CHARS: usize = 43;
const FEED_NONE: &str = "BOT_CALENDAR_FEED_NONE";
const FEED_NOT_FOUND: &str = "BOT_CALENDAR_FEED_NOT_FOUND";
const FETCHES_PER_MINUTE: u32 = 60;
const MISSES_PER_MINUTE: u32 = 20;
const TOKEN_FETCHES_PER_MINUTE: u32 = 30;
const TOUCH_EVERY_MINUTES: i64 = 5;
const CACHE_CONTROL: &str = "private, max-age=300";

static FETCHES: LazyLock<WindowLimiter> =
    LazyLock::new(|| WindowLimiter::new(FETCHES_PER_MINUTE, Duration::from_secs(60)));
static MISSES: LazyLock<WindowLimiter> =
    LazyLock::new(|| WindowLimiter::new(MISSES_PER_MINUTE, Duration::from_secs(60)));
static TOKEN_FETCHES: LazyLock<WindowLimiter> =
    LazyLock::new(|| WindowLimiter::new(TOKEN_FETCHES_PER_MINUTE, Duration::from_secs(60)));

/// A bot's calendar subscription as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CalendarFeed {
    /// The private `https` link calendar apps subscribe to. Anyone who has it can read the feed.
    pub url: String,
    /// The same link with the `webcal://` scheme, which opens the system calendar app.
    pub webcal_url: String,
    pub include_events: bool,
    pub include_tasks: bool,
    pub include_memories: bool,
    pub include_notes: bool,
    pub created_at: DateTime<FixedOffset>,
    /// When a calendar app last fetched the feed, to within five minutes.
    pub last_fetched_at: Option<DateTime<FixedOffset>>,
}

/// What the feed shows; what is missing stays, and a new feed starts with events and tasks.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateCalendarFeed {
    /// The bot's own calendar events (needs the calendar capability).
    pub include_events: Option<bool>,
    /// Scheduled tasks as short entries (needs the scheduling capability).
    pub include_tasks: Option<bool>,
    /// Remembered dates and short spans (needs the memory capability; server-hosted bots only).
    pub include_memories: Option<bool>,
    /// Instructions and other notes as event descriptions.
    pub include_notes: Option<bool>,
}

pub(crate) fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn new_token() -> Result<String, ApiError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal(format!(
            "No random bytes for a calendar feed token: {error}"
        ))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn seal(master: &[u8; 32], bot_id: &str, token: &str) -> Result<String, ApiError> {
    secret_cipher::encrypt(master, bot_id, SECRET_NAME, token).map_err(|error| {
        ApiError::internal(format!(
            "Encrypting the calendar feed token of bot '{bot_id}' failed: {error}"
        ))
    })
}

fn open(master: &[u8; 32], row: &bot_calendar_feed::Model) -> Result<String, ApiError> {
    secret_cipher::decrypt(master, &row.bot_id, SECRET_NAME, &row.token_encrypted).map_err(
        |error| {
            ApiError::internal(format!(
                "Decrypting the calendar feed token of bot '{}' failed: {error}",
                row.bot_id
            ))
        },
    )
}

fn feed_url(origin: &str, token: &str) -> String {
    format!(
        "{}/api/v1/calendar-feeds/{token}.ics",
        origin.trim_end_matches('/')
    )
}

fn webcal_of(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    format!("webcal://{rest}")
}

pub(crate) fn view(
    row: &bot_calendar_feed::Model,
    master: &[u8; 32],
    origin: &str,
) -> Result<CalendarFeed, ApiError> {
    let url = feed_url(origin, &open(master, row)?);
    Ok(CalendarFeed {
        webcal_url: webcal_of(&url),
        url,
        include_events: row.include_events,
        include_tasks: row.include_tasks,
        include_memories: row.include_memories,
        include_notes: row.include_notes,
        created_at: row.created_at,
        last_fetched_at: row.last_fetched_at,
    })
}

fn public_origin(state: &AppState) -> Result<String, ApiError> {
    let configured = std::env::var("PUBLIC_API_URL").ok();
    let url = api_url(
        configured.as_deref(),
        &state.platform_config.domain,
        state.platform_config.secure,
    )?;
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn none_yet(bot_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::NOT_FOUND,
        FEED_NONE,
        format!("Bot '{bot_id}' has no calendar feed yet"),
    )
}

async fn row_of<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Option<bot_calendar_feed::Model>, ApiError> {
    Ok(bot_calendar_feed::Entity::find()
        .filter(bot_calendar_feed::Column::BotId.eq(bot_id))
        .one(db)
        .await?)
}

/// The bot's feed once the bot is the caller's.
pub(crate) async fn feed_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<bot_calendar_feed::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    row_of(db, bot_id)
        .await?
        .filter(|row| row.owner_id == owner_id)
        .ok_or_else(|| none_yet(bot_id))
}

/// Creates the feed with a new token, or applies `update` to the existing one.
pub(crate) async fn upsert_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    update: &UpdateCalendarFeed,
    master: &[u8; 32],
) -> Result<bot_calendar_feed::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let now = Utc::now().fixed_offset();
    let Some(existing) = row_of(db, bot_id).await? else {
        let token = new_token()?;
        return Ok(bot_calendar_feed::ActiveModel {
            id: Set(format!("bcf_{}", uuid::Uuid::new_v4().simple())),
            bot_id: Set(bot_id.to_string()),
            owner_id: Set(owner_id.to_string()),
            token_hash: Set(hash_token(&token)),
            token_encrypted: Set(seal(master, bot_id, &token)?),
            include_events: Set(update.include_events.unwrap_or(true)),
            include_tasks: Set(update.include_tasks.unwrap_or(true)),
            include_memories: Set(update.include_memories.unwrap_or(false)),
            include_notes: Set(update.include_notes.unwrap_or(false)),
            created_at: Set(now),
            updated_at: Set(now),
            last_fetched_at: Set(None),
        }
        .insert(db)
        .await?);
    };
    let mut active: bot_calendar_feed::ActiveModel = existing.into();
    if let Some(value) = update.include_events {
        active.include_events = Set(value);
    }
    if let Some(value) = update.include_tasks {
        active.include_tasks = Set(value);
    }
    if let Some(value) = update.include_memories {
        active.include_memories = Set(value);
    }
    if let Some(value) = update.include_notes {
        active.include_notes = Set(value);
    }
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

/// Replaces the token; the old link stops working with this write.
pub(crate) async fn rotate_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    master: &[u8; 32],
) -> Result<bot_calendar_feed::Model, ApiError> {
    let row = feed_in(db, owner_id, bot_id).await?;
    let token = new_token()?;
    let mut active: bot_calendar_feed::ActiveModel = row.into();
    active.token_hash = Set(hash_token(&token));
    active.token_encrypted = Set(seal(master, bot_id, &token)?);
    active.updated_at = Set(Utc::now().fixed_offset());
    Ok(active.update(db).await?)
}

pub(crate) async fn delete_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<(), ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    bot_calendar_feed::Entity::delete_many()
        .filter(bot_calendar_feed::Column::BotId.eq(bot_id))
        .filter(bot_calendar_feed::Column::OwnerId.eq(owner_id))
        .exec(db)
        .await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/calendar/feed",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The private subscription link of the bot's calendar and what it shows.", body = CalendarFeed),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot read your calendar link"),
        (status = 404, description = "You have no bot with this id, or BOT_CALENDAR_FEED_NONE: the bot has no calendar link yet")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/calendar/feed", skip(state, user))]
pub async fn get_calendar_feed(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<CalendarFeed>, ApiError> {
    let sub = user.sub()?;
    let row = feed_in(&state.db, &sub, &bot_id).await?;
    Ok(Json(view(
        &row,
        &state.encryption_key,
        &public_origin(&state)?,
    )?))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/calendar/feed",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = UpdateCalendarFeed,
    responses(
        (status = 200, description = "The subscription link of the bot's calendar. The first call creates the link; later calls change what it shows and keep the link.", body = CalendarFeed),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot change your calendar link"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/calendar/feed", skip(state, user, body))]
pub async fn put_calendar_feed(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<UpdateCalendarFeed>,
) -> Result<Json<CalendarFeed>, ApiError> {
    let sub = user.sub()?;
    let origin = public_origin(&state)?;
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, body, master) = (
                sub.clone(),
                bot_id.clone(),
                body.clone(),
                state.encryption_key,
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                upsert_in(txn, &sub, &bot_id, &body, &master).await
            })
        })
        .await?;
    Ok(Json(view(&row, &state.encryption_key, &origin)?))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/calendar/feed/rotate",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "A new subscription link. The old link stops working at once, so calendars subscribed to it stop updating.", body = CalendarFeed),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot change your calendar link"),
        (status = 404, description = "You have no bot with this id, or BOT_CALENDAR_FEED_NONE: the bot has no calendar link yet")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/calendar/feed/rotate", skip(state, user))]
pub async fn rotate_calendar_feed(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<CalendarFeed>, ApiError> {
    let sub = user.sub()?;
    let origin = public_origin(&state)?;
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, master) = (sub.clone(), bot_id.clone(), state.encryption_key);
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                rotate_in(txn, &sub, &bot_id, &master).await
            })
        })
        .await?;
    Ok(Json(view(&row, &state.encryption_key, &origin)?))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/calendar/feed",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 204, description = "The calendar link is gone; calendars subscribed to it stop updating"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot remove your calendar link"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/calendar/feed", skip(state, user))]
pub async fn delete_calendar_feed(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    delete_in(&state.db, &sub, &bot_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn not_found() -> ApiError {
    ApiError::coded(
        StatusCode::NOT_FOUND,
        FEED_NOT_FOUND,
        "This calendar link doesn't exist",
    )
}

/// The token a request path carries: `<43 URL-safe characters>` with an optional `.ics`.
pub(crate) fn token_of(segment: &str) -> Option<&str> {
    let token = segment.strip_suffix(".ics").unwrap_or(segment);
    (token.len() == TOKEN_CHARS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(token)
}

/// The feed and its live bot for a token; every miss is `None`.
pub(crate) async fn lookup_in<C: ConnectionTrait>(
    db: &C,
    token: &str,
) -> Result<Option<(bot_calendar_feed::Model, bot::Model)>, ApiError> {
    let Some(feed) = bot_calendar_feed::Entity::find()
        .filter(bot_calendar_feed::Column::TokenHash.eq(hash_token(token)))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let bot = bot::Entity::find_by_id(&feed.bot_id)
        .filter(bot::Column::OwnerId.eq(&feed.owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?;
    Ok(bot.map(|bot| (feed, bot)))
}

async fn schedule_rows<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<bot_task::Model>, ApiError> {
    Ok(bot_task::Entity::find()
        .filter(bot_task::Column::BotId.eq(bot_id))
        .filter(bot_task::Column::Kind.is_in([STORED_EVENT, STORED_ONCE, STORED_CRON]))
        .order_by_asc(bot_task::Column::CreatedAt)
        .order_by_asc(bot_task::Column::Id)
        .all(db)
        .await?)
}

async fn facts_of(state: &AppState, bot: &bot::Model) -> Vec<FactView> {
    let admin = match admin_for(state, &bot.owner_id, &bot.id).await {
        Ok(Some(admin)) => admin,
        Ok(None) => return Vec::new(),
        Err(error) => {
            tracing::warn!(bot_id = %bot.id, %error, "Calendar feed skipped the bot's memories: the memory could not be opened");
            return Vec::new();
        }
    };
    admin.list_views(false).await.unwrap_or_else(|error| {
        tracing::warn!(bot_id = %bot.id, %error, "Calendar feed skipped the bot's memories: listing facts failed");
        Vec::new()
    })
}

pub(crate) fn layers_of(feed: &bot_calendar_feed::Model, bot: &bot::Model) -> FeedLayers {
    FeedLayers {
        events: feed.include_events,
        tasks: feed.include_tasks,
        memories: feed.include_memories,
        notes: feed.include_notes,
    }
    .within(&bot.capabilities)
}

pub(crate) fn name_of(bot: &bot::Model) -> &str {
    bot.name.as_deref().unwrap_or("Bot")
}

/// The feed's HTTP answer for `body`: 304 when the caller already holds it.
pub(crate) fn feed_response(body: String, slug: &str, if_none_match: Option<&str>) -> Response {
    let etag = etag_of(&body);
    let held = if_none_match.is_some_and(|presented| {
        presented
            .split(',')
            .map(|candidate| candidate.trim().trim_start_matches("W/"))
            .any(|candidate| candidate == etag || candidate == "*")
    });
    let status = if held {
        StatusCode::NOT_MODIFIED
    } else {
        StatusCode::OK
    };
    let mut response = if held {
        status.into_response()
    } else {
        (status, body).into_response()
    };
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    if !held {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/calendar; charset=utf-8"),
        );
        if let Ok(value) = HeaderValue::from_str(&format!("inline; filename=\"{slug}.ics\"")) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
    }
    response
}

/// Records a fetch, at most once every five minutes per feed, in one conditional write.
pub(crate) async fn touch_in(
    db: &DatabaseConnection,
    feed_id: &str,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    let stale = now - ChronoDuration::minutes(TOUCH_EVERY_MINUTES);
    bot_calendar_feed::Entity::update_many()
        .col_expr(
            bot_calendar_feed::Column::LastFetchedAt,
            Expr::value(now.fixed_offset()),
        )
        .filter(bot_calendar_feed::Column::Id.eq(feed_id))
        .filter(
            Condition::any()
                .add(bot_calendar_feed::Column::LastFetchedAt.is_null())
                .add(bot_calendar_feed::Column::LastFetchedAt.lt(stale.fixed_offset())),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub fn public_routes() -> Router<AppState> {
    Router::new().route("/{token}", get(serve_calendar_feed))
}

/// The bot's calendar as an iCalendar file. The link is the credential: no sign-in is needed, so
/// calendar apps and services can fetch it, and every miss answers the same 404.
#[utoipa::path(
    get,
    path = "/calendar-feeds/{token}",
    tag = "Bots",
    params(("token" = String, Path, description = "The private token of the link, with an optional `.ics` ending")),
    responses(
        (status = 200, description = "The bot's calendar as an iCalendar file: its events, tasks and, if switched on, dated memories from 30 days ago to 180 days ahead.", content_type = "text/calendar", body = String),
        (status = 304, description = "The file has not changed since the `If-None-Match` ETag"),
        (status = 404, description = "BOT_CALENDAR_FEED_NOT_FOUND: no such link, or it was replaced or turned off"),
        (status = 429, description = "Too many requests; wait a minute")
    )
)]
#[tracing::instrument(name = "GET /calendar-feeds/{token}", skip_all)]
pub async fn serve_calendar_feed(
    State(state): State<AppState>,
    client: Option<Extension<ClientIp>>,
    Path(segment): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let now = Instant::now();
    let caller = client
        .and_then(|Extension(ClientIp(ip))| ip)
        .unwrap_or_else(|| "unknown".to_string());
    if MISSES.exhausted(&caller, now) || !FETCHES.allow(&caller, now) {
        return Err(ApiError::too_many_requests(
            "Too many calendar requests; wait a minute",
        ));
    }
    let miss = || {
        MISSES.allow(&caller, now);
        not_found()
    };
    let Some(token) = token_of(&segment) else {
        return Err(miss());
    };
    let Some((feed, bot)) = lookup_in(&state.db, token).await? else {
        return Err(miss());
    };
    if !TOKEN_FETCHES.allow(&feed.id, now) {
        return Err(ApiError::too_many_requests(
            "This calendar link was fetched too often; wait a minute",
        ));
    }
    let layers = layers_of(&feed, &bot);
    let tasks = if layers.events || layers.tasks {
        schedule_rows(&state.db, &bot.id).await?
    } else {
        Vec::new()
    };
    let facts = if layers.memories {
        facts_of(&state, &bot).await
    } else {
        Vec::new()
    };
    let body = render(&FeedSource {
        bot_name: name_of(&bot),
        layers,
        tasks: &tasks,
        facts: &facts,
        now: Utc::now(),
    });
    if let Err(error) = touch_in(&state.db, &feed.id, Utc::now()).await {
        tracing::warn!(feed_id = %feed.id, %error, "Recording a calendar feed fetch failed");
    }
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    Ok(feed_response(
        body,
        &file_slug(name_of(&bot)),
        if_none_match,
    ))
}
