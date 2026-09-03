use sea_orm::sea_query::ExprTrait;
use std::sync::Arc;

use crate::{
    entity::{event, event_sink},
    error::ApiError,
    state::AppState,
    utils::fork::{ForkDatabaseMode, ForkPolicy},
};
use flow_like_storage::Path;
use flow_like_types::anyhow;
use futures_util::TryStreamExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a remote-event token would be reused during a fork. Returned
/// from the preview endpoint so the UI can ask the user once for a
/// single token (or warn that OAuth-bound events need re-auth).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub enum RemoteTokenSite {
    /// HTTP/api/webhook event whose `event.config` JSON has an
    /// `auth_token` field set on the source.
    HttpAuthToken { event_id: String },
    /// Sink with an encrypted Personal Access Token (cron, scheduled,
    /// any sink that needs to authenticate to the host on the user's
    /// behalf). Replaceable with a single PAT supplied at fork time.
    Pat { event_id: String, sink_id: String },
    /// Sink with OAuth tokens. Cannot be replaced with a PAT — the
    /// fork must re-authenticate the provider after creation.
    OAuth { event_id: String, sink_id: String },
}

impl RemoteTokenSite {
    /// Whether a single user-supplied token can satisfy this site. PAT
    /// + HTTP auth are token-replaceable; OAuth must re-auth.
    pub fn is_token_replaceable(&self) -> bool {
        matches!(self, Self::HttpAuthToken { .. } | Self::Pat { .. })
    }
}

/// The project LanceDB lives at `apps/{app_id}/storage/db`. Its objects
/// are excluded from the fork's **object count** (never from the byte
/// total): one table fans out into a file per fragment, per index and
/// per commit manifest, so a database that is small on disk still runs
/// into five-digit object counts and would trip
/// `forking.max_file_count` on every fork. The byte cap stays the real
/// resource guard.
pub fn project_db_prefix(app_prefix: &Path) -> Path {
    app_prefix.child("storage").child("db")
}

fn is_under_prefix(location: &Path, prefix: &Path) -> bool {
    let location = location.as_ref();
    let prefix = prefix.as_ref();
    location.len() > prefix.len()
        && location.starts_with(prefix)
        && location.as_bytes()[prefix.len()] == b'/'
}

/// Bytes + object count for one fork category.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ForkCategorySize {
    pub bytes: u64,
    pub objects: u64,
}

impl ForkCategorySize {
    fn add(&mut self, bytes: u64, counted: bool) {
        self.bytes = self.bytes.saturating_add(bytes);
        if counted {
            self.objects = self.objects.saturating_add(1);
        }
    }

    fn merge(&mut self, other: ForkCategorySize) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.objects = self.objects.saturating_add(other.objects);
    }
}

/// Per-category size of a source app, so the fork preview can show what
/// the owner's policy actually costs and the caps can be enforced against
/// the *selected* subset rather than the whole app.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ForkSizeBreakdown {
    /// Always copied regardless of policy: manifest, events, pages,
    /// metadata and app media.
    pub always: ForkCategorySize,
    pub flows: ForkCategorySize,
    pub files: ForkCategorySize,
    pub databases: ForkCategorySize,
    pub widgets: ForkCategorySize,
    pub templates: ForkCategorySize,
}

impl ForkSizeBreakdown {
    /// Whole-app totals — what a fully permissive fork copies.
    pub fn total(&self) -> (u64, u64) {
        let mut sum = self.always;
        sum.merge(self.flows);
        sum.merge(self.files);
        sum.merge(self.databases);
        sum.merge(self.widgets);
        sum.merge(self.templates);
        (sum.bytes, sum.objects)
    }

    /// Totals after applying `policy` — what the fork will really copy.
    /// Schema-only databases contribute effectively nothing: the reserved
    /// artifact tables that ride along are tiny and the user rows don't
    /// travel at all.
    pub fn selected(&self, policy: &ForkPolicy) -> (u64, u64) {
        let mut sum = self.always;
        if policy.flows {
            sum.merge(self.flows);
        }
        if policy.files {
            sum.merge(self.files);
        }
        if policy.databases == ForkDatabaseMode::WithData {
            sum.merge(self.databases);
        }
        if policy.widgets {
            sum.merge(self.widgets);
        }
        if policy.templates {
            sum.merge(self.templates);
        }
        (sum.bytes, sum.objects)
    }
}

