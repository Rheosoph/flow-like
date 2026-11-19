//! Places the owner's phones watch for their bots' place tasks, and the crossings they report.
//! A phone reads the list on every bot refresh and reports an arrival or a departure here; the
//! hub fires the task (see `bot_host::place_tasks`).

use crate::{
    bot_host::place_tasks::{self, Crossed, Dropped},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{now_millis, wake},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use flow_like_types::bot_task::{
    PlaceCrossing, PlaceCrossingOutcome, PlaceCrossingReport, WatchedPlaceList,
};
use serde::Serialize;
use utoipa::ToSchema;

/// A place one of your bots watches, as your phone reads it.
#[derive(Debug, Serialize, ToSchema)]
pub struct WatchedPlaceInfo {
    pub task_id: String,
    pub bot_id: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Radius of the watched circle in meters.
    pub radius_m: f64,
    /// `enter`, `exit` or `both`.
    pub on: String,
    /// RFC 3339 time after which the place is no longer watched.
    pub until: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WatchedPlacesResponse {
    pub places: Vec<WatchedPlaceInfo>,
}

/// What your phone reports when you cross a watched place.
#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct PlaceCrossingRequest {
    /// `enter` or `exit`.
    pub crossing: String,
    /// RFC 3339 time your phone saw the crossing.
    pub occurred_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlaceCrossingResponse {
    /// Whether this report fired the task.
    pub fired: bool,
    /// Why it did not: `stale`, `direction`, `not_watched` or `duplicate`.
    pub reason: Option<String>,
}

#[utoipa::path(
    get,
    path = "/bots/places",
    tag = "Bots",
    responses(
        (status = 200, description = "The places your bots' location tasks watch: at most 20, soonest expiry first. Your phones watch these circles and report when you cross one.", body = WatchedPlacesResponse),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "A bot token cannot read your places")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/places", skip(state, user))]
pub async fn list_places(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<WatchedPlaceList>, ApiError> {
    let owner = user.sub()?;
    Ok(Json(
        place_tasks::watched_list_in(&state.db, &owner, Utc::now()).await?,
    ))
}

fn malformed(report: &PlaceCrossingReport) -> Result<DateTime<Utc>, ApiError> {
    if report.crossing == PlaceCrossing::Both {
        return Err(ApiError::bad_request(
            "A crossing is either 'enter' or 'exit', not 'both'",
        ));
    }
    DateTime::parse_from_rfc3339(report.occurred_at.trim())
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| {
            ApiError::bad_request(format!(
                "'occurred_at' value '{}' is not an RFC 3339 time",
                report.occurred_at
            ))
        })
}

fn outcome(dropped: Dropped) -> Json<PlaceCrossingOutcome> {
    Json(PlaceCrossingOutcome {
        fired: false,
        reason: Some(dropped.reason().to_string()),
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/tasks/{task_id}/crossing",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("task_id" = String, Path, description = "The place task's id")
    ),
    request_body = PlaceCrossingRequest,
    responses(
        (status = 200, description = "Your phone saw you cross a watched place. The bot runs the task in your chat when the crossing matches and nothing fired it yet; otherwise `fired` is false with a `reason` (`stale`, `direction`, `not_watched` or `duplicate`) and the phone just moves on.", body = PlaceCrossingResponse),
        (status = 400, description = "The crossing or its time is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such place task; the phone stops watching it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/tasks/{task_id}/crossing",
    skip(state, user, request)
)]
pub async fn report_crossing(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, task_id)): Path<(String, String)>,
    Json(request): Json<PlaceCrossingRequest>,
) -> Result<Json<PlaceCrossingOutcome>, ApiError> {
    let owner = user.sub()?;
    let crossing = PlaceCrossing::parse(&request.crossing).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Crossing '{}' is not 'enter' or 'exit'",
            request.crossing
        ))
    })?;
    let body = PlaceCrossingReport {
        crossing,
        occurred_at: request.occurred_at,
    };
    let occurred_at = malformed(&body)?;
    let (bot, task) = place_tasks::place_task_of(&state.db, &owner, &bot_id, &task_id).await?;
    let now = now_millis();
    if body.check(now.with_timezone(&Utc)).is_err() {
        return Ok(outcome(Dropped::Stale));
    }
    let crossed = state
        .transaction(|txn| {
            let (task, bot) = (task.clone(), bot.clone());
            Box::pin(async move {
                place_tasks::fire_crossing_in(txn, &task, &bot, crossing, occurred_at, now).await
            })
        })
        .await?;
    match crossed {
        Crossed::Dropped(dropped) => Ok(outcome(dropped)),
        Crossed::Sent(people, bots) => {
            wake(&state, people).await;
            crate::bot_host::dispatch(&state, &bots).await;
            Ok(Json(PlaceCrossingOutcome {
                fired: true,
                reason: None,
            }))
        }
    }
}
