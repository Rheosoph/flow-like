//! Run-once cron tick shared by the Azure and GCP scheduler jobs.
//!
//! On Azure and GCP the API's own `SINK_SCHEDULER_PROVIDER` is unset, so this
//! tick — invoked every minute as a Container Apps / Cloud Run job — is the only
//! thing that dispatches cron sinks. Per enabled schedule it reads the last
//! fired instant from a per-schedule state document, computes the cron
//! occurrences in `(last_fired_at, now]` (bounded by the catch-up limit),
//! **claims** the schedule with a compare-and-swap on the document's
//! ETag / updateTime, and only then POSTs one trigger per occurrence. The CAS is
//! what makes overlapping or duplicated ticks safe: two ticks that both see the
//! same occurrence race on one write and exactly one of them fires. The
//! `Idempotency-Key` on each POST is a second, weaker guard on the API side.
//!
//! The crate is a library on purpose: `apps/backend/azure/scheduler` and
//! `apps/backend/gcp/scheduler` are thin binaries that guard their cloud's
//! forbidden env, build the matching [`TickStore`], call [`run_tick`] and exit
//! with [`exit_code`].

pub mod api;
pub mod cron;
pub mod store;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::{StreamExt, stream};
use std::{fmt, time::Duration};

pub use self::cron::{CronError, normalize_cron_for_crate, occurrences_in};
pub use api::{ApiClient, ApiError, CronScheduleInfo, TriggerApi};

pub const DEFAULT_MAX_CATCHUP_SECS: u64 = 600;
pub const DEFAULT_TICK_DEADLINE_SECS: u64 = 200;
pub const DEFAULT_FANOUT: usize = 16;
/// Upper bound on occurrences one schedule may fire in one tick. Thirty means an
/// every-second expression is refused outright (a 60 s window already exceeds
/// it) while `*/10 * * * * *` still catches up across a five-minute outage.
pub const DEFAULT_MAX_OCCURRENCES: usize = 30;
/// A schedule with no state document fires from this minute forward: its window
/// opens one tick interval before `now`, never further back.
const MISSING_STATE_LOOKBACK_SECS: i64 = 60;

/// A string that must never reach a log line or a panic message. `Debug` and
/// `Display` print a fixed placeholder; the only way to the bytes is [`Secret::expose`].
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Clone, Debug)]
pub struct TickConfig {
    /// Normalised (scheme, host, optional path, no trailing slash) API origin.
    pub api_base_url: String,
    /// Set only when `ALLOW_INSECURE_API_BASE_URL` let a plain-HTTP URL through;
    /// mirrored into the HTTP client so it refuses plaintext otherwise.
    pub allow_insecure_api_base_url: bool,
    /// Pre-minted HS256 token with `sub=sink-trigger`, `sink_types=["cron"]`.
    pub jwt: Secret,
    /// How far behind `now` a window may open. Bounds catch-up after an outage.
    pub max_catchup: Duration,
    /// Wall-clock budget for the whole tick, well inside the job's timeout.
    pub deadline: Duration,
    /// Number of schedules processed concurrently.
    pub fanout: usize,
    /// Per-schedule occurrence cap per tick; see [`DEFAULT_MAX_OCCURRENCES`].
    pub max_occurrences: usize,
}

