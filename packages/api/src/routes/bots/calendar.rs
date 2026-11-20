//! A bot's calendar: its wake-up settings and a listing of its events (its own and those of the
//! owner's connected calendars). Events themselves are event tasks (see `tasks`); changing the
//! settings works every event task out again (see `bot_host::calendar_tasks`).

use super::{
    lock_bot, owned_bot,
    tasks::{CALENDAR_OWNER_CHAT_ONLY, TaskScope, scope_of_turn, turn_of_call},
};
use crate::{
    bot_host::{calendar_tasks, task_schedules},
    bot_tools::{connectors::ApiConnectors, schedule::ApiScheduler},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use flow_like_types::bot_calendar::{CalendarRange, CalendarWake, UpdateCalendarWake, check_range};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which calendar events wake a bot and how long before they start.
#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarWakeInfo {
    /// Wakes the bot before every event that passes the filters. Off by default.
    pub enabled: bool,
    /// Minutes before the start, largest first; one to four, each up to a week.
    pub offsets_minutes: Vec<u32>,
    /// Wakes for events of the bot's own calendar.
    pub internal: bool,
    /// Wakes for events of your connected calendars.
    pub external: bool,
    /// When not empty, wakes only for events whose title or location contains one of these.
    pub only_matching: Vec<String>,
    /// Never wakes for events whose title or location contains one of these.
    pub skip_matching: Vec<String>,
    /// Skips whole-day events.
    pub skip_all_day: bool,
    /// A wake-up that finds nothing to do ends without a message.
    pub quiet: bool,
}

/// Changes to a bot's calendar wake-up settings; what is missing stays.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UpdateCalendarWakeRequest {
    pub enabled: Option<bool>,
    pub offsets_minutes: Option<Vec<u32>>,
    pub internal: Option<bool>,
    pub external: Option<bool>,
    pub only_matching: Option<Vec<String>>,
    pub skip_matching: Option<Vec<String>>,
    pub skip_all_day: Option<bool>,
    pub quiet: Option<bool>,
    /// The bot's turn that asks for the change, sent by the device running that turn. Only a turn
    /// in your own chat with the bot may change its calendar. A bot's own token always uses its
    /// turn.
    #[serde(default)]
    pub turn_id: Option<String>,
}

impl UpdateCalendarWakeRequest {
    fn change(&self) -> UpdateCalendarWake {
        UpdateCalendarWake {
            enabled: self.enabled,
            offsets_minutes: self.offsets_minutes.clone(),
            internal: self.internal,
            external: self.external,
            only_matching: self.only_matching.clone(),
            skip_matching: self.skip_matching.clone(),
            skip_all_day: self.skip_all_day,
            quiet: self.quiet,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CalendarRangeQuery {
    /// RFC 3339 start of the span.
    pub from: String,
    /// RFC 3339 end of the span, after `from` and at most 92 days later.
    pub to: String,
}

/// Saves `update` to the bot's calendar settings, works out every event task of the bot again
/// and brings their schedules in line. Returns the settings now in effect.
pub(crate) async fn set_wake(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    update: UpdateCalendarWake,
) -> Result<CalendarWake, ApiError> {
    let (wake, changed) = state
        .transaction(|txn| {
            let (owner_id, bot_id, update) =
                (owner_id.to_string(), bot_id.to_string(), update.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let bot = owned_bot(txn, &owner_id, &bot_id).await?;
                calendar_tasks::apply_in(txn, &bot, update, now_millis()).await
            })
        })
        .await?;
    for row in &changed {
        task_schedules::sync(state, row).await;
    }
    Ok(wake)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/calendar",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "Which calendar events wake the bot and how long before they start; the defaults (off) until changed.", body = CalendarWakeInfo),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/calendar", skip(state, user))]
pub async fn get_calendar_wake(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<CalendarWake>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    Ok(Json(calendar_tasks::wake_of(&bot)))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/calendar",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = UpdateCalendarWakeRequest,
    responses(
        (status = 200, description = "The wake-up settings now in effect. The wake-ups of every event the bot keeps are planned again.", body = CalendarWakeInfo),
        (status = 400, description = "A setting is invalid, such as more than four offsets or an offset over a week"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_CALENDAR_OWNER_CHAT_ONLY: the bot's calendar can only be changed from your own chat with it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/calendar", skip(state, user, body))]
pub async fn put_calendar_wake(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<UpdateCalendarWakeRequest>,
) -> Result<Json<CalendarWake>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    if let Some(turn_id) = turn_of_call(&user, body.turn_id.as_deref())
        && scope_of_turn(&state.db, &sub, &bot_id, turn_id).await? != TaskScope::Everything
    {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            CALENDAR_OWNER_CHAT_ONLY,
            format!(
                "A bot's calendar can only be changed in the owner's own chat with it; turn '{turn_id}' runs elsewhere or was asked for by another bot"
            ),
        ));
    }
    Ok(Json(set_wake(&state, &sub, &bot_id, body.change()).await?))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/calendar/events",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id"), CalendarRangeQuery),
    responses(
        (status = 200, description = "The bot's calendar between `from` and `to`: occurrences of its own events and the events of your connected Google and Microsoft calendars, earliest first, with a source entry per connected calendar (carrying an `error` when it could not be read).", body = Object),
        (status = 400, description = "`from` or `to` is not an RFC 3339 time, `to` is not after `from`, or the span is over 92 days"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot list your calendar"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/calendar/events", skip(state, user))]
pub async fn list_calendar_events(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(range): Query<CalendarRangeQuery>,
) -> Result<Json<CalendarRange>, ApiError> {
    let owner = user.sub()?;
    owned_bot(&state.db, &owner, &bot_id).await?;
    let (from, to) = check_range(&range.from, &range.to).map_err(ApiError::bad_request)?;
    let connectors = ApiConnectors::new(&state, &owner, &bot_id);
    let scheduler = ApiScheduler::new(&state, &owner, &bot_id, None);
    let listed = flow_like_bot_tools::calendar::list_range(
        Some(&connectors),
        &scheduler,
        from,
        to,
        Utc::now(),
    )
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "The calendar of bot '{bot_id}' could not be listed: {error}"
        ))
    })?;
    Ok(Json(listed))
}
