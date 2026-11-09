use super::{load, update};
use crate::{error::ApiError, middleware::jwt::AppUser, routes::oauth, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{
    reqwest,
    user_availability::{
        BusyInterval, CALENDAR_CACHE_MS, CalendarLink, FreeBusy, StoredCalendarLink,
        UserAvailability,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

#[path = "calendar_data.rs"]
mod calendar_data;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkCalendar {
    pub provider_id: String,
    pub connection_handle: String,
    pub label: Option<String>,
}

fn supported_scope(provider: &str, scope: &str) -> bool {
    scope.split_whitespace().any(|scope| match provider {
        "google" => matches!(
            scope,
            "https://www.googleapis.com/auth/calendar.freebusy"
                | "https://www.googleapis.com/auth/calendar.events.freebusy"
                | "https://www.googleapis.com/auth/calendar.readonly"
                | "https://www.googleapis.com/auth/calendar"
        ),
        "microsoft" => matches!(
            scope
                .strip_prefix("https://graph.microsoft.com/")
                .unwrap_or(scope)
                .to_ascii_lowercase()
                .as_str(),
            "calendars.readbasic" | "calendars.read" | "calendars.readwrite"
        ),
        _ => false,
    })
}

async fn token(
    state: &AppState,
    subject: &str,
    link: &StoredCalendarLink,
) -> Result<String, ApiError> {
    if !matches!(link.display.provider_id.as_str(), "google" | "microsoft")
        || !link.connection_handle.starts_with("fl_oauth_")
    {
        return Err(ApiError::bad_request(
            "Choose a Google or Microsoft account connected through this hub",
        ));
    }
    let value = oauth::resolve_sink_oauth_token(
        state,
        &link.display.provider_id,
        &json!({"refresh_token":link.connection_handle}),
        Some(subject),
    )
    .await?;
    if !supported_scope(
        &link.display.provider_id,
        value["scope"].as_str().unwrap_or_default(),
    ) {
        return Err(ApiError::bad_request(
            "Reconnect this account with permission to read calendar availability",
        ));
    }
    value["access_token"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::bad_request("Reconnect your calendar account"))
}

async fn read_response(mut response: reqwest::Response) -> Result<Value, ApiError> {
    if !response.status().is_success() {
        return Err(ApiError::bad_request(
            "Calendar access failed. Reconnect your calendar",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad_request("Calendar could not be read"))?
    {
        if bytes.len() + chunk.len() > 512 * 1024 {
            return Err(ApiError::bad_request("Calendar response is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::bad_request("Calendar returned an invalid response"))
}

async fn fetch(
    state: &AppState,
    subject: &str,
    link: &StoredCalendarLink,
    start: i64,
    end: i64,
) -> Result<FreeBusy, ApiError> {
    let access_token = token(state, subject, link).await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| ApiError::bad_request("Calendar connection failed"))?;
    let start_text = chrono::DateTime::from_timestamp_millis(start)
        .unwrap()
        .to_rfc3339();
    let end_text = chrono::DateTime::from_timestamp_millis(end)
        .unwrap()
        .to_rfc3339();
    let response = match link.display.provider_id.as_str() {
        "google" => client.post("https://www.googleapis.com/calendar/v3/freeBusy").bearer_auth(access_token)
            .json(&json!({"timeMin":start_text,"timeMax":end_text,"timeZone":"UTC","items":[{"id":"primary"}]})).send().await,
        "microsoft" => client.get("https://graph.microsoft.com/v1.0/me/calendarView").bearer_auth(access_token)
            .header("Prefer", "outlook.timezone=\"UTC\"")
            .query(&[("startDateTime",start_text.as_str()),("endDateTime",end_text.as_str()),("$select","start,end,showAs,isCancelled"),("$top","1000")]).send().await,
        _ => return Err(ApiError::bad_request("Unsupported calendar provider")),
    }.map_err(|_| ApiError::bad_request("Calendar connection failed"))?;
    let value = read_response(response).await?;
    match link.display.provider_id.as_str() {
        "google" => calendar_data::google(&value, start, end),
        _ => calendar_data::microsoft(&value, start, end),
    }
    .map_err(ApiError::bad_request)
}

#[utoipa::path(
    post,
    path = "/user/availability/calendars",
    tag = "user",
    description = "Link a Google or Microsoft calendar connected through this hub so your status can follow your meetings. You can link up to four calendars.",
    request_body = LinkCalendar,
    responses(
        (status = 200, description = "Your availability with the calendar linked", body = Object),
        (status = 400, description = "The calendar could not be read or four calendars are already linked"),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /user/availability/calendars", skip_all)]
pub async fn link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<LinkCalendar>,
) -> Result<Json<UserAvailability>, ApiError> {
    let subject = user.sub()?;
    let label = body.label.unwrap_or_else(|| {
        if body.provider_id == "google" {
            "Google Calendar"
        } else {
            "Microsoft Calendar"
        }
        .into()
    });
    if label.trim().is_empty() || label.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "Choose a calendar label of at most 80 characters",
        ));
    }
    let linked = StoredCalendarLink {
        display: CalendarLink {
            id: flow_like_types::create_id(),
            provider_id: body.provider_id,
            label: label.trim().into(),
        },
        connection_handle: body.connection_handle,
    };
    let now = chrono::Utc::now().timestamp_millis();
    let free_busy = flow_like_types::tokio::time::timeout(
        std::time::Duration::from_secs(20),
        fetch(&state, &subject, &linked, now, now + 86_400_000),
    )
    .await
    .map_err(|_| ApiError::bad_request("Calendar connection timed out"))??;
    crate::routes::user::ensure_user_exists(&state, &subject).await?;
    let updated = update(&state.db, &subject, |stored| {
        if stored.calendar_links.iter().any(|previous| {
            previous.connection_handle == linked.connection_handle
                && previous.display.provider_id == linked.display.provider_id
        }) {
            return Ok(());
        }
        if stored.calendar_links.len() >= 4 {
            return Err(ApiError::bad_request("You can link up to four calendars"));
        }
        stored.calendar_links.push(linked.clone());
        // A single result is complete only when this is the first connected calendar.
        stored.free_busy = (stored.calendar_links.len() == 1).then(|| free_busy.clone());
        stored.calendar_valid_until = if stored.free_busy.is_some() {
            now + CALENDAR_CACHE_MS
        } else {
            0
        };
        stored.calendar_checked_at = now;
        stored.calendar_error = None;
        Ok(())
    })
    .await?;
    state.channels.publish_realtime_invalidation(&subject).await;
    if updated.calendar_links.len() > 1 {
        return refresh_for(&state, &subject).await.map(Json);
    }
    Ok(Json(updated.view(now)))
}

#[utoipa::path(
    delete,
    path = "/user/availability/calendars/{id}",
    tag = "user",
    description = "Unlink a calendar from your availability.",
    params(("id" = String, Path, description = "The linked calendar's id")),
    responses(
        (status = 200, description = "Your availability without the calendar", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 409, description = "Your availability changed at the same time. Try again")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /user/availability/calendars/{id}", skip_all)]
pub async fn unlink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<UserAvailability>, ApiError> {
    let subject = user.sub()?;
    let updated = update(&state.db, &subject, |stored| {
        stored.calendar_links.retain(|link| link.display.id != id);
        stored.free_busy = None;
        stored.calendar_valid_until = 0;
        stored.calendar_checked_at = 0;
        stored.calendar_error = None;
        Ok(())
    })
    .await?;
    state.channels.publish_realtime_invalidation(&subject).await;
    Ok(Json(updated.view(chrono::Utc::now().timestamp_millis())))
}

#[utoipa::path(
    post,
    path = "/user/availability/calendars/refresh",
    tag = "user",
    description = "Refresh your availability from your linked calendars. A refresh within a minute of the last check returns the cached result instead of asking the calendar providers again.",
    responses(
        (status = 200, description = "Your availability, refreshed or cached", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 409, description = "Your linked calendars changed during the refresh")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /user/availability/calendars/refresh", skip_all)]
pub async fn refresh(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<UserAvailability>, ApiError> {
    let subject = user.sub()?;
    let stored = load(&state.db, &subject).await?;
    let now = chrono::Utc::now().timestamp_millis();
    if super::refresh_throttled(&stored, now) {
        return Ok(Json(stored.view(now)));
    }
    refresh_for(&state, &subject).await.map(Json)
}

pub(super) async fn refresh_for(
    state: &AppState,
    subject: &str,
) -> Result<UserAvailability, ApiError> {
    let initial = load(&state.db, subject).await?;
    let now = chrono::Utc::now().timestamp_millis();
    if initial.calendar_links.is_empty() {
        return Ok(initial.view(now));
    }
    let end = now + 86_400_000;
    let mut intervals: Vec<BusyInterval> = Vec::new();
    let mut failed = false;
    let fetched = futures::future::join_all(initial.calendar_links.iter().map(|link| {
        flow_like_types::tokio::time::timeout(
            std::time::Duration::from_secs(20),
            fetch(state, subject, link, now, end),
        )
    }))
    .await;
    for result in fetched {
        match result {
            Ok(Ok(value)) => intervals.extend(value.busy),
            _ => {
                failed = true;
            }
        }
    }
    let free_busy = FreeBusy::normalized(now, end, intervals);
    failed |= free_busy.busy.len() > 128;
    let updated = update(&state.db, subject, |stored| {
        if stored.calendar_links != initial.calendar_links {
            return Err(ApiError::conflict(
                "Your linked calendars changed. Refresh again",
            ));
        }
        stored.free_busy = (!failed).then(|| free_busy.clone());
        stored.calendar_valid_until = if failed { 0 } else { now + CALENDAR_CACHE_MS };
        stored.calendar_checked_at = now;
        stored.calendar_error = failed.then(|| {
            "Calendar availability could not be refreshed. Reconnect your calendar or try again"
                .into()
        });
        Ok(())
    })
    .await?;
    state.channels.publish_realtime_invalidation(subject).await;
    Ok(updated.view(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_scopes_are_provider_specific() {
        assert!(supported_scope(
            "google",
            "https://www.googleapis.com/auth/calendar.freebusy"
        ));
        assert!(supported_scope(
            "microsoft",
            "openid Calendars.ReadBasic offline_access"
        ));
        assert!(!supported_scope("google", "Calendars.ReadBasic"));
        assert!(!supported_scope("microsoft", "User.Read"));
    }
}