impl TickConfig {
    /// Reads `API_BASE_URL` (falling back to `API_URL`), `SINK_TRIGGER_JWT`,
    /// `SCHEDULER_MAX_CATCHUP_SECS`, `SCHEDULER_TICK_DEADLINE_SECS` and
    /// `ALLOW_INSECURE_API_BASE_URL`. Fan-out and the occurrence cap are not
    /// operator-tunable: both are sized against the API's per-replica trigger
    /// queue rather than against any deployment property.
    pub fn from_env() -> Result<Self, TickError> {
        let raw_url = non_empty_env("API_BASE_URL")
            .or_else(|| non_empty_env("API_URL"))
            .ok_or_else(|| TickError::Configuration("API_BASE_URL is required".to_string()))?;
        let allow_insecure_api_base_url = std::env::var("ALLOW_INSECURE_API_BASE_URL")
            .ok()
            .is_some_and(|value| {
                matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
            });
        let api_base_url = normalize_api_base_url(&raw_url, allow_insecure_api_base_url)
            .map_err(TickError::Configuration)?;

        let jwt = non_empty_env("SINK_TRIGGER_JWT")
            .ok_or_else(|| TickError::Configuration("SINK_TRIGGER_JWT is required".to_string()))?;
        validate_jwt_shape(&jwt).map_err(TickError::Configuration)?;

        let max_catchup_secs = env_u64(
            "SCHEDULER_MAX_CATCHUP_SECS",
            DEFAULT_MAX_CATCHUP_SECS,
            60,
            86_400,
        )?;
        let deadline_secs = env_u64(
            "SCHEDULER_TICK_DEADLINE_SECS",
            DEFAULT_TICK_DEADLINE_SECS,
            1,
            3_600,
        )?;

        Ok(Self {
            api_base_url,
            allow_insecure_api_base_url,
            jwt: Secret::new(jwt),
            max_catchup: Duration::from_secs(max_catchup_secs),
            deadline: Duration::from_secs(deadline_secs),
            fanout: DEFAULT_FANOUT,
            max_occurrences: DEFAULT_MAX_OCCURRENCES,
        })
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u64(name: &str, default: u64, min: u64, max: u64) -> Result<u64, TickError> {
    let Some(raw) = non_empty_env(name) else {
        return Ok(default);
    };
    let value = raw.parse::<u64>().map_err(|_| {
        TickError::Configuration(format!(
            "{name} must be an integer between {min} and {max}, got '{raw}'"
        ))
    })?;
    if !(min..=max).contains(&value) {
        return Err(TickError::Configuration(format!(
            "{name} must be between {min} and {max}, got {value}"
        )));
    }
    Ok(value)
}

/// A JWT is three non-empty base64url segments. This catches the two mistakes
/// that otherwise surface only as a 401 from the API: an empty secret and a
/// value that was pasted with its `Bearer ` prefix or a trailing newline.
fn validate_jwt_shape(value: &str) -> Result<(), String> {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(
            "SINK_TRIGGER_JWT is not a compact JWT (expected header.payload.signature)".to_string(),
        );
    }
    if value.chars().any(char::is_whitespace) {
        return Err("SINK_TRIGGER_JWT must not contain whitespace".to_string());
    }
    Ok(())
}

/// Same rules as `apps/backend/aws/maintenance`: absolute HTTP(S) URL, no
/// credentials, no query or fragment, HTTPS unless the caller explicitly
/// accepted plaintext for trusted private networking.
pub fn normalize_api_base_url(value: &str, allow_insecure: bool) -> Result<String, String> {
    let value = value.trim();
    let parsed = reqwest::Url::parse(value)
        .map_err(|error| format!("API_BASE_URL is not a valid URL: {error}"))?;

    if parsed.host_str().is_none() || parsed.cannot_be_a_base() {
        return Err("API_BASE_URL must be an absolute HTTP(S) URL".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("API_BASE_URL must not contain credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("API_BASE_URL must not contain a query or fragment".to_string());
    }
    if parsed.scheme() != "https" && !(allow_insecure && parsed.scheme() == "http") {
        return Err(
            "API_BASE_URL must use HTTPS; set ALLOW_INSECURE_API_BASE_URL=1 only for trusted development or private networking"
                .to_string(),
        );
    }

    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

/// Refuse to start unless `SCHEDULER_STATE_BACKEND` names the store this binary
/// was built with. The two scheduler images share this crate and differ only in
/// the store, so a mis-set backend must fail loudly rather than write cron
/// state into a container that no other job reads.
pub fn require_state_backend(expected: &str) -> Result<(), TickError> {
    match non_empty_env("SCHEDULER_STATE_BACKEND") {
        Some(actual) if actual.eq_ignore_ascii_case(expected) => Ok(()),
        Some(actual) => Err(TickError::Configuration(format!(
            "SCHEDULER_STATE_BACKEND is '{actual}' but this binary only supports '{expected}'"
        ))),
        None => Err(TickError::Configuration(format!(
            "SCHEDULER_STATE_BACKEND is required (expected '{expected}')"
        ))),
    }
}

// ---------------------------------------------------------------------------
// State store
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);

/// The state document as the tick needs it. `version` is the store's
/// concurrency token (Cosmos `_etag`, Firestore `updateTime`) and is opaque
/// here: it only ever travels back into [`TickStore::claim`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleState {
    pub last_fired_at: DateTime<Utc>,
    pub cron_expression: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    /// The precondition failed: another tick wrote the document after this one
    /// read it (or created it when this one saw none). Skip the schedule.
    Lost,
}

/// One document per schedule, addressed by `(event_id, app_id)`; `app_id` is
/// the Cosmos partition key and is carried for symmetry on Firestore.
#[async_trait]
pub trait TickStore: Send + Sync {
    async fn read(&self, event_id: &str, app_id: &str)
    -> Result<Option<ScheduleState>, StoreError>;

    /// Compare-and-swap `last_fired_at` to `new_last_fired_at`. With
    /// `expected_version == None` the document must not exist yet (create), else
    /// the stored token must equal `expected_version` (replace). Both refusals
    /// come back as [`ClaimOutcome::Lost`]; anything else is an error.
    async fn claim(
        &self,
        event_id: &str,
        app_id: &str,
        cron_expression: &str,
        expected_version: Option<&str>,
        new_last_fired_at: DateTime<Utc>,
    ) -> Result<ClaimOutcome, StoreError>;
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum TickError {
    #[error("scheduler configuration error: {0}")]
    Configuration(String),
    #[error("listing cron schedules failed: {0}")]
    ListSchedules(#[from] ApiError),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Enabled schedules the tick considered.
    pub schedules: usize,
    /// Schedules whose claim succeeded (whether or not anything was due).
    pub claimed: usize,
    /// Trigger POSTs the API accepted.
    pub fired: usize,
    /// Schedules another tick owned at the moment of the claim.
    pub skipped_lost_claim: usize,
    /// Schedules skipped because the expression could not be parsed or would
    /// fire more than the cap in this window. Logged, not fatal: this is user
    /// data, and a stuck job would alert every minute for one bad expression.
    pub rejected: usize,
    /// Schedules with at least one read, claim or fire error.
    pub failed: usize,
    /// Individual occurrences whose trigger POST failed after the claim.
    pub failed_fires: usize,
    pub deadline_exceeded: bool,
}

/// Non-zero whenever something needs an operator: a fire or store failure, or a
/// tick that did not finish inside its deadline. Lost claims and rejected
/// expressions are expected steady-state outcomes and do not fail the run.
pub fn exit_code(report: &TickReport) -> i32 {
    if report.failed == 0 && !report.deadline_exceeded {
        0
    } else {
        1
    }
}

enum ScheduleOutcome {
    /// Stored watermark is already at or past `now`; nothing read or written.
    Ahead,
    Rejected,
    LostClaim,
    Claimed {
        fired: usize,
        failed_fires: usize,
    },
    Failed,
}

/// Run one tick at instant `now`. Errors only when the schedule listing itself
/// fails; every per-schedule problem is folded into the report so one broken
/// schedule cannot stop the others from firing.
pub async fn run_tick<S, A>(
    config: &TickConfig,
    store: &S,
    api: &A,
    now: DateTime<Utc>,
) -> Result<TickReport, TickError>
where
    S: TickStore + ?Sized,
    A: TriggerApi + ?Sized,
{
    let started = tokio::time::Instant::now();
    let deadline_at = started + config.deadline;

    let schedules: Vec<CronScheduleInfo> = api
        .list_cron_schedules()
        .await?
        .into_iter()
        .filter(|schedule| schedule.enabled)
        .collect();

    let mut report = TickReport {
        schedules: schedules.len(),
        ..TickReport::default()
    };

    let mut outcomes = stream::iter(schedules)
        .map(|schedule| async move { process_schedule(config, store, api, &schedule, now).await })
        .buffer_unordered(config.fanout.max(1));

    // The deadline is checked per completed schedule rather than around the whole
    // stream so the counts gathered so far survive a timeout. Dropping the
    // stream cancels the in-flight futures; a claim that was written without
    // its fires is caught up by the next tick through the capped window.
    loop {
        match tokio::time::timeout_at(deadline_at, outcomes.next()).await {
            Ok(Some(outcome)) => record(&mut report, outcome),
            Ok(None) => break,
            Err(_) => {
                report.deadline_exceeded = true;
                break;
            }
        }
    }
    drop(outcomes);

    tracing::info!(
        schedules = report.schedules,
        claimed = report.claimed,
        fired = report.fired,
        skipped_lost_claim = report.skipped_lost_claim,
        rejected = report.rejected,
        failed = report.failed,
        failed_fires = report.failed_fires,
        deadline_exceeded = report.deadline_exceeded,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "scheduler tick finished"
    );
    Ok(report)
}

fn record(report: &mut TickReport, outcome: ScheduleOutcome) {
    match outcome {
        ScheduleOutcome::Ahead => {}
        ScheduleOutcome::Rejected => report.rejected += 1,
        ScheduleOutcome::LostClaim => report.skipped_lost_claim += 1,
        ScheduleOutcome::Claimed {
            fired,
            failed_fires,
        } => {
            report.claimed += 1;
            report.fired += fired;
            report.failed_fires += failed_fires;
            if failed_fires > 0 {
                report.failed += 1;
            }
        }
        ScheduleOutcome::Failed => report.failed += 1,
    }
}

async fn process_schedule<S, A>(
    config: &TickConfig,
    store: &S,
    api: &A,
    schedule: &CronScheduleInfo,
    now: DateTime<Utc>,
) -> ScheduleOutcome
where
    S: TickStore + ?Sized,
    A: TriggerApi + ?Sized,
{
    let event_id = schedule.event_id.as_str();
    let app_id = schedule.app_id.as_str();

    let state = match store.read(event_id, app_id).await {
        Ok(state) => state,
        Err(error) => {
            tracing::error!(event_id, app_id, %error, "reading scheduler state failed");
            return ScheduleOutcome::Failed;
        }
    };

    // Missing document: open the window one tick back so a brand-new schedule
    // fires this minute's occurrence but never back-fills history. Existing
    // document: continue from where the last claim left off, bounded by the
    // catch-up limit so an outage does not replay hours of backlog.
    let window_start = match &state {
        Some(state) => state.last_fired_at,
        None => now - ChronoDuration::seconds(MISSING_STATE_LOOKBACK_SECS),
    };
    let catchup_floor = now
        - ChronoDuration::from_std(config.max_catchup)
            .unwrap_or_else(|_| ChronoDuration::seconds(DEFAULT_MAX_CATCHUP_SECS as i64));
    let window_start = window_start.max(catchup_floor);

    // A stored `last_fired_at` at or beyond `now` means a tick with a later
    // clock already owns everything up to its instant. Writing our earlier `now`
    // would move the watermark backwards and re-open occurrences it fired.
    if window_start >= now {
        tracing::debug!(
            event_id,
            %window_start,
            %now,
            "state is already ahead of this tick; nothing to do"
        );
        return ScheduleOutcome::Ahead;
    }

    // The expression is evaluated before the claim so a schedule that can
    // never fire is not written every minute. An over-cap window is different:
    // it is claimed with nothing fired, so a burst after an outage advances the
    // watermark instead of wedging the schedule at "too many" forever.
    let (occurrences, over_cap) = match occurrences_in(
        &schedule.cron_expression,
        window_start,
        now,
        config.max_occurrences,
    ) {
        Ok(occurrences) => (occurrences, false),
        Err(error @ CronError::Parse { .. }) => {
            tracing::warn!(event_id, app_id, %error, "skipping schedule with unparseable cron expression");
            return ScheduleOutcome::Rejected;
        }
        Err(error @ CronError::TooManyOccurrences { .. }) => {
            tracing::warn!(
                event_id,
                app_id,
                %error,
                "schedule exceeds the per-tick occurrence cap; claiming the window without firing"
            );
            (Vec::new(), true)
        }
    };

    let claim = store
        .claim(
            event_id,
            app_id,
            &schedule.cron_expression,
            state.as_ref().map(|state| state.version.as_str()),
            now,
        )
        .await;
    match claim {
        Ok(ClaimOutcome::Claimed) => {}
        Ok(ClaimOutcome::Lost) => {
            tracing::debug!(
                event_id,
                app_id,
                "another tick owns this schedule; skipping"
            );
            return ScheduleOutcome::LostClaim;
        }
        Err(error) => {
            tracing::error!(event_id, app_id, %error, "claiming scheduler state failed");
            return ScheduleOutcome::Failed;
        }
    }
    if over_cap {
        return ScheduleOutcome::Rejected;
    }

    // Past the claim there is no rollback: a failed POST is logged and counted,
    // the remaining occurrences still fire, and the job exits non-zero. Rolling
    // the watermark back would replay the whole window and double-fire the
    // occurrences that succeeded.
    let mut fired = 0;
    let mut failed_fires = 0;
    for occurrence in occurrences {
        match api.trigger(event_id, occurrence).await {
            Ok(()) => {
                fired += 1;
                tracing::info!(
                    event_id,
                    app_id,
                    occurrence = %crate::api::occurrence_rfc3339(occurrence),
                    "cron occurrence fired"
                );
            }
            Err(error) => {
                failed_fires += 1;
                tracing::error!(
                    event_id,
                    app_id,
                    occurrence = %crate::api::occurrence_rfc3339(occurrence),
                    %error,
                    "cron trigger failed after the claim; not rolling back"
                );
            }
        }
    }
    ScheduleOutcome::Claimed {
        fired,
        failed_fires,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    // -- in-memory store ---------------------------------------------------

    #[derive(Clone, Debug)]
    struct StoredDoc {
        last_fired_at: DateTime<Utc>,
        cron_expression: String,
        version: u64,
    }

    #[derive(Default)]
    struct MemoryStore {
        docs: Mutex<HashMap<(String, String), StoredDoc>>,
        next_version: AtomicU64,
        /// When set, `read` answers with this snapshot instead of the live map,
        /// modelling a tick that read the document before a competitor's claim.
        stale_read: Mutex<Option<Option<ScheduleState>>>,
    }

    impl MemoryStore {
        fn seed(&self, event_id: &str, app_id: &str, expr: &str, last_fired_at: DateTime<Utc>) {
            let version = self.next_version.fetch_add(1, Ordering::SeqCst) + 1;
            self.docs.lock().unwrap().insert(
                (event_id.to_string(), app_id.to_string()),
                StoredDoc {
                    last_fired_at,
                    cron_expression: expr.to_string(),
                    version,
                },
            );
        }

        fn doc(&self, event_id: &str, app_id: &str) -> Option<StoredDoc> {
            self.docs
                .lock()
                .unwrap()
                .get(&(event_id.to_string(), app_id.to_string()))
                .cloned()
        }
    }

    #[async_trait]
    impl TickStore for MemoryStore {
        async fn read(
            &self,
            event_id: &str,
            app_id: &str,
        ) -> Result<Option<ScheduleState>, StoreError> {
            if let Some(snapshot) = self.stale_read.lock().unwrap().clone() {
                return Ok(snapshot);
            }
            Ok(self.doc(event_id, app_id).map(|doc| ScheduleState {
                last_fired_at: doc.last_fired_at,
                cron_expression: doc.cron_expression,
                version: doc.version.to_string(),
            }))
        }

        async fn claim(
            &self,
            event_id: &str,
            app_id: &str,
            cron_expression: &str,
            expected_version: Option<&str>,
            new_last_fired_at: DateTime<Utc>,
        ) -> Result<ClaimOutcome, StoreError> {
            let mut docs = self.docs.lock().unwrap();
            let key = (event_id.to_string(), app_id.to_string());
            let matches = match (docs.get(&key), expected_version) {
                (None, None) => true,
                (Some(doc), Some(expected)) => doc.version.to_string() == expected,
                _ => false,
            };
            if !matches {
                return Ok(ClaimOutcome::Lost);
            }
            let version = self.next_version.fetch_add(1, Ordering::SeqCst) + 1;
            docs.insert(
                key,
                StoredDoc {
                    last_fired_at: new_last_fired_at,
                    cron_expression: cron_expression.to_string(),
                    version,
                },
            );
            Ok(ClaimOutcome::Claimed)
        }
    }

    // -- mock API ----------------------------------------------------------

    #[derive(Default)]
    struct MockApi {
        schedules: Vec<CronScheduleInfo>,
        triggers: Mutex<Vec<(String, DateTime<Utc>)>>,
        fail_occurrences: Vec<DateTime<Utc>>,
        trigger_delay: Option<Duration>,
    }

    #[async_trait]
    impl TriggerApi for MockApi {
        async fn list_cron_schedules(&self) -> Result<Vec<CronScheduleInfo>, ApiError> {
            Ok(self.schedules.clone())
        }

        async fn trigger(&self, event_id: &str, occurrence: DateTime<Utc>) -> Result<(), ApiError> {
            if let Some(delay) = self.trigger_delay {
                tokio::time::sleep(delay).await;
            }
            if self.fail_occurrences.contains(&occurrence) {
                return Err(ApiError::Status {
                    status: reqwest::StatusCode::BAD_GATEWAY,
                    body: "upstream down".to_string(),
                });
            }
            self.triggers
                .lock()
                .unwrap()
                .push((event_id.to_string(), occurrence));
            Ok(())
        }
    }

    // -- helpers -----------------------------------------------------------

    fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 15, h, m, s).unwrap()
    }

    fn config() -> TickConfig {
        TickConfig {
            api_base_url: "https://api.example.com".to_string(),
            allow_insecure_api_base_url: false,
            jwt: Secret::new("a.b.c"),
            max_catchup: Duration::from_secs(DEFAULT_MAX_CATCHUP_SECS),
            deadline: Duration::from_secs(DEFAULT_TICK_DEADLINE_SECS),
            fanout: DEFAULT_FANOUT,
            max_occurrences: DEFAULT_MAX_OCCURRENCES,
        }
    }

    fn schedule(event_id: &str, expr: &str) -> CronScheduleInfo {
        CronScheduleInfo {
            id: event_id.to_string(),
            event_id: event_id.to_string(),
            cron_expression: expr.to_string(),
            app_id: "app_1".to_string(),
            enabled: true,
            last_triggered: None,
            next_trigger: None,
        }
    }

    fn fired(api: &MockApi) -> Vec<(String, DateTime<Utc>)> {
        let mut triggers = api.triggers.lock().unwrap().clone();
        triggers.sort();
        triggers
    }

    // -- tests -------------------------------------------------------------

    #[tokio::test]
    async fn missing_document_fires_only_the_current_minute() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };
        let now = at(12, 5, 30);

        let report = run_tick(&config(), &store, &api, now).await.unwrap();

        assert_eq!(fired(&api), vec![("evt".to_string(), at(12, 5, 0))]);
        assert_eq!(report.schedules, 1);
        assert_eq!(report.claimed, 1);
        assert_eq!(report.fired, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(exit_code(&report), 0);
        let doc = store
            .doc("evt", "app_1")
            .expect("claim created the document");
        assert_eq!(doc.last_fired_at, now);
        assert_eq!(doc.cron_expression, "* * * * *");
    }

    #[tokio::test]
    async fn two_ticks_racing_on_one_schedule_fire_once() {
        let store = MemoryStore::default();
        store.seed("evt", "app_1", "* * * * *", at(12, 4, 30));
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };

        // Tick A reads version 1 and claims it.
        let stale = store.read("evt", "app_1").await.unwrap();
        let report_a = run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();
        assert_eq!(report_a.fired, 1);

        // Tick B read the same version 1 before A's write landed; its claim loses.
        *store.stale_read.lock().unwrap() = Some(stale);
        let report_b = run_tick(&config(), &store, &api, at(12, 5, 33))
            .await
            .unwrap();
        assert_eq!(report_b.skipped_lost_claim, 1);
        assert_eq!(report_b.fired, 0);
        assert_eq!(report_b.failed, 0);
        assert_eq!(exit_code(&report_b), 0);

        assert_eq!(fired(&api), vec![("evt".to_string(), at(12, 5, 0))]);
        assert_eq!(
            store.doc("evt", "app_1").unwrap().last_fired_at,
            at(12, 5, 30)
        );
    }

    #[tokio::test]
    async fn two_ticks_racing_on_a_missing_document_create_once() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };

