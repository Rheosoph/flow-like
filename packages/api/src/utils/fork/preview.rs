use std::sync::Arc;

use crate::{
    entity::{event, event_sink},
    error::ApiError,
    state::AppState,
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

/// Walks `apps/{app_id}/...` and app metadata media, then sums total
/// bytes + object count across **both** the meta and content stores.
/// Used by the preview endpoint and by the cross-mode flows for
/// size-cap enforcement — the cap is on the user-visible bundle,
/// which spans both stores plus `media/apps/{app_id}`.
pub async fn compute_app_size_and_count(
    state: &AppState,
    app_id: &str,
) -> Result<(u64, u64), ApiError> {
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
    let (meta_bytes, meta_count) = sum_prefix(&meta_store, &prefix).await?;
    // Same physical bucket aliasing → don't double count. We can't
    // detect aliasing reliably from `Arc::ptr_eq` (different `Arc`
    // instances point at the same backing impl), so when meta and
    // content are aliased the loop hits the same entries twice;
    // accept the duplication as the upper bound (size caps are
    // conservative anyway). For physically separate stores this
    // computes the true total.
    let (content_bytes, content_count) = sum_prefix(&content_store, &prefix).await?;
    let media_prefix = Path::from("media").child("apps").child(app_id.to_string());
    let (media_bytes, media_count) = sum_prefix(&content_store, &media_prefix).await?;
    Ok((
        meta_bytes
            .saturating_add(content_bytes)
            .saturating_add(media_bytes),
        meta_count
            .saturating_add(content_count)
            .saturating_add(media_count),
    ))
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

    let (app_bytes, app_count) = sum_prefix(&content_store, &app_prefix).await?;
    let (media_bytes, media_count) = sum_prefix(&content_store, &media_prefix).await?;
    Ok((
        app_bytes.saturating_add(media_bytes),
        app_count.saturating_add(media_count),
    ))
}

async fn sum_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
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