/// Walks `apps/{app_id}/...` and app metadata media, bucketing every
/// object into the fork category that owns it. One pass per store — the
/// per-category split is free relative to the listing itself.
pub async fn compute_fork_size_breakdown(
    state: &AppState,
    app_id: &str,
) -> Result<ForkSizeBreakdown, ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let meta_store = credentials
        .to_store(true)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();
    let content_store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    let prefix = Path::from("apps").child(app_id.to_string());
    let mut breakdown = ForkSizeBreakdown::default();

    bucket_prefix(&meta_store, &prefix, &mut breakdown, classify_meta).await?;
    // Same physical bucket aliasing → don't double count. We can't
    // detect aliasing reliably from `Arc::ptr_eq` (different `Arc`
    // instances point at the same backing impl), so when meta and
    // content are aliased the loop hits the same entries twice;
    // accept the duplication as the upper bound (size caps are
    // conservative anyway). For physically separate stores this
    // computes the true total.
    bucket_prefix(&content_store, &prefix, &mut breakdown, classify_content).await?;

    let media_prefix = Path::from("media").child("apps").child(app_id.to_string());
    bucket_prefix(&content_store, &media_prefix, &mut breakdown, |_| {
        (ForkCategory::Always, true)
    })
    .await?;

    Ok(breakdown)
}

/// Enforces the deployment's fork caps against what the owner's policy
/// will actually copy. Sharing this between the online and offline entry
/// points keeps them from drifting apart from each other — and from the
/// preview endpoint, which decides whether the fork button is enabled.
pub async fn ensure_fork_within_limits(
    state: &AppState,
    app_id: &str,
    policy: &ForkPolicy,
) -> Result<(), ApiError> {
    let breakdown = compute_fork_size_breakdown(state, app_id).await?;
    let (selected_size, selected_count) = breakdown.selected(policy);
    let max_size = state.platform_config.forking.max_size_bytes;
    let max_count = state.platform_config.forking.max_file_count;
    if selected_size > max_size {
        return Err(ApiError::bad_request(format!(
            "source app exceeds the deployment's fork size cap ({selected_size} bytes > {max_size} bytes)"
        )));
    }
    if selected_count > max_count {
        return Err(ApiError::bad_request(format!(
            "source app exceeds the deployment's fork file-count cap ({selected_count} > {max_count})"
        )));
    }
    Ok(())
}

