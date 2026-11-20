//! The bot calendar on this desktop: the commands behind the calendar view and the wake-up
//! settings of local bots, and the periodic sync that plans wake-ups for the connected-calendar
//! events of the account bots that live here (the hub syncs those of server and device bots).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use flow_like_bot_tools::calendar::{SyncReport, list_range, sync_wakes};
use flow_like_types::{
    Result, anyhow,
    bot_calendar::{
        CALENDAR_SYNC_EVERY_SECS, CalendarRange, CalendarWake, UpdateCalendarWake, check_range,
    },
    bot_task::TaskKind,
};
use reqwest::Method;
use serde_json::Value;
use tauri::{AppHandle, State};

use super::hub::{ProfileHub, TurnHub};
use crate::functions::ai::bot_tools::{
    connectors::{DesktopConnectors, SecretReader},
    held_locally,
    schedule::{AccountScheduler, LocalScheduler, TaskApi},
};
use crate::functions::ai::native_platform_tools::{NativePlatformSession, segment};
use crate::functions::local_bots::{LocalBots, on_store};

const FIRST_SYNC_DELAY: Duration = Duration::from_secs(45);
const CALENDAR_CAPABILITY: &str = "calendar";

/// The signed-in session's calls to the hub, outside any turn.
struct SessionApi(NativePlatformSession);

#[async_trait]
impl TaskApi for SessionApi {
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> std::result::Result<Value, String> {
        self.0.request(method, path, body).await
    }
}

/// Outside a turn the bot has no secrets; calendar sign-ins are OAuth accounts of the device.
struct NoSecrets;

#[async_trait]
impl SecretReader for NoSecrets {
    async fn secret(&self, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

/// An account bot's calendar surfaces on this desktop: its connectors and its hub-side tasks.
struct AccountCalendar {
    connectors: DesktopConnectors,
    scheduler: AccountScheduler,
}

async fn account_workspace(api: &dyn TaskApi, bot_id: &str) -> String {
    let path = format!("bots/{}/workspace", segment(bot_id));
    match api.request(Method::POST, &path, None).await {
        Ok(workspace) => workspace
            .get("app_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Err(error) => {
            tracing::warn!(%error, bot_id, "Bot calendar: the bot's workspace is unavailable");
            String::new()
        }
    }
}

async fn account_calendar(
    app: &AppHandle,
    session: &Arc<SessionApi>,
    bot_id: &str,
) -> AccountCalendar {
    let workspace = account_workspace(session.as_ref(), bot_id).await;
    let connectors = DesktopConnectors::new(
        app,
        session.0.auth.clone(),
        &workspace,
        bot_id,
        session.clone(),
        Arc::new(NoSecrets),
    )
    .await;
    AccountCalendar {
        connectors,
        scheduler: AccountScheduler {
            api: session.clone(),
            bot_id: bot_id.to_string(),
            turn_id: None,
            conversation_id: None,
        },
    }
}

async fn desktop_hosts(app: &AppHandle, bot_id: &str) -> bool {
    ProfileHub::new(app.clone())
        .desktop_bots()
        .await
        .is_ok_and(|bots| bots.iter().any(|id| id == bot_id))
}

async fn session_api(app: &AppHandle) -> std::result::Result<Arc<SessionApi>, String> {
    NativePlatformSession::new(app, None)
        .await
        .map(|session| Arc::new(SessionApi(session)))
}

/// The bot's calendar between `from` and `to` (RFC 3339): a local bot's from the store, a
/// desktop-hosted account bot's with the sign-ins of this device, any other account bot's from
/// the hub.
#[tauri::command]
pub async fn bot_calendar_events(
    app: AppHandle,
    bot_id: String,
    from: String,
    to: String,
) -> std::result::Result<CalendarRange, String> {
    let (start, end) = check_range(&from, &to)?;
    let now = Utc::now();
    if held_locally(&app, &bot_id).await {
        let scheduler = LocalScheduler::unbound(&app, &bot_id);
        return list_range(None, &scheduler, start, end, now)
            .await
            .map_err(|error| error.to_string());
    }
    let session = session_api(&app).await?;
    if desktop_hosts(&app, &bot_id).await {
        let calendar = account_calendar(&app, &session, &bot_id).await;
        return list_range(
            Some(&calendar.connectors),
            &calendar.scheduler,
            start,
            end,
            now,
        )
        .await
        .map_err(|error| error.to_string());
    }
    let path = format!(
        "bots/{}/calendar/events?from={}&to={}",
        segment(&bot_id),
        urlencoding::encode(&from),
        urlencoding::encode(&to)
    );
    let range = session.request(Method::GET, &path, None).await?;
    serde_json::from_value(range)
        .map_err(|error| format!("The hub sent an unusable calendar: {error}"))
}

/// The wake-up settings of a local bot.
#[tauri::command]
pub async fn local_bot_calendar_wake(
    bots: State<'_, LocalBots>,
    bot_id: String,
) -> std::result::Result<CalendarWake, String> {
    on_store(&bots, move |store| store.calendar_wake(&bot_id))
        .await
        .map_err(|error| error.to_string())
}

/// Changes the wake-up settings of a local bot and re-plans the wake-ups of its events.
#[tauri::command]
pub async fn local_bot_set_calendar_wake(
    bots: State<'_, LocalBots>,
    bot_id: String,
    update: UpdateCalendarWake,
) -> std::result::Result<CalendarWake, String> {
    on_store(&bots, move |store| store.set_calendar_wake(&bot_id, update))
        .await
        .map_err(|error| error.to_string())
}

async fn holds_calendar(api: &dyn TaskApi, bot_id: &str) -> Result<bool> {
    let bot = api
        .request(Method::GET, &format!("bots/{}", segment(bot_id)), None)
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(bot
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(CALENDAR_CAPABILITY))))
}