        let report_a = run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();
        assert_eq!(report_a.fired, 1);

        *store.stale_read.lock().unwrap() = Some(None);
        let report_b = run_tick(&config(), &store, &api, at(12, 5, 31))
            .await
            .unwrap();
        assert_eq!(report_b.skipped_lost_claim, 1);
        assert_eq!(fired(&api).len(), 1);
    }

    #[tokio::test]
    async fn sequential_ticks_do_not_refire_the_same_occurrence() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };

        run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();
        let report = run_tick(&config(), &store, &api, at(12, 5, 50))
            .await
            .unwrap();
        assert_eq!(report.claimed, 1);
        assert_eq!(report.fired, 0);
        let report = run_tick(&config(), &store, &api, at(12, 6, 30))
            .await
            .unwrap();
        assert_eq!(report.fired, 1);
        assert_eq!(
            fired(&api),
            vec![
                ("evt".to_string(), at(12, 5, 0)),
                ("evt".to_string(), at(12, 6, 0))
            ]
        );
    }

    #[tokio::test]
    async fn catch_up_is_capped_by_max_catchup() {
        let store = MemoryStore::default();
        store.seed("evt", "app_1", "* * * * *", at(11, 0, 30));
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };
        let now = at(12, 5, 30);

        let report = run_tick(&config(), &store, &api, now).await.unwrap();

        // Window is (11:55:30, 12:05:30]: 11:56 .. 12:05 inclusive.
        let occurrences: Vec<DateTime<Utc>> = fired(&api).into_iter().map(|(_, at)| at).collect();
        assert_eq!(occurrences.len(), 10);
        assert_eq!(occurrences.first(), Some(&at(11, 56, 0)));
        assert_eq!(occurrences.last(), Some(&at(12, 5, 0)));
        assert_eq!(report.fired, 10);
        assert_eq!(store.doc("evt", "app_1").unwrap().last_fired_at, now);
    }

    #[tokio::test]
    async fn fire_failure_exits_non_zero_without_rolling_back_the_claim() {
        let store = MemoryStore::default();
        store.seed("evt", "app_1", "* * * * *", at(12, 2, 30));
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            fail_occurrences: vec![at(12, 4, 0)],
            ..MockApi::default()
        };
        let now = at(12, 5, 30);

        let report = run_tick(&config(), &store, &api, now).await.unwrap();

        assert_eq!(report.fired, 2);
        assert_eq!(report.failed_fires, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(exit_code(&report), 1);
        assert_eq!(
            fired(&api),
            vec![
                ("evt".to_string(), at(12, 3, 0)),
                ("evt".to_string(), at(12, 5, 0))
            ]
        );
        assert_eq!(store.doc("evt", "app_1").unwrap().last_fired_at, now);

        // The next tick continues from the claim; the failed occurrence is not replayed.
        let report = run_tick(&config(), &store, &api, at(12, 6, 30))
            .await
            .unwrap();
        assert_eq!(report.fired, 1);
        assert_eq!(fired(&api).len(), 3);
    }

    #[tokio::test]
    async fn over_cap_windows_are_claimed_but_not_fired() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * * *")],
            ..MockApi::default()
        };
        let now = at(12, 5, 30);

        let report = run_tick(&config(), &store, &api, now).await.unwrap();

        assert_eq!(report.rejected, 1);
        assert_eq!(report.fired, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(exit_code(&report), 0);
        assert!(fired(&api).is_empty());
        assert_eq!(store.doc("evt", "app_1").unwrap().last_fired_at, now);
    }

    #[tokio::test]
    async fn unparseable_expressions_are_rejected_without_a_claim() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "every tuesday")],
            ..MockApi::default()
        };

        let report = run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();

        assert_eq!(report.rejected, 1);
        assert_eq!(report.claimed, 0);
        assert!(store.doc("evt", "app_1").is_none());
        assert_eq!(exit_code(&report), 0);
    }

    #[tokio::test]
    async fn disabled_schedules_are_ignored() {
        let store = MemoryStore::default();
        let mut disabled = schedule("evt", "* * * * *");
        disabled.enabled = false;
        let api = MockApi {
            schedules: vec![disabled],
            ..MockApi::default()
        };

        let report = run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();

        assert_eq!(report, TickReport::default());
        assert!(store.doc("evt", "app_1").is_none());
    }

    #[tokio::test]
    async fn state_ahead_of_now_is_left_alone() {
        let store = MemoryStore::default();
        store.seed("evt", "app_1", "* * * * *", at(12, 5, 35));
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            ..MockApi::default()
        };

        let report = run_tick(&config(), &store, &api, at(12, 5, 30))
            .await
            .unwrap();

        assert_eq!(report.schedules, 1);
        assert_eq!(report.claimed, 0);
        assert_eq!(report.fired, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(
            store.doc("evt", "app_1").unwrap().last_fired_at,
            at(12, 5, 35)
        );
    }

    #[tokio::test]
    async fn deadline_exceeded_is_reported_and_fails_the_run() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![schedule("evt", "* * * * *")],
            trigger_delay: Some(Duration::from_secs(5)),
            ..MockApi::default()
        };
        let mut config = config();
        config.deadline = Duration::from_millis(50);

        let report = run_tick(&config, &store, &api, at(12, 5, 30))
            .await
            .unwrap();

        assert!(report.deadline_exceeded);
        assert_eq!(exit_code(&report), 1);
    }

    #[tokio::test]
    async fn multiple_schedules_are_processed_independently() {
        let store = MemoryStore::default();
        let api = MockApi {
            schedules: vec![
                schedule("every_minute", "* * * * *"),
                schedule("hourly", "0 * * * *"),
                schedule("broken", "?? ??"),
            ],
            ..MockApi::default()
        };

        let report = run_tick(&config(), &store, &api, at(12, 0, 30))
            .await
            .unwrap();

        assert_eq!(report.schedules, 3);
        assert_eq!(report.claimed, 2);
        assert_eq!(report.fired, 2);
        assert_eq!(report.rejected, 1);
        assert_eq!(
            fired(&api),
            vec![
                ("every_minute".to_string(), at(12, 0, 0)),
                ("hourly".to_string(), at(12, 0, 0))
            ]
        );
    }

    // -- config ------------------------------------------------------------

    #[test]
    fn secret_never_prints_its_value() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret([redacted])");
        assert_eq!(secret.to_string(), "[redacted]");
        assert_eq!(secret.expose(), "hunter2");
        let config = TickConfig {
            jwt: Secret::new("hunter2"),
            ..config()
        };
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    #[test]
    fn url_is_normalized() {
        assert_eq!(
            normalize_api_base_url(" https://api.example.com/ ", false).unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            normalize_api_base_url("https://api.example.com/base/", false).unwrap(),
            "https://api.example.com/base"
        );
        assert!(normalize_api_base_url("https://user:pw@api.example.com", false).is_err());
        assert!(normalize_api_base_url("https://api.example.com/?x=1", false).is_err());
        assert!(normalize_api_base_url("not a url", false).is_err());
    }

    #[test]
    fn insecure_api_urls_require_an_explicit_override() {
        assert!(normalize_api_base_url("http://api.internal", false).is_err());
        assert_eq!(
            normalize_api_base_url("http://api.internal/", true).unwrap(),
            "http://api.internal"
        );
        assert!(normalize_api_base_url("ftp://api.example.com", true).is_err());
    }

    #[test]
    fn jwt_shape_is_checked() {
        assert!(validate_jwt_shape("aaa.bbb.ccc").is_ok());
        assert!(validate_jwt_shape("").is_err());
        assert!(validate_jwt_shape("aaa.bbb").is_err());
        assert!(validate_jwt_shape("Bearer aaa.bbb.ccc").is_err());
        assert!(validate_jwt_shape("aaa..ccc").is_err());
    }
}
