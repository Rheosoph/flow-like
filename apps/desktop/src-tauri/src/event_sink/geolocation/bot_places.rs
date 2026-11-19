//! Places the owner's bots watch (location-trigger tasks). The hub lists them, this device
//! watches them as background regions next to the app's Location Events, and a crossing is
//! reported to the hub, which fires the task.

use super::{
    dispatch,
    model::{NativeRegistration, Scope, Transition},
    native,
};
use crate::event_sink::{
    manager::DbConnection,
    trigger_token::{hub_client, read_body},
};
use anyhow::{Context, Result, anyhow, ensure};
use flow_like_types::bot_task::{
    PlaceCrossing, PlaceCrossingReport, WatchedPlace, WatchedPlaceList,
};
use rusqlite::{Connection, params};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::AppHandle;

/// Phones watch bot places; a desktop has no way to follow its owner around.
pub const WATCHES: bool = cfg!(any(target_os = "ios", target_os = "android"));
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const REFRESH_RETRY: Duration = Duration::from_secs(60);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(20);

static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);
static PERMISSION_ASKED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq)]
pub struct BotPlace {
    pub registration_id: String,
    pub bot_id: String,
    pub task_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub radius: f64,
    pub on: String,
    pub until: Option<i64>,
}

impl BotPlace {
    fn of(scope: &str, place: &WatchedPlace) -> Option<Self> {
        let valid = place.latitude.is_finite()
            && (-90.0..=90.0).contains(&place.latitude)
            && place.longitude.is_finite()
            && (-180.0..=180.0).contains(&place.longitude)
            && place.radius_m.is_finite()
            && (100.0..=100_000.0).contains(&place.radius_m)
            && !place.task_id.is_empty()
            && !place.bot_id.is_empty();
        if !valid {
            return None;
        }
        let until = place
            .until
            .as_deref()
            .and_then(|until| chrono::DateTime::parse_from_rfc3339(until).ok())
            .map(|until| until.timestamp_millis());
        let on = place.on.as_str();
        let content = serde_json::json!([
            "bot-place",
            scope,
            place.bot_id,
            place.task_id,
            place.latitude,
            place.longitude,
            place.radius_m,
            on
        ]);
        Some(Self {
            registration_id: blake3::hash(content.to_string().as_bytes())
                .to_hex()
                .to_string(),
            bot_id: place.bot_id.clone(),
            task_id: place.task_id.clone(),
            latitude: place.latitude,
            longitude: place.longitude,
            radius: place.radius_m,
            on: on.into(),
            until,
        })
    }

    pub fn registration(&self) -> NativeRegistration {
        NativeRegistration {
            id: self.registration_id.clone(),
            app_id: self.bot_id.clone(),
            event_id: self.task_id.clone(),
            latitude: self.latitude,
            longitude: self.longitude,
            radius_meters: self.radius,
            transition: self.on.clone(),
            background: true,
            #[cfg(target_os = "android")]
            delivery: None,
        }
    }

    fn expired(&self, now: i64) -> bool {
        self.until.is_some_and(|until| until <= now)
    }
}

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS geofence_bot_places (
            scope TEXT NOT NULL, registration_id TEXT NOT NULL, bot_id TEXT NOT NULL,
            task_id TEXT NOT NULL, latitude REAL NOT NULL, longitude REAL NOT NULL,
            radius REAL NOT NULL, on_crossing TEXT NOT NULL, until INTEGER,
            PRIMARY KEY (scope, registration_id)
        );",
    )?;
    Ok(())
}

/// Replaces the places `scope` watches with the hub's list, in the hub's order.
pub fn replace(conn: &Connection, scope: &str, places: &[WatchedPlace]) -> Result<usize> {
    initialize(conn)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM geofence_bot_places WHERE scope=?1", [scope])?;
    let mut kept = 0;
    for place in places.iter().filter_map(|place| BotPlace::of(scope, place)) {
        kept += tx.execute(
            "INSERT OR IGNORE INTO geofence_bot_places VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                scope,
                place.registration_id,
                place.bot_id,
                place.task_id,
                place.latitude,
                place.longitude,
                place.radius,
                place.on,
                place.until
            ],
        )?;
    }
    tx.commit()?;
    Ok(kept)
}