/// Walks `apps/{app_id}/...` and app metadata media, then sums total
/// bytes + object count across **both** the meta and content stores.
/// Used by the preview endpoint and by the cross-mode flows for
/// size-cap enforcement — the cap is on the user-visible bundle,
/// which spans both stores plus `media/apps/{app_id}`.
pub async fn compute_app_size_and_count(
    state: &AppState,
    app_id: &str,
) -> Result<(u64, u64), ApiError> {
    Ok(compute_fork_size_breakdown(state, app_id).await?.total())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForkCategory {
    Always,
    Flows,
    Files,
    Databases,
    Widgets,
    Templates,
}

/// Meta store layout: boards and their version archives are flows;
/// `.widget` / `.template` files are their own categories; the manifest,
/// events and pages always travel.
fn classify_meta(relative: &str) -> (ForkCategory, bool) {
    if relative.starts_with("versions/") || relative.ends_with(".board") {
        return (ForkCategory::Flows, true);
    }
    if relative.ends_with(".widget") {
        return (ForkCategory::Widgets, true);
    }
    if relative.ends_with(".template") {
        return (ForkCategory::Templates, true);
    }
    (ForkCategory::Always, true)
}

/// Content store layout: `upload/` is user files, `storage/db/` is the
/// project database, everything else (metadata, flow scratch) always
/// travels. Database objects contribute bytes but are never counted —
/// see [`project_db_prefix`].
fn classify_content(relative: &str) -> (ForkCategory, bool) {
    if relative.starts_with("upload/") {
        return (ForkCategory::Files, true);
    }
    if relative.starts_with("storage/db/") {
        return (ForkCategory::Databases, false);
    }
    (ForkCategory::Always, true)
}

async fn bucket_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
    breakdown: &mut ForkSizeBreakdown,
    classify: impl Fn(&str) -> (ForkCategory, bool),
) -> Result<(), ApiError> {
    let mut listing = store.list(Some(prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list app prefix: {e}")))?
    {
        let location = item.location.as_ref();
        let relative = match location.strip_prefix(prefix.as_ref()) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => rest.trim_start_matches('/'),
            _ => continue,
        };
        let (category, counted) = classify(relative);
        let bucket = match category {
            ForkCategory::Always => &mut breakdown.always,
            ForkCategory::Flows => &mut breakdown.flows,
            ForkCategory::Files => &mut breakdown.files,
            ForkCategory::Databases => &mut breakdown.databases,
            ForkCategory::Widgets => &mut breakdown.widgets,
            ForkCategory::Templates => &mut breakdown.templates,
        };
        bucket.add(item.size, counted);
    }
    Ok(())
}

/// Same as [`compute_app_size_and_count`] but only for the content
/// store. Used by `finalize_online` — the desktop uploads content to
/// the content store, and metadata media is materialized under
/// `media/apps/{app_id}` before this check; the meta store is populated
/// server-side via the normal app-edit endpoints.
pub async fn compute_app_content_size_and_count(
    state: &AppState,
    app_id: &str,
) -> Result<(u64, u64), ApiError> {
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let content_store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();

    let app_prefix = Path::from("apps").child(app_id.to_string());
    let media_prefix = Path::from("media").child("apps").child(app_id.to_string());
    let db_prefix = project_db_prefix(&app_prefix);

    let (app_bytes, app_count) = sum_prefix(&content_store, &app_prefix, Some(&db_prefix)).await?;
    let (media_bytes, media_count) = sum_prefix(&content_store, &media_prefix, None).await?;
    Ok((
        app_bytes.saturating_add(media_bytes),
        app_count.saturating_add(media_count),
    ))
}

/// Sums bytes + object count below `prefix`. Objects under
/// `uncounted_prefix` still contribute their bytes but are left out of
/// the object count — see [`project_db_prefix`].
async fn sum_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
    uncounted_prefix: Option<&Path>,
) -> Result<(u64, u64), ApiError> {
    let mut total_bytes: u64 = 0;
    let mut count: u64 = 0;
    let mut listing = store.list(Some(prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list app prefix: {e}")))?
    {
        total_bytes = total_bytes.saturating_add(item.size);
        if uncounted_prefix.is_some_and(|skip| is_under_prefix(&item.location, skip)) {
            continue;
        }
        count = count.saturating_add(1);
    }
    Ok((total_bytes, count))
}

/// Detects every remote-token site on the source app — events with a
/// non-empty `auth_token` in their config, sinks with `pat_encrypted`
/// or `oauth_tokens_encrypted` set. Used by the preview endpoint so the
/// UI can decide whether to prompt the caller for a token.
pub async fn detect_remote_token_sites(
    state: &AppState,
    app_id: &str,
) -> Result<Vec<RemoteTokenSite>, ApiError> {
    let mut sites = Vec::new();

    let event_rows = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(
            event::Column::EventType
                .eq("api")
                .or(event::Column::EventType.eq("http"))
                .or(event::Column::EventType.eq("webhook")),
        )
        .all(&state.db)
        .await?;
    for row in event_rows {
        if event_config_has_auth_token(row.config.as_ref()) {
            sites.push(RemoteTokenSite::HttpAuthToken { event_id: row.id });
        }
    }

    let sink_rows = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;
    for sink in sink_rows {
        if sink.pat_encrypted.is_some() {
            sites.push(RemoteTokenSite::Pat {
                event_id: sink.event_id.clone(),
                sink_id: sink.id.clone(),
            });
        }
        if sink.oauth_tokens_encrypted.is_some() {
            sites.push(RemoteTokenSite::OAuth {
                event_id: sink.event_id.clone(),
                sink_id: sink.id.clone(),
            });
        }
    }

    Ok(sites)
}

/// Inspects the `config: Option<Json>` column on an event row and
/// reports whether it has a non-empty `auth_token` field. The DB stores
/// the event config as either `{ "base64": "..." }` (raw bytes encoded)
/// or directly as the parsed JSON object — either way the auth_token,
/// if present, is at the top level once decoded.
fn event_config_has_auth_token(config: Option<&serde_json::Value>) -> bool {
    let Some(value) = config else { return false };
    match value {
        serde_json::Value::Object(obj) => {
            if let Some(b64) = obj.get("base64").and_then(|v| v.as_str()) {
                use base64::Engine;
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
                    return false;
                };
                let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return false;
                };
                parsed
                    .get("auth_token")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            } else {
                obj.get("auth_token")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            }
        }
        _ => false,
    }
}
