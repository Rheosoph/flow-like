//! Cleanup outside the relational database.
//!
//! Every step is keyed by the root id, re-runnable, and never executes inside
//! a database transaction. Steps that need child rows to find their targets
//! run before those rows drain (declared in [`super::overrides`]).

pub mod app;
pub mod bit;
pub mod course;
pub mod template;
pub mod wasm_package;

use std::sync::Arc;

use flow_like_storage::object_store::{ObjectStore, path::Path};
use flow_like_types::anyhow;
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::drain::{Flow, Pass};
use crate::error::ApiError;
use crate::state::AppState;

/// Objects deleted between two lease renewals. Small enough that a prefix of
/// any size keeps the job's lease alive and can hand the budget back, large
/// enough that the extra job write is noise next to the deletes.
const OBJECTS_PER_TICK: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalStep {
    /// Cron schedules of the app's sinks, looked up from `EventSink` rows.
    AppSinkSchedules,
    /// `apps/{id}` on the meta and content stores plus `media/apps/{id}`.
    AppStoragePrefixes,
    /// Entries on a non-relational cache backend.
    AppCacheBackend,
    /// Package artifacts, bundles, assets and compiled binaries.
    WasmPackageArtifacts,
    /// `media/courses/{id}` on the content store.
    CourseMedia,
    /// The bit's CDN object, looked up from the `Bit` row.
    BitCdnArtifact,
    /// The template's board file, version archive and page payloads under the
    /// owning app's prefix.
    TemplateStorage,
    /// Staged `ExecutionEvent` payloads of the app's runs, looked up from the
    /// `payloadRef` those rows carry.
    ExecutionEventPayloads,
}

impl ExternalStep {
    pub const ALL: [Self; 8] = [
        Self::AppSinkSchedules,
        Self::AppStoragePrefixes,
        Self::AppCacheBackend,
        Self::WasmPackageArtifacts,
        Self::CourseMedia,
        Self::BitCdnArtifact,
        Self::TemplateStorage,
        Self::ExecutionEventPayloads,
    ];

    pub fn describe(self) -> &'static str {
        match self {
            Self::AppSinkSchedules => "delete the app's sink cron schedules",
            Self::AppStoragePrefixes => "delete the app's storage prefixes",
            Self::AppCacheBackend => "delete the app's cache entries on the cache backend",
            Self::WasmPackageArtifacts => "delete the package's stored artifacts",
            Self::CourseMedia => "delete the course's media prefix",
            Self::BitCdnArtifact => "delete the bit's CDN object",
            Self::TemplateStorage => "delete the template's board, versions and page payloads",
            Self::ExecutionEventPayloads => {
                "delete the staged payload objects of the app's execution events"
            }
        }
    }
}

/// Run one external step against the pass that owns the job.
///
/// Steps that walk an object store are unbounded in wall-clock time, so they
/// take the pass: it renews the lease as they go and stops them once the pass
/// budget is spent. Every step is re-runnable, so a suspended step simply
/// starts over on the next pass against the shorter remainder.
pub async fn run(
    state: &AppState,
    step: ExternalStep,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    let root_id = pass.root_id.clone();
    match step {
        ExternalStep::AppSinkSchedules => {
            app::delete_sink_schedules(state, &root_id).await?;
            Ok(Flow::Continue)
        }
        ExternalStep::AppCacheBackend => {
            app::delete_cache_backend(state, &root_id).await?;
            Ok(Flow::Continue)
        }
        ExternalStep::BitCdnArtifact => {
            bit::delete_cdn_artifact(state, &root_id).await?;
            Ok(Flow::Continue)
        }
        ExternalStep::AppStoragePrefixes => {
            app::delete_storage_prefixes(state, &root_id, pass).await
        }
        ExternalStep::WasmPackageArtifacts => {
            wasm_package::delete_artifacts(state, &root_id, pass).await
        }
        ExternalStep::CourseMedia => course::delete_media(state, &root_id, pass).await,
        ExternalStep::TemplateStorage => template::delete_storage(state, &root_id, pass).await,
        ExternalStep::ExecutionEventPayloads => {
            app::delete_execution_event_payloads(state, &root_id, pass).await
        }
    }
}

/// Delete every object under `prefix`, listing and deleting one bounded page
/// at a time so neither side is held in memory at once and the pass can renew
/// its lease — or give the job back — between pages.
pub(super) async fn delete_prefix(
    store: &Arc<dyn ObjectStore>,
    prefix: &Path,
    label: &str,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    let fail = |error: flow_like_storage::object_store::Error| {
        ApiError::internal_error(anyhow!("delete {label} prefix {prefix}: {error}"))
    };
    let mut pages = store.list(Some(prefix)).chunks(OBJECTS_PER_TICK);
    let mut deleted = 0u64;
    let mut flow = Flow::Continue;
    while let Some(page) = pages.next().await {
        let locations: Vec<Path> = page
            .into_iter()
            .map(|meta| meta.map(|meta| meta.location))
            .collect::<Result<_, _>>()
            .map_err(fail)?;
        if locations.is_empty() {
            continue;
        }
        let count = locations.len() as u64;
        store
            .delete_stream(stream::iter(locations.into_iter().map(Ok)).boxed())
            .try_for_each(|_| async { Ok(()) })
            .await
            .map_err(fail)?;
        deleted += count;
        if pass.after_chunk(count).await? == Flow::Suspend {
            flow = Flow::Suspend;
            break;
        }
    }
    if deleted > 0 {
        tracing::info!(deleted, %prefix, label, "Deleted storage prefix");
    }
    Ok(flow)
}
