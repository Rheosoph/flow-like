//! The audit worker: the only component that seals, signs, archives and prunes.
//!
//! Requests only insert records. Every minute one worker, holding the lease, runs a
//! tick: seal pending records per chain, commit new seals to a signed epoch, write the
//! daily head, archive closed months, prune archived data past its window, expire
//! personal values, export remaining legacy entries and deliver customer webhooks. Every
//! step is idempotent and bounded, so a crashed or overlapping tick is harmless.

mod archive;
pub mod bucket;
pub mod checkpoint;
mod epoch;
mod head;
pub mod lease;
mod legacy;
mod prune;
mod seal;
mod sweep;
pub mod watermark;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use flow_like::hub::AuditConfig;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::tokio;
use sea_orm::DatabaseConnection;
use serde::Serialize;

use crate::db::DbDialect;

use super::signer::{BudgetedSigner, SharedSigner};
use lease::Lease;

/// Wait after a failed key-service connection before the next attempt; each attempt is
/// a billed request.
const CONNECT_RETRY: Duration = Duration::from_secs(600);

pub struct AuditWorkerContext {
    pub db: DatabaseConnection,
    pub dialect: DbDialect,
    pub config: AuditConfig,
    /// `AUDIT_BUCKET`. Without it nothing is archived and no evidence is pruned.
    pub bucket: Option<Arc<FlowLikeStore>>,
    /// Decrypts customer webhook secrets. Without it no webhook is delivered.
    pub encryption_key: Option<[u8; 32]>,
    signer: SignerSlot,
    /// Long-lived workers keep the lease across ticks, so exactly one process signs.
    sticky: bool,
    lease: tokio::sync::Mutex<Option<Lease>>,
    details_backfill: tokio::sync::Mutex<sweep::DetailsBackfill>,
    checkpoints: bool,
    checkpoint: tokio::sync::Mutex<checkpoint::CheckpointState>,
}

/// The audit key. Without it records are still sealed, but no epoch is written and
/// nothing is pruned.
enum SignerSlot {
    Ready(Option<SharedSigner>),
    /// `AUDIT_KMS_KEY_ID`, connected by the first tick that wins the lease, so replicas
    /// that never work never contact the key service.
    KeyService {
        kid: Option<String>,
        signer: tokio::sync::OnceCell<SharedSigner>,
        retry_at: Mutex<Option<Instant>>,
    },
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TickReport {
    /// Another worker held the lease.
    pub skipped: bool,
    pub seals: u64,
    pub sealed_records: u64,
    /// Pending records whose MAC failed; marked and never sealed.
    pub quarantined: u64,
    /// Due chains with a pending record whose MAC matched no entry key this worker
    /// holds. Nothing of theirs was sealed or quarantined; the next tick retries.
    pub held_chains: u64,
    pub oldest_pending_seconds: Option<i64>,
    pub epochs: u64,
    pub anchored_seals: u64,
    pub head_written: bool,
    pub archived_parts: u64,
    pub pruned_records: u64,
    pub pruned_seals: u64,
    pub pruned_epochs: u64,
    pub expired_values: u64,
    pub legacy_exported: u64,
    pub exports_delivered: u64,
    /// Steps that failed this tick, by name. The next tick retries them.
    pub failed_steps: Vec<String>,
}

impl AuditWorkerContext {
    /// A context with a ready audit key that releases the lease after every tick, for
    /// tests and one-off tools.
    pub fn new(
        db: DatabaseConnection,
        dialect: DbDialect,
        config: AuditConfig,
        signer: Option<SharedSigner>,
        bucket: Option<Arc<FlowLikeStore>>,
        encryption_key: Option<[u8; 32]>,
    ) -> Self {
        Self {
            db,
            dialect,
            config,
            bucket,
            encryption_key,
            signer: SignerSlot::Ready(signer),
            sticky: false,
            lease: tokio::sync::Mutex::new(None),
            details_backfill: tokio::sync::Mutex::new(Default::default()),
            checkpoints: false,
            checkpoint: tokio::sync::Mutex::new(Default::default()),
        }
    }

    /// Legacy local/AWS entry points share their database with State. API State itself
    /// never loads a private audit key; only constructing a worker does.
    pub fn from_state(state: &crate::state::State) -> flow_like_types::Result<Self> {
        Self::from_env(
            state.db.clone(),
            state.db_dialect,
            state.platform_config.audit.clone(),
            Some(state.encryption_key),
            true,
        )
    }

