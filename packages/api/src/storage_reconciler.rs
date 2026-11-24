//! Storage accounting where the store sends no events: Docker Compose and Kubernetes, with any
//! storage provider. A pass lists the content store under `apps/` and `users/`, compares it with
//! the ledger and applies what changed through the shared accounting core, so App, User and
//! payer totals and Lance dataset rows behave as on AWS, only up to one interval late.

use std::{future::ready, sync::Arc, time::Duration};

use flow_like_file_accounting::{apply_batch, measurement_sequencer, reconcile};
use flow_like_storage::Path;
use futures::TryStreamExt;
use sea_orm::{ConnectionTrait, Statement};
use tokio::task::JoinHandle;

use crate::{error::ApiError, interval_lease, state::AppState};

const INTERVAL_ENV: &str = "FILE_ACCOUNTING_RECONCILE_INTERVAL_SECS";
const DEFAULT_INTERVAL: Duration = Duration::from_secs(900);
/// Providers without byte-bounded upload grants have no bucket name the API keys objects by,
/// so their rows only need a stable label.
const UNNAMED_BUCKET: &str = "content";
const ACCOUNTED_PREFIXES: [&str; 2] = ["apps", "users"];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub listed: usize,
    pub changed: usize,
    pub failed: usize,
}

pub async fn reconcile_once(state: &AppState) -> Result<ReconcileReport, ApiError> {
    let sequencer = measurement_sequencer(std::time::SystemTime::now());
    let credentials = state.master_credentials().await?;
    let bucket = crate::routes::app::data::upload_policy::content_bucket(&credentials)
        .unwrap_or(UNNAMED_BUCKET)
        .to_owned();
    let store = credentials.to_store(false).await?.as_generic();
    let mut listed = Vec::new();
    for prefix in ACCOUNTED_PREFIXES {
        let mut objects = store.list(Some(&Path::from(prefix)));
        while let Some(meta) = objects.try_next().await.map_err(|error| {
            ApiError::internal(format!(
                "Listing {prefix}/ for storage accounting failed: {error}"
            ))
        })? {
            listed.push((
                meta.location.as_ref().to_owned(),
                i64::try_from(meta.size).unwrap_or(i64::MAX),
            ));
        }
    }
    let rows = state
        .db
        .query_all_raw(Statement::from_sql_and_values(
            state.db.get_database_backend(),
            r#"SELECT "objectKey", "size" FROM "FileAccountingObject" WHERE "bucket" = $1"#,
            [bucket.as_str().into()],
        ))
        .await?;
    let mut accounted = Vec::with_capacity(rows.len());
    for row in rows {
        accounted.push((
            row.try_get::<String>("", "objectKey")?,
            row.try_get::<i64>("", "size")?,
        ));
    }

    let mut report = ReconcileReport {
        listed: listed.len(),
        ..Default::default()
    };
    let mut pass = reconcile::diff(&bucket, listed, accounted, &sequencer);
    let observations = std::mem::take(&mut pass.observations);
    report.changed = observations.len();
    if observations.is_empty() {
        return Ok(report);
    }
    let sizes = Arc::new(pass.sizes);
    let results = apply_batch(
        &state.db,
        state.db_dialect,
        &observations,
        |_| ready(Ok(0)),
        |_| ready(Ok(Vec::new())),
        move |observation| ready(Ok(sizes.get(&observation.key).copied().unwrap_or(0))),
    )
    .await;
    for (observation, result) in observations.iter().zip(results) {
        if let Err(error) = result {
            report.failed += 1;
            tracing::warn!(key = %observation.key, %error, "Storage accounting of a changed object failed");
        }
    }
    Ok(report)
}

pub fn spawn_storage_reconciler(state: AppState) -> Option<JoinHandle<()>> {
    let Some(interval) = interval_lease::interval_from_env(INTERVAL_ENV, DEFAULT_INTERVAL) else {
        tracing::info!("Storage reconciler disabled via {INTERVAL_ENV}=0");
        return None;
    };
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if !interval_lease::claim(&state, "storage-reconciler", interval).await {
                continue;
            }
            match reconcile_once(&state).await {
                Ok(report) if report.changed > 0 => tracing::info!(
                    listed = report.listed,
                    changed = report.changed,
                    failed = report.failed,
                    "Storage reconciliation applied changes"
                ),
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "Storage reconciliation failed"),
            }
        }
    }))
}
