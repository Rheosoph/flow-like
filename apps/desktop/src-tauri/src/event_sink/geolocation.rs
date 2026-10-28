use super::{
    EventConfig, EventRegistration, EventSink,
    manager::{DbConnection, EventSinkManager, Refusal},
    trigger_token::TokenRefused,
};
use anyhow::{Result, anyhow, ensure};
use flow_like_types::{tokio, tokio_util::sync::CancellationToken};
use std::{
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tauri::AppHandle;

mod dispatch;
mod inbox;
mod model;
mod native;
pub use model::GeoLocationSink;
use model::{NativeRegistration, Scope, now_ms};

static WAKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
static SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static WORKER: Mutex<Option<CancellationToken>> = Mutex::new(None);
static ACTIVE: Mutex<Option<(String, CancellationToken)>> = Mutex::new(None);

const NOT_CLEARED: &str =
    "This location trigger was not cleared with the hub before registering; save the event again";
const CHANGED_WHILE_CLEARED: &str = "The location trigger or the signed-in account changed while the hub cleared it; save the event again";

/// What authorizing a location trigger depended on: the account, app, event
/// and region it watches (all bound into the region id), where its crossings
/// run, and the token it signs in with.
#[derive(Clone, PartialEq, Eq)]
struct ClearedTrigger {
    region_id: String,
    remote: bool,
    personal_access_token: Option<String>,
    pat_owner_sub: Option<String>,
    hub_origin: Option<String>,
}

impl ClearedTrigger {
    fn of(config: &GeoLocationSink, registration: &EventRegistration, scope: &str) -> Self {
        Self {
            region_id: config
                .registration(scope, &registration.app_id, &registration.event_id)
                .id,
            remote: registration.remote,
            personal_access_token: registration.personal_access_token.clone(),
            pat_owner_sub: registration.pat_owner_sub.clone(),
            hub_origin: registration.hub_origin.clone(),
        }
    }
}

/// Why a location trigger was not cleared. Registering fails with the same
/// error, so a refused token is dropped and the refusal recorded as when the
/// hub refuses it at registration.
#[derive(Debug, Clone)]
enum Denial {
    Refused(Refusal),
    Failed(String),
}

impl Denial {
    fn of(error: &anyhow::Error) -> Self {
        dispatch::refusal(error).map_or_else(|| Self::Failed(format!("{error:#}")), Self::Refused)
    }

    fn error(&self) -> anyhow::Error {
        match self {
            Self::Refused(Refusal {
                reason,
                token_refused: true,
            }) => TokenRefused(reason.clone()).into(),
            Self::Refused(Refusal { reason, .. }) => dispatch::Rejected(reason.clone()).into(),
            Self::Failed(reason) => anyhow!(reason.clone()),
        }
    }
}

/// The answer to whether a location trigger may watch its region, resolved
/// before a save locks the sink manager: authorizing asks the hub, and the
/// fire paths only try that lock. Registering asks the hub nothing and only
/// checks that the clearance is for the same trigger and account.
#[derive(Clone)]
pub struct GeofenceClearance {
    trigger: Option<ClearedTrigger>,
    denial: Option<Denial>,
}

impl GeofenceClearance {
    /// Authorizes `registration` for the signed-in account. A refusal is kept
    /// and returned when the trigger registers.
    pub async fn resolve(app: &AppHandle, registration: &EventRegistration) -> Self {
        let scope = match dispatch::current_scope(app).await {
            Ok(scope) => scope,
            Err(error) => {
                return Self {
                    trigger: None,
                    denial: Some(Denial::of(&error)),
                };
            }
        };
        let trigger = match &registration.config {
            EventConfig::GeoLocation(config) => {
                Some(ClearedTrigger::of(config, registration, &scope.raw))
            }
            _ => None,
        };
        let denial = dispatch::authorize(app, registration, &scope)
            .await
            .err()
            .map(|error| Denial::of(&error));
        Self { trigger, denial }
    }

    /// Whether `registration` of the region `config` may register in `scope`:
    /// it is the trigger this clearance was resolved for, and was allowed.
    fn admit(
        &self,
        config: &GeoLocationSink,
        registration: &EventRegistration,
        scope: &str,
    ) -> Result<()> {
        ensure!(
            self.trigger.as_ref() == Some(&ClearedTrigger::of(config, registration, scope)),
            CHANGED_WHILE_CLEARED
        );
        self.denial
            .as_ref()
            .map_or(Ok(()), |denial| Err(denial.error()))
    }
}

pub(crate) fn notify() {
    WAKE.notify_one();
}
pub(crate) fn expire() {
    if let Some((_, token)) = ACTIVE.lock().unwrap().as_ref() {
        token.cancel();
    }
    notify();
}
fn cancel_event(event_id: &str) {
    if let Some((id, token)) = ACTIVE.lock().unwrap().as_ref()
        && id == event_id
    {
        token.cancel();
    }
}

fn registrations(db: &DbConnection, scope: &Scope) -> Result<Vec<NativeRegistration>> {
    let rows = EventSinkManager::read_registrations(db.clone())?;
    let conn = db.lock().unwrap();
    let mut current = Vec::new();
    for row in rows {
        let EventConfig::GeoLocation(config) = &row.config else {
            continue;
        };
        if config.validate().is_err() {
            inbox::remove(&conn, &row.event_id)?;
            continue;
        }
        let registration = config.registration(&scope.raw, &row.app_id, &row.event_id);
        if inbox::binding(&conn, &row.event_id, &scope.raw)?.as_deref() == Some(&registration.id) {
            current.push(registration);
        }
    }
    current.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(current)
}

async fn sync_native(app: &AppHandle, db: &DbConnection) -> Result<Scope> {
    let scope = dispatch::current_scope(app).await?;
    inbox::prune(&db.lock().unwrap(), Some(&scope.raw), now_ms())?;
    let current = registrations(db, &scope)?;
    ensure!(
        current.len() <= 20,
        "Apple supports at most 20 monitored geofences per app; disable another Location Event first"
    );
    native::sync(&scope.raw, &current)?;
    Ok(scope)
}

fn still_registered(db: &DbConnection, scope: &Scope, transition: &model::Transition) -> bool {
    let Ok(Some(row)) = EventSinkManager::read_registration(db.clone(), &transition.event_id)
    else {
        return false;
    };
    let EventConfig::GeoLocation(config) = row.config else {
        return false;
    };
    config.validate().is_ok()
        && config
            .registration(&scope.raw, &row.app_id, &row.event_id)
            .id
            == transition.registration_id
        && inbox::binding(&db.lock().unwrap(), &row.event_id, &scope.raw)
            .ok()
            .flatten()
            .as_deref()
            == Some(&transition.registration_id)
}

/// Declines a registration the hub refused, keeping the reason for the
/// event's status. When the manager cannot, only the binding is removed.
async fn decline_refused(
    app: &AppHandle,
    db: &DbConnection,
    row: &EventRegistration,
    refusal: &Refusal,
) -> Result<()> {
    let declined = match crate::state::TauriEventSinkManagerState::construct(app).await {
        Ok(manager) => {
            manager
                .lock()
                .await
                .reject_registration(app, &row.app_id, &row.event_id, refusal)
                .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = declined {
        tracing::warn!(event_id=%row.event_id, error=%error, "Could not decline a refused geofence, removing its binding only");
        inbox::remove(&db.lock().unwrap(), &row.event_id)?;
    }
    Ok(())
}

/// What is left to do with a crossing whose delivery failed.
#[derive(Debug, PartialEq, Eq)]
enum Undelivered {
    /// The hub refused the trigger, so its region is declined.
    Decline(Refusal),
    /// Its region is no longer watched, or it is too old to be sent again.
    Complete,
    Retry,
}

/// What a delivery that failed with `error` leaves to do, given whether the
/// crossing's region is still `registered` and the crossing still
/// `deliverable` (see [`model::Transition::deliverable`]). Anything but a
/// refusal keeps the region, such as a project that is not active right now.
fn undelivered(error: &anyhow::Error, registered: bool, deliverable: bool) -> Undelivered {
    match dispatch::refusal(error) {
        Some(refusal) if registered => Undelivered::Decline(refusal),
        _ if registered && deliverable => Undelivered::Retry,
        _ => Undelivered::Complete,
    }
}

impl GeoLocationSink {
    /// Watches the region of `registration` on this device. Runs while the
    /// sink manager is locked, so it asks the hub nothing: `clearance`, which
    /// the save resolved beforehand, must be for this trigger and account.
    pub(crate) async fn register_cleared(
        &self,
        app: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
        clearance: Option<&GeofenceClearance>,
    ) -> Result<()> {
        self.validate()?;
        let scope = dispatch::current_scope(app).await?;
        clearance
            .ok_or_else(|| anyhow!(NOT_CLEARED))?
            .admit(self, registration, &scope.raw)?;
        let _lock = SYNC.lock().await;
        ensure!(
            native::scope()? == scope.raw,
            "Geofence account changed during registration"
        );
        self.watch(app, registration, &db, &scope).await
    }

    /// Binds the region of `registration` in `scope` and syncs the native
    /// geofences; a failed sync removes the binding again.
    async fn watch(
        &self,
        app: &AppHandle,
        registration: &EventRegistration,
        db: &DbConnection,
        scope: &Scope,
    ) -> Result<()> {
        let region = self.registration(&scope.raw, &registration.app_id, &registration.event_id);
        let others = registrations(db, scope)?
            .iter()
            .filter(|watched| watched.event_id != registration.event_id)
            .count();
        ensure!(
            others < 20,
            "Apple supports at most 20 monitored geofences per app"
        );
        inbox::bind(
            &db.lock().unwrap(),
            &registration.event_id,
            &scope.raw,
            &region.id,
        )?;
        if let Err(error) = sync_native(app, db).await {
            inbox::remove(&db.lock().unwrap(), &registration.event_id)?;
            let _ = sync_native(app, db).await;
            return Err(error);
        }
        notify();
        Ok(())
    }
}

async fn process(app: &AppHandle, db: &DbConnection) -> Result<()> {
    let scope = {
        let _lock = SYNC.lock().await;
        let raw = native::scope()?;
        if raw.is_empty() {
            inbox::prune(&db.lock().unwrap(), None, now_ms())?;
            return Ok(());
        }
        let scope = sync_native(app, db).await?;
        let current = registrations(db, &scope)?;
        let mut acknowledged = Vec::new();
        for transition in native::peek()?.into_iter().take(model::MAX_PENDING) {
            let valid = current
                .iter()
                .any(|registration| transition.matches(&scope.raw, registration, now_ms()));
            if !valid || inbox::accept(&db.lock().unwrap(), &transition)? {
                acknowledged.push(transition.id);
            }
        }
        // The native copy stays durable until the SQLite insertion has committed.
        native::ack(&acknowledged)?;
        scope
    };
    let pending = inbox::pending(&db.lock().unwrap(), &scope.raw, now_ms())?;
    for transition in pending {
        if native::scope()? != scope.raw {
            return Ok(());
        }
        let row = EventSinkManager::read_registration(db.clone(), &transition.event_id)?;
        let Some(row) = row else {
            inbox::complete(&db.lock().unwrap(), &transition.id)?;
            continue;
        };
        let EventConfig::GeoLocation(config) = &row.config else {
            inbox::complete(&db.lock().unwrap(), &transition.id)?;
            continue;
        };
        if !still_registered(db, &scope, &transition)
            || !transition.matches(
                &scope.raw,
                &config.registration(&scope.raw, &row.app_id, &row.event_id),
                now_ms(),
            )
            || !transition.deliverable(row.remote, now_ms())
        {
            inbox::complete(&db.lock().unwrap(), &transition.id)?;
            continue;
        }
        if !config.background && !native::foreground() {
            continue;
        }
        let remaining = native::remaining();
        if (0.0..=1.0).contains(&remaining) {
            break;
        }
        let budget = if remaining < 0.0 {
            3600.0
        } else {
            (remaining - 1.0).min(20.0)
        };
        let cancellation = CancellationToken::new();
        *ACTIVE.lock().unwrap() = Some((row.event_id.clone(), cancellation.clone()));
        let result = {
            let work = dispatch::execute(app, &row, &scope, &transition);
            tokio::pin!(work);
            tokio::select! {
                result = &mut work => result,
                _ = tokio::time::sleep(Duration::from_secs_f64(budget)) => Err(anyhow!("Geofence execution exceeded the device's background time budget")),
                _ = cancellation.cancelled() => Err(anyhow!("Geofence execution was interrupted by the operating system or account change")),
                _ = async {
                    loop {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        let remaining = native::remaining();
                        if native::scope().ok().as_deref() != Some(&scope.raw)
                            || (!config.background && !native::foreground())
                            || (0.0..=1.0).contains(&remaining)
                            || !still_registered(db,&scope,&transition) { break; }
                    }
                } => Err(anyhow!("Geofence execution stopped because its registration or foreground session changed")),
            }
        }; // LocalRunGuard cancels a started run when the execution future is interrupted.
        *ACTIVE.lock().unwrap() = None;
        match result {
            Ok(()) => inbox::complete(&db.lock().unwrap(), &transition.id)?,
            Err(error) => {
                tracing::warn!(event_id=%row.event_id, transition_id=%transition.id, error=%error, "Geofence Event dispatch failed");
                let registered = still_registered(db, &scope, &transition);
                let deliverable = transition.deliverable(row.remote, now_ms());
                match undelivered(&error, registered, deliverable) {
                    Undelivered::Decline(refusal) => {
                        decline_refused(app, db, &row, &refusal).await?;
                        notify();
                    }
                    Undelivered::Complete => inbox::complete(&db.lock().unwrap(), &transition.id)?,
                    Undelivered::Retry => {
                        inbox::retry(&db.lock().unwrap(), &transition.id, now_ms())?
                    }
                }
            }
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl EventSink for GeoLocationSink {
    async fn start(&self, app: &AppHandle, db: DbConnection) -> Result<()> {
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        anyhow::bail!("Native geofences require iOS or macOS");
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        {
            inbox::initialize(&db.lock().unwrap())?;
            let token = {
                let mut worker = WORKER.lock().unwrap();
                if worker.is_some() {
                    return Ok(());
                }
                let token = CancellationToken::new();
                *worker = Some(token.clone());
                token
            };
            native::setup();
            let handle = app.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        result = process(&handle,&db) => {
                            if let Err(error) = result { tracing::warn!(error=%error,"Geofence registration or inbox reconciliation failed"); }
                        }
                    }
                    tokio::select! {
                        _ = token.cancelled() => break,
                        _ = WAKE.notified() => {},
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    }
                }
            });
            Ok(())
        }
    }

    async fn stop(&self, _app: &AppHandle, db: DbConnection) -> Result<()> {
        if let Some(token) = WORKER.lock().unwrap().take() {
            token.cancel();
        }
        expire();
        if let Ok(scope) = native::scope()
            && !scope.is_empty()
        {
            native::sync(&scope, &[])?;
        }
        inbox::prune(&db.lock().unwrap(), None, now_ms())?;
        Ok(())
    }

    /// A region registers only with the clearance a save resolved before
    /// locking the sink manager, see [`GeoLocationSink::register_cleared`].
    async fn on_register(
        &self,
        app: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        self.register_cleared(app, registration, db, None).await
    }

    async fn on_unregister(
        &self,
        app: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        cancel_event(&registration.event_id);
        let _lock = SYNC.lock().await;
        inbox::initialize(&db.lock().unwrap())?;
        inbox::remove(&db.lock().unwrap(), &registration.event_id)?;
        // The manager removes its registration row after this hook, so the binding is the exclusion authority.
        if let Err(error) = sync_native(app, &db).await {
            tracing::warn!(event_id=%registration.event_id,error=%error,"Could not reconcile native geofences after unregistering");
        }
        notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::oauth::OAuthToken;
    use std::collections::HashMap;

    const SCOPE: &str = "scope-of-user-1";

    fn region(radius: f64) -> GeoLocationSink {
        GeoLocationSink {
            latitude: 52.5,
            longitude: 13.4,
            radius,
            ..GeoLocationSink::default()
        }
    }

    /// A Remote location trigger of `config` signing in with `pat`, verified
    /// for user-1.
    fn trigger(config: &GeoLocationSink, pat: &str) -> EventRegistration {
        let now = std::time::SystemTime::now();
        EventRegistration {
            event_id: "event".to_string(),
            name: "Region".to_string(),
            r#type: "geolocation".to_string(),
            updated_at: now,
            created_at: now,
            config: EventConfig::GeoLocation(config.clone()),
            offline: false,
            app_id: "app".to_string(),
            default_payload: None,
            personal_access_token: Some(pat.to_string()),
            pat_owner_sub: Some("user-1".to_string()),
            hub_origin: Some("https://hub.example".to_string()),
            remote: true,
            oauth_tokens: HashMap::new(),
        }
    }

    fn cleared(
        config: &GeoLocationSink,
        registration: &EventRegistration,
        denial: Option<Denial>,
    ) -> GeofenceClearance {
        GeofenceClearance {
            trigger: Some(ClearedTrigger::of(config, registration, SCOPE)),
            denial,
        }
    }

    fn changed(admitted: Result<()>) -> bool {
        admitted.is_err_and(|error| error.to_string() == CHANGED_WHILE_CLEARED)
    }

    #[test]
    fn a_clearance_admits_only_the_trigger_and_account_it_was_resolved_for() {
        let config = region(200.0);
        let registration = trigger(&config, "pat_1");
        let clearance = cleared(&config, &registration, None);
        assert!(clearance.admit(&config, &registration, SCOPE).is_ok());

        let renamed_with_grants = EventRegistration {
            name: "Renamed".to_string(),
            oauth_tokens: HashMap::from([(
                "github".to_string(),
                OAuthToken::new("access".to_string()),
            )]),
            ..registration.clone()
        };
        assert!(
            clearance
                .admit(&config, &renamed_with_grants, SCOPE)
                .is_ok()
        );

        let other_token = trigger(&config, "pat_2");
        assert!(changed(clearance.admit(&config, &other_token, SCOPE)));
        let unverified = EventRegistration {
            pat_owner_sub: None,
            ..registration.clone()
        };
        assert!(changed(clearance.admit(&config, &unverified, SCOPE)));
        let runs_here = EventRegistration {
            remote: false,
            ..registration.clone()
        };
        assert!(changed(clearance.admit(&config, &runs_here, SCOPE)));
        let wider = region(500.0);
        assert!(changed(clearance.admit(
            &wider,
            &trigger(&wider, "pat_1"),
            SCOPE
        )));
        assert!(changed(clearance.admit(
            &config,
            &registration,
            "scope-of-user-2"
        )));
    }

    #[test]
    fn a_refused_clearance_fails_the_registration_with_the_same_error() {
        let config = region(200.0);
        let registration = trigger(&config, "pat_1");
        let registering = |error: anyhow::Error| {
            cleared(&config, &registration, Some(Denial::of(&error)))
                .admit(&config, &registration, SCOPE)
                .unwrap_err()
        };

        assert!(registering(TokenRefused("revoked".into()).into()).is::<TokenRefused>());
        let paused = registering(dispatch::Rejected("paused".into()).into());
        assert!(paused.is::<dispatch::Rejected>());
        assert_eq!(paused.to_string(), "paused");
        let unreachable = registering(anyhow!("hub unreachable"));
        assert_eq!(unreachable.to_string(), "hub unreachable");
        assert_eq!(dispatch::refusal(&unreachable), None);

        let unscoped = GeofenceClearance {
            trigger: None,
            denial: Some(Denial::of(&anyhow!("no account scope"))),
        };
        assert!(changed(unscoped.admit(&config, &registration, SCOPE)));
    }

    #[test]
    fn only_a_refusal_declines_and_a_crossing_past_its_window_is_not_sent_again() {
        let paused = || anyhow::Error::from(dispatch::Rejected("paused".into()));
        let project_inactive =
            || anyhow!("The hub could not take the geofence crossing yet (409 PROJECT_INACTIVE)");
        let declined = |reason: &str, token_refused| {
            Undelivered::Decline(Refusal {
                reason: reason.into(),
                token_refused,
            })
        };
        let cases = [
            (paused(), true, true, declined("paused", false)),
            (paused(), true, false, declined("paused", false)),
            (paused(), false, true, Undelivered::Complete),
            (
                TokenRefused("revoked".into()).into(),
                true,
                true,
                declined("revoked", true),
            ),
            (project_inactive(), true, true, Undelivered::Retry),
            (
                anyhow!("Geofence app app-1 is inactive"),
                true,
                true,
                Undelivered::Retry,
            ),
            (project_inactive(), true, false, Undelivered::Complete),
            (project_inactive(), false, true, Undelivered::Complete),
        ];
        for (error, registered, deliverable, expected) in cases {
            assert_eq!(
                undelivered(&error, registered, deliverable),
                expected,
                "{error:#} registered={registered} deliverable={deliverable}"
            );
        }
    }
}
