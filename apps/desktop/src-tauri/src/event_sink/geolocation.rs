use super::{
    EventConfig, EventRegistration, EventSink,
    manager::{DbConnection, EventSinkManager},
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
        {
            inbox::complete(&db.lock().unwrap(), &transition.id)?;
            continue;
        }
        if !config.background && !native::foreground() {
            continue;
        }
        let remaining = native::remaining();
        if remaining >= 0.0 && remaining <= 1.0 {
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
                            || (remaining >= 0.0 && remaining <= 1.0)
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
                if error.is::<dispatch::Rejected>() && still_registered(db, &scope, &transition) {
                    inbox::remove(&db.lock().unwrap(), &row.event_id)?;
                    notify();
                } else if !still_registered(db, &scope, &transition) {
                    inbox::complete(&db.lock().unwrap(), &transition.id)?;
                } else {
                    inbox::retry(&db.lock().unwrap(), &transition.id, now_ms())?;
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

    async fn on_register(
        &self,
        app: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        self.validate()?;
        let scope = dispatch::current_scope(app).await?;
        dispatch::authorize(app, registration, &scope).await?;
        let _lock = SYNC.lock().await;
        ensure!(
            native::scope()? == scope.raw,
            "Geofence account changed during registration"
        );
        let region = self.registration(&scope.raw, &registration.app_id, &registration.event_id);
        let existing = registrations(&db, &scope)?;
        ensure!(
            existing
                .iter()
                .filter(|r| r.event_id != registration.event_id)
                .count()
                < 20,
            "Apple supports at most 20 monitored geofences per app"
        );
        inbox::bind(
            &db.lock().unwrap(),
            &registration.event_id,
            &scope.raw,
            &region.id,
        )?;
        if let Err(error) = sync_native(app, &db).await {
            inbox::remove(&db.lock().unwrap(), &registration.event_id)?;
            let _ = sync_native(app, &db).await;
            return Err(error);
        }
        notify();
        Ok(())
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
