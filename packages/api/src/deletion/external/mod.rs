//! Cleanup outside the relational database.
//!
//! Every step is keyed by the root id, re-runnable, and never executes inside
//! a database transaction. Steps that need child rows to find their targets
//! run before those rows drain (declared in [`super::overrides`]).

pub mod app;
pub mod bit;
pub mod course;
pub mod wasm_package;

use std::sync::Arc;

use flow_like_storage::object_store::{ObjectStore, path::Path};
use flow_like_types::anyhow;
use futures_util::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::state::AppState;

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
}

impl ExternalStep {
    pub fn describe(self) -> &'static str {
        match self {
            Self::AppSinkSchedules => "delete the app's sink cron schedules",
            Self::AppStoragePrefixes => "delete the app's storage prefixes",
            Self::AppCacheBackend => "delete the app's cache entries on the cache backend",
            Self::WasmPackageArtifacts => "delete the package's stored artifacts",
            Self::CourseMedia => "delete the course's media prefix",
            Self::BitCdnArtifact => "delete the bit's CDN object",
        }
    }
}

pub async fn run(state: &AppState, step: ExternalStep, root_id: &str) -> Result<(), ApiError> {
    match step {
        ExternalStep::AppSinkSchedules => app::delete_sink_schedules(state, root_id).await,
        ExternalStep::AppStoragePrefixes => app::delete_storage_prefixes(state, root_id).await,
        ExternalStep::AppCacheBackend => app::delete_cache_backend(state, root_id).await,
        ExternalStep::WasmPackageArtifacts => wasm_package::delete_artifacts(state, root_id).await,
        ExternalStep::CourseMedia => course::delete_media(state, root_id).await,
        ExternalStep::BitCdnArtifact => bit::delete_cdn_artifact(state, root_id).await,
    }
}

/// Delete every object under `prefix`, streaming the listing into the
/// delete so neither side is held in memory at once.
pub(super) async fn delete_prefix(
    store: &Arc<dyn ObjectStore>,
    prefix: &Path,
    label: &str,
) -> Result<u64, ApiError> {
    let locations = store
        .list(Some(prefix))
        .map_ok(|meta| meta.location)
        .boxed();
    let deleted = store
        .delete_stream(locations)
        .try_fold(0u64, |count, _| async move { Ok(count + 1) })
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("delete {label} prefix {prefix}: {error}"))
        })?;
    if deleted > 0 {
        tracing::info!(deleted, %prefix, label, "Deleted storage prefix");
    }
    Ok(deleted)
}