async fn plans_external_wakes(calendar: &AccountCalendar) -> Result<bool> {
    use flow_like_bot_tools::Scheduler;
    if calendar.scheduler.calendar_wake().await?.syncs_external() {
        return Ok(true);
    }
    Ok(calendar.scheduler.list().await?.iter().any(|task| {
        task.kind == TaskKind::Event
            && task
                .event
                .as_ref()
                .is_some_and(|event| event.external.is_some())
    }))
}

async fn sync_bot(
    app: &AppHandle,
    session: &Arc<SessionApi>,
    bot_id: &str,
) -> Result<Option<SyncReport>> {
    if !holds_calendar(session.as_ref(), bot_id).await? {
        return Ok(None);
    }
    let calendar = account_calendar(app, session, bot_id).await;
    if !plans_external_wakes(&calendar).await? {
        return Ok(None);
    }
    sync_wakes(&calendar.connectors, &calendar.scheduler, Utc::now())
        .await
        .map(Some)
}

async fn sync_all(app: &AppHandle, hub: &ProfileHub) {
    let Ok(bots) = hub.desktop_bots().await else {
        return;
    };
    if bots.is_empty() {
        return;
    }
    let session = match session_api(app).await {
        Ok(session) if session.0.auth.is_some() => session,
        _ => return,
    };
    for bot_id in bots {
        match sync_bot(app, &session, &bot_id).await {
            Ok(Some(report)) => {
                tracing::info!(bot_id, ?report, "Bot calendar: synced wake-ups");
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, bot_id, "Bot calendar: the sync failed");
            }
        }
    }
}

/// Plans the wake-ups of the connected calendars of the account bots on this desktop, shortly
/// after start and then every [`CALENDAR_SYNC_EVERY_SECS`]. A failure never stops the loop.
pub(super) async fn run(app: AppHandle, hub: Arc<ProfileHub>) {
    tokio::time::sleep(FIRST_SYNC_DELAY).await;
    let mut tick = tokio::time::interval(Duration::from_secs(CALENDAR_SYNC_EVERY_SECS as u64));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        sync_all(&app, hub.as_ref()).await;
    }
}