    /// Build a worker without API State, backend JWT keys, application storage, or
    /// request handlers. Scheduled jobs release the lease after every tick.
    pub fn from_env(
        db: DatabaseConnection,
        dialect: DbDialect,
        config: AuditConfig,
        encryption_key: Option<[u8; 32]>,
        sticky: bool,
    ) -> flow_like_types::Result<Self> {
        use crate::storage_config::{non_empty_env, secret_env};
        let bucket = bucket::from_env()?;
        let key_service = super::kms::configured()?;
        let kid = non_empty_env("AUDIT_KID");
        if let Some(keys) = secret_env("AUDIT_VERIFYING_KEYS")? {
            super::signer::register_verifying_keys_json(&keys)?;
        }
        let local = secret_env("AUDIT_SIGNING_KEY")?;
        let signer = match (local, key_service) {
            (Some(_), true) => {
                return Err(flow_like_types::anyhow!(
                    "both AUDIT_SIGNING_KEY and AUDIT_KMS_KEY_ID are set; set exactly one audit key"
                ));
            }
            (Some(key), false) => {
                let local: SharedSigner =
                    Arc::new(super::signer::LocalSigner::from_pem_b64(&key, kid)?);
                super::signer::register_verifying_key(local.kid(), local.verifying_key())?;
                tracing::info!(
                    kid = local.kid(),
                    public_key = %super::signer::public_key_pem(&local.verifying_key()),
                    "audit signing key loaded"
                );
                SignerSlot::Ready(Some(BudgetedSigner::hourly(local)))
            }
            (None, true) => SignerSlot::KeyService {
                kid,
                signer: tokio::sync::OnceCell::new(),
                retry_at: Mutex::new(None),
            },
            (None, false) => SignerSlot::Ready(None),
        };
        if config.enabled && config.require_signing && matches!(signer, SignerSlot::Ready(None)) {
            return Err(flow_like_types::anyhow!(
                "AUDIT SIGNING REQUIRED: the audit worker needs AUDIT_SIGNING_KEY or AUDIT_KMS_KEY_ID"
            ));
        }
        Ok(Self {
            db,
            dialect,
            config,
            bucket,
            encryption_key,
            signer,
            sticky,
            lease: tokio::sync::Mutex::new(None),
            details_backfill: tokio::sync::Mutex::new(Default::default()),
            checkpoints: true,
            checkpoint: tokio::sync::Mutex::new(Default::default()),
        })
    }

    /// The audit key once available: the local key, or the key-service key after a tick
    /// connected it.
    pub fn signer(&self) -> Option<&SharedSigner> {
        match &self.signer {
            SignerSlot::Ready(signer) => signer.as_ref(),
            SignerSlot::KeyService { signer, .. } => signer.get(),
        }
    }

    /// Whether an audit key is configured, connected or not.
    pub fn signing_configured(&self) -> bool {
        !matches!(self.signer, SignerSlot::Ready(None))
    }

    /// Enable durable rollback checks for a context built by a test or tool.
    pub fn with_checkpoints(mut self) -> Self {
        self.checkpoints = true;
        self
    }

    /// Release a continuous worker's lease after its last completed tick.
    pub async fn release_lease(&self) {
        if let Some(lease) = self.lease.lock().await.take() {
            lease.release().await;
        }
    }

    /// Connect the key-service key once, retrying a failure only after
    /// [`CONNECT_RETRY`].
    async fn connect_signer(&self) {
        let SignerSlot::KeyService {
            kid,
            signer,
            retry_at,
        } = &self.signer
        else {
            return;
        };
        if signer.initialized() {
            return;
        }
        let waiting = retry_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|at| Instant::now() < at);
        if waiting {
            return;
        }
        match super::kms::from_env(kid.clone()).await {
            Ok(Some(connected)) => {
                if let Err(error) = super::signer::register_verifying_key(
                    connected.kid(),
                    connected.verifying_key(),
                ) {
                    tracing::error!(%error, "audit key-service key conflicts with AUDIT_VERIFYING_KEYS");
                    *retry_at.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(Instant::now() + CONNECT_RETRY);
                    return;
                }
                tracing::info!(
                    kid = connected.kid(),
                    public_key = %super::signer::public_key_pem(&connected.verifying_key()),
                    "audit key-service key connected; processes without it verify through AUDIT_VERIFYING_KEYS"
                );
                let _ = signer.set(BudgetedSigner::hourly(connected));
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "connecting the audit key-service key failed; retrying in 10 minutes");
                *retry_at.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(Instant::now() + CONNECT_RETRY);
            }
        }
    }
}