pub fn list(conn: &Connection, scope: &str) -> Result<Vec<BotPlace>> {
    initialize(conn)?;
    let mut statement = conn.prepare(
        "SELECT registration_id,bot_id,task_id,latitude,longitude,radius,on_crossing,until
         FROM geofence_bot_places WHERE scope=?1 ORDER BY rowid",
    )?;
    let places = statement
        .query_map([scope], |row| {
            Ok(BotPlace {
                registration_id: row.get(0)?,
                bot_id: row.get(1)?,
                task_id: row.get(2)?,
                latitude: row.get(3)?,
                longitude: row.get(4)?,
                radius: row.get(5)?,
                on: row.get(6)?,
                until: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(places)
}

pub fn find(conn: &Connection, scope: &str, registration_id: &str) -> Result<Option<BotPlace>> {
    Ok(list(conn, scope)?
        .into_iter()
        .find(|place| place.registration_id == registration_id))
}

pub fn forget_task(conn: &Connection, scope: &str, task_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM geofence_bot_places WHERE scope=?1 AND task_id=?2",
        params![scope, task_id],
    )?;
    Ok(())
}

/// Drops the places of every scope but `scope`, or of all when signed out.
pub fn prune(conn: &Connection, scope: Option<&str>) -> Result<()> {
    initialize(conn)?;
    match scope {
        Some(scope) => conn.execute("DELETE FROM geofence_bot_places WHERE scope<>?1", [scope])?,
        None => conn.execute("DELETE FROM geofence_bot_places", [])?,
    };
    Ok(())
}

/// The places to watch now, soonest-ending first (open-ended last), within `slots`.
pub fn select(places: &[BotPlace], now: i64, slots: usize) -> Vec<&BotPlace> {
    let mut live: Vec<&BotPlace> = places.iter().filter(|place| !place.expired(now)).collect();
    live.sort_by_key(|place| (place.until.is_none(), place.until));
    live.truncate(slots);
    live
}

/// The registrations of every live place of `scope`, for matching crossings.
pub fn live_registrations(
    conn: &Connection,
    scope: &str,
    now: i64,
) -> Result<Vec<NativeRegistration>> {
    Ok(select(&list(conn, scope)?, now, usize::MAX)
        .into_iter()
        .map(BotPlace::registration)
        .collect())
}

/// The app's registrations first, then the bot places that fit under `limit`.
pub fn merge(
    app: &[NativeRegistration],
    conn: &Connection,
    scope: &str,
    now: i64,
    limit: usize,
) -> Result<Vec<NativeRegistration>> {
    let mut merged = app.to_vec();
    if !WATCHES {
        return Ok(merged);
    }
    let slots = limit.saturating_sub(app.len());
    merged.extend(
        select(&list(conn, scope)?, now, slots)
            .into_iter()
            .map(BotPlace::registration),
    );
    Ok(merged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Done,
    Gone,
    Retry,
}

/// What the hub's answer to a crossing report means.
pub fn verdict(status: u16) -> Verdict {
    match status {
        200..=299 | 401 | 403 => Verdict::Done,
        404 => Verdict::Gone,
        408 | 429 | 500..=599 => Verdict::Retry,
        _ => Verdict::Done,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Settle {
    Complete,
    Forget,
    Retry,
}

/// What is left to do with a crossing after its delivery `result`, given whether
/// it is still `deliverable`.
pub fn settle(result: &Result<Verdict>, deliverable: bool) -> Settle {
    match result {
        Ok(Verdict::Done) => Settle::Complete,
        Ok(Verdict::Gone) => Settle::Forget,
        _ if deliverable => Settle::Retry,
        _ => Settle::Complete,
    }
}

fn crossing_of(transition: &Transition) -> Result<PlaceCrossing> {
    match transition.transition.as_str() {
        "enter" => Ok(PlaceCrossing::Enter),
        "exit" => Ok(PlaceCrossing::Exit),
        other => Err(anyhow!("A place crossing is enter or exit, not '{other}'")),
    }
}

fn report_of(transition: &Transition) -> Result<PlaceCrossingReport> {
    let occurred_at = chrono::DateTime::from_timestamp_millis(transition.occurred_at)
        .context("The crossing time is out of range")?;
    Ok(PlaceCrossingReport {
        crossing: crossing_of(transition)?,
        occurred_at: occurred_at.to_rfc3339(),
    })
}

fn crossing_url(scope: &Scope, place: &BotPlace) -> String {
    format!(
        "{}/api/v1/bots/{}/tasks/{}/crossing",
        scope.origin,
        urlencoding::encode(&place.bot_id),
        urlencoding::encode(&place.task_id)
    )
}

async fn access_token(scope: &Scope) -> Result<String> {
    let session = crate::native_auth::SessionBinding::capture(&scope.profile_id, &scope.origin)
        .await
        .map_err(anyhow::Error::msg)?
        .context("Sign in to report a location trigger")?;
    ensure!(
        session.subject == scope.subject,
        "The signed-in account changed"
    );
    session.access_token(None).await.map_err(anyhow::Error::msg)
}

/// Reports a crossing of `place` to the hub. A network failure is an error, any answer a verdict.
pub async fn deliver(scope: &Scope, place: &BotPlace, transition: &Transition) -> Result<Verdict> {
    let report = report_of(transition)?;
    let token = access_token(scope).await?;
    let response = hub_client()?
        .post(crossing_url(scope, place))
        .bearer_auth(token)
        .json(&report)
        .send()
        .await?;
    let status = response.status().as_u16();
    let _ = read_body(response).await;
    Ok(verdict(status))
}

pub fn request_refresh() {
    REFRESH_REQUESTED.store(true, Ordering::Release);
    super::notify();
}

/// Stores the hub's list of watched places for `scope` and wakes the sink to watch them.
pub fn store(db: &DbConnection, scope: &str, body: serde_json::Value) -> Result<usize> {
    let list: WatchedPlaceList = serde_json::from_value(body).context("Unusable place list")?;
    let kept = replace(&db.lock().unwrap(), scope, &list.places)?;
    super::notify();
    Ok(kept)
}

/// Fetches the watched places from the hub as the signed-in account. A failure keeps the old rows.
pub async fn refresh(app: &AppHandle, db: &DbConnection) -> Result<()> {
    let scope = dispatch::current_scope(app).await?;
    let token = access_token(&scope).await?;
    let response = hub_client()?
        .get(format!("{}/api/v1/bots/places", scope.origin))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = read_body(response).await?;
    ensure!(
        status.is_success(),
        "The hub refused the place list ({status})"
    );
    store(db, &scope.raw, serde_json::from_slice(&body)?)?;
    Ok(())
}

/// When the sink fetches the watched places again.
#[derive(Debug, Default)]
pub struct RefreshClock {
    next: Option<Instant>,
    was_foreground: bool,
}

impl RefreshClock {
    pub fn due(&mut self, now: Instant, foreground: bool, requested: bool) -> bool {
        let came_forward = foreground && !self.was_foreground;
        self.was_foreground = foreground;
        requested || came_forward || self.next.is_none_or(|next| now >= next)
    }

    pub fn finished(&mut self, now: Instant, succeeded: bool) {
        self.next = Some(
            now + if succeeded {
                REFRESH_EVERY
            } else {
                REFRESH_RETRY
            },
        );
    }
}

/// One pass of the sink worker: refreshes the places when due and, with the app in front and
/// places watched, asks once per launch for the background location access they need.
pub async fn tick(app: &AppHandle, db: &DbConnection, clock: &mut RefreshClock) {
    if !WATCHES {
        return;
    }
    let Ok(raw) = native::scope() else { return };
    if raw.is_empty() {
        let _ = prune(&db.lock().unwrap(), None);
        return;
    }
    let foreground = native::foreground();
    let requested = REFRESH_REQUESTED.swap(false, Ordering::AcqRel);
    if clock.due(Instant::now(), foreground, requested) {
        let outcome = tokio::time::timeout(REFRESH_TIMEOUT, refresh(app, db)).await;
        let succeeded = matches!(outcome, Ok(Ok(())));
        if let Ok(Err(error)) | Err(error) = outcome.map_err(|_| anyhow!("Place refresh timed out"))
        {
            tracing::warn!(error=%error, "Could not refresh the places the bots watch");
        }
        clock.finished(Instant::now(), succeeded);
    }
    if foreground && !PERMISSION_ASKED.load(Ordering::Acquire) {
        ask_background_access(&raw, db).await;
    }
}

async fn ask_background_access(raw: &str, db: &DbConnection) {
    let watching = list(&db.lock().unwrap(), raw)
        .map(|places| select(&places, super::now_ms(), 1).len())
        .unwrap_or(0);
    if watching == 0 {
        return;
    }
    let Ok(status) = crate::functions::native::native_geofence_permission("status".into()).await
    else {
        return;
    };
    if status["authorization"] == "always" {
        PERMISSION_ASKED.store(true, Ordering::Release);
        return;
    }
    if crate::functions::native::native_geofence_permission("background".into())
        .await
        .is_ok()
    {
        PERMISSION_ASKED.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "scope";
    const NOW: i64 = 1_800_000_000_000;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        conn
    }

    fn watched(task: &str, until: Option<&str>) -> WatchedPlace {
        WatchedPlace {
            task_id: task.into(),
            bot_id: "bot".into(),
            latitude: 52.5,
            longitude: 13.4,
            radius_m: 150.0,
            on: PlaceCrossing::Enter,
            until: until.map(str::to_owned),
        }
    }

    fn place(task: &str, until: Option<i64>) -> BotPlace {
        BotPlace {
            until,
            ..BotPlace::of(SCOPE, &watched(task, None)).unwrap()
        }
    }

    fn app_registration(id: &str) -> NativeRegistration {
        NativeRegistration {
            id: id.into(),
            app_id: "app".into(),
            event_id: id.into(),
            latitude: 1.0,
            longitude: 2.0,
            radius_meters: 200.0,
            transition: "enter".into(),
            background: true,
            #[cfg(target_os = "android")]
            delivery: None,
        }
    }

    #[test]
    fn registration_ids_are_stable_and_bound_to_the_place() {
        let first = BotPlace::of(SCOPE, &watched("task", None)).unwrap();
        let expected = blake3::hash(
            serde_json::json!([
                "bot-place",
                SCOPE,
                "bot",
                "task",
                52.5,
                13.4,
                150.0,
                "enter"
            ])
            .to_string()
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        assert_eq!(first.registration_id, expected);
        assert_eq!(
            first.registration_id,
            BotPlace::of(SCOPE, &watched("task", Some("2026-11-20T00:00:00Z")))
                .unwrap()
                .registration_id
        );
        let moved = WatchedPlace {
            radius_m: 300.0,
            ..watched("task", None)
        };
        assert_ne!(
            first.registration_id,
            BotPlace::of(SCOPE, &moved).unwrap().registration_id
        );
        assert_ne!(
            first.registration_id,
            BotPlace::of("other", &watched("task", None))
                .unwrap()
                .registration_id
        );
        let registration = first.registration();
        assert_eq!(registration.app_id, "bot");
        assert_eq!(registration.event_id, "task");
        assert!(registration.background);
    }

    #[test]
    fn unusable_places_are_not_stored() {
        let conn = db();
        let bad = |change: fn(&mut WatchedPlace)| {
            let mut place = watched("bad", None);
            change(&mut place);
            place
        };
        let places = [
            watched("good", None),
            bad(|place| place.latitude = 91.0),
            bad(|place| place.radius_m = 50.0),
            bad(|place| place.longitude = f64::NAN),
        ];
        assert_eq!(replace(&conn, SCOPE, &places).unwrap(), 1);
        assert_eq!(list(&conn, SCOPE).unwrap().len(), 1);
    }

    #[test]
    fn places_replace_per_scope_and_prune_drops_other_scopes() {
        let conn = db();
        replace(&conn, "a", &[watched("one", None), watched("two", None)]).unwrap();
        replace(&conn, "b", &[watched("three", None)]).unwrap();
        replace(&conn, "a", &[watched("four", None)]).unwrap();
        let tasks = |scope| {
            list(&conn, scope)
                .unwrap()
                .into_iter()
                .map(|place| place.task_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(tasks("a"), ["four"]);
        assert_eq!(tasks("b"), ["three"]);
        prune(&conn, Some("a")).unwrap();
        assert!(tasks("b").is_empty());
        assert_eq!(tasks("a"), ["four"]);
        prune(&conn, None).unwrap();
        assert!(tasks("a").is_empty());
    }

    #[test]
    fn places_end_soonest_first_open_ended_last_and_expired_are_dropped() {
        let places = [
            place("open", None),
            place("late", Some(NOW + 2_000)),
            place("expired", Some(NOW - 1)),
            place("soon", Some(NOW + 1_000)),
        ];
        let tasks = |slots| {
            select(&places, NOW, slots)
                .into_iter()
                .map(|place| place.task_id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(tasks(10), ["soon", "late", "open"]);
        assert_eq!(tasks(2), ["soon", "late"]);
        assert!(tasks(0).is_empty());
    }

    #[test]
    fn app_registrations_come_first_and_bot_places_fill_the_slots_left() {
        let conn = db();
        replace(
            &conn,
            SCOPE,
            &[
                watched("open", None),
                watched("late", Some("2027-01-02T00:00:00Z")),
                watched("soon", Some("2027-01-01T00:00:00Z")),
                watched("gone", Some("2020-01-01T00:00:00Z")),
            ],
        )
        .unwrap();
        let app = [app_registration("a"), app_registration("b")];
        let now = 1_790_000_000_000;
        let ids = |limit| {
            merge(&app, &conn, SCOPE, now, limit)
                .unwrap()
                .into_iter()
                .map(|registration| registration.event_id)
                .collect::<Vec<_>>()
        };
        if WATCHES {
            assert_eq!(ids(4), ["a", "b", "soon", "late"]);
            assert_eq!(ids(2), ["a", "b"]);
            assert_eq!(ids(20), ["a", "b", "soon", "late", "open"]);
        } else {
            assert_eq!(ids(20), ["a", "b"]);
        }
        let live = live_registrations(&conn, SCOPE, now).unwrap();
        assert_eq!(live.len(), 3);
    }

    #[test]
    fn hub_answers_map_to_what_happens_to_the_crossing() {
        for status in [200, 204, 401, 403, 400, 409] {
            assert_eq!(verdict(status), Verdict::Done, "{status}");
        }
        assert_eq!(verdict(404), Verdict::Gone);
        for status in [408, 429, 500, 502, 503] {
            assert_eq!(verdict(status), Verdict::Retry, "{status}");
        }
    }

    #[test]
    fn settling_follows_the_verdict_and_the_delivery_window() {
        assert_eq!(settle(&Ok(Verdict::Done), true), Settle::Complete);
        assert_eq!(settle(&Ok(Verdict::Gone), true), Settle::Forget);
        assert_eq!(settle(&Ok(Verdict::Retry), true), Settle::Retry);
        assert_eq!(settle(&Ok(Verdict::Retry), false), Settle::Complete);
        assert_eq!(settle(&Err(anyhow!("offline")), true), Settle::Retry);
        assert_eq!(settle(&Err(anyhow!("offline")), false), Settle::Complete);
    }

    #[test]
    fn a_crossing_report_carries_direction_and_time() {
        let transition = Transition {
            id: "t".into(),
            registration_id: "r".into(),
            scope: SCOPE.into(),
            app_id: "bot".into(),
            event_id: "task".into(),
            transition: "exit".into(),
            occurred_at: 1_800_000_000_000,
            geometry: serde_json::json!({}),
            radius_meters: 150.0,
        };
        let report = report_of(&transition).unwrap();
        assert_eq!(report.crossing, PlaceCrossing::Exit);
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(&report.occurred_at)
                .unwrap()
                .timestamp_millis(),
            1_800_000_000_000
        );
        let both = Transition {
            transition: "both".into(),
            ..transition
        };
        assert!(report_of(&both).is_err());
    }

    #[test]
    fn the_refresh_clock_runs_on_start_foreground_request_and_every_quarter_hour() {
        let mut clock = RefreshClock::default();
        let start = Instant::now();
        assert!(clock.due(start, false, false));
        clock.finished(start, true);
        assert!(!clock.due(start + Duration::from_secs(60), false, false));
        assert!(clock.due(start + Duration::from_secs(61), true, false));
        assert!(!clock.due(start + Duration::from_secs(62), true, false));
        assert!(clock.due(start + Duration::from_secs(63), true, true));
        assert!(clock.due(start + REFRESH_EVERY, true, false));
        clock.finished(start, false);
        assert!(!clock.due(start + Duration::from_secs(30), true, false));
        assert!(clock.due(start + REFRESH_RETRY, true, false));
    }
}