macro_rules! step {
    ($report:ident, $name:literal, $lease:ident, $call:expr) => {
        if !$lease.keepalive().await {
            tracing::error!(step = $name, "audit worker lost its lease; stopping this tick");
            $report.failed_steps.push(format!("{}: lease lost", $name));
            return Ok($report);
        }
        if let Err(error) = $call.await {
            tracing::error!(step = $name, %error, "audit worker step failed");
            $report.failed_steps.push($name.to_owned());
        }
    };
}

/// One pass of every step. Returns early with `skipped` when another worker is active.
/// A context from [`AuditWorkerContext::from_state`] keeps the lease between ticks.
pub async fn tick(
    context: &AuditWorkerContext,
    now: DateTime<Utc>,
) -> flow_like_types::Result<TickReport> {
    if !context.config.enabled {
        return Ok(TickReport::default());
    }
    let mut held = context.lease.lock().await;
    let kept = match held.as_ref() {
        Some(lease) => lease.renew().await,
        None => false,
    };
    if !kept {
        *held = Lease::acquire(&context.db).await?;
        if context.checkpoints {
            context.checkpoint.lock().await.invalidate();
        }
    }
    let Some(lease) = held.as_ref() else {
        return Ok(TickReport {
            skipped: true,
            ..Default::default()
        });
    };
    context.connect_signer().await;
    let report = if context.config.require_signing && context.signer().is_none() {
        Err(flow_like_types::anyhow!(
            "required audit signer is unavailable"
        ))
    } else {
        run_steps(context, lease, now).await
    };
    if !context.sticky
        && let Some(lease) = held.take()
    {
        lease.release().await;
    }
    let report = report?;
    if !report.failed_steps.is_empty() {
        tracing::warn!(failed = ?report.failed_steps, "audit worker tick finished with failures");
    }
    Ok(report)
}

async fn run_steps(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
) -> flow_like_types::Result<TickReport> {
    let mut report = TickReport::default();
    if context.checkpoints {
        // Stop before signing or pruning if an independently stored checkpoint
        // no longer agrees with the database, including after process restarts.
        checkpoint::check_previous(context, lease).await?;
    }
    step!(
        report,
        "seal",
        lease,
        seal::run(context, lease, now, &mut report)
    );
    step!(
        report,
        "epoch",
        lease,
        epoch::run(context, lease, now, &mut report)
    );
    if context.bucket.is_some() {
        if context.checkpoints {
            // A failed checkpoint must stop archival/pruning rather than silently
            // advancing the database without an external witness.
            checkpoint::write_daily(context, lease, now).await?;
        }
        step!(report, "head", lease, head::run(context, now, &mut report));
        step!(
            report,
            "archive",
            lease,
            archive::run(context, lease, now, &mut report)
        );
        step!(
            report,
            "legacy",
            lease,
            legacy::run(context, lease, now, &mut report)
        );
    }
    // Evidence is pruned only once its month is archived; activity has no archive.
    step!(
        report,
        "prune",
        lease,
        prune::run(context, lease, now, &mut report)
    );
    step!(
        report,
        "sweep",
        lease,
        sweep::run(context, lease, now, &mut report)
    );
    if context.encryption_key.is_some() {
        step!(
            report,
            "export",
            lease,
            super::export::deliver(context, lease, now, &mut report)
        );
    }
    Ok(report)
}

/// Run the worker in this process every `interval` until the task is aborted. For
/// long-lived servers; serverless deployments call [`tick`] from a scheduled trigger.
pub fn spawn(context: Arc<AuditWorkerContext>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tracing::info!(
        interval_secs = interval.as_secs(),
        signing = context.signing_configured(),
        archiving = context.bucket.is_some(),
        "audit worker started"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = tick(&context, Utc::now()).await {
                tracing::error!(%error, "audit worker tick failed");
            }
        }
    })
}

/// Default interval between ticks.
pub const TICK_INTERVAL: Duration = Duration::from_secs(60);
