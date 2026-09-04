//! Canary assignment support tooling.
//!
//! Recomputes the deterministic live-variant assignment for any split key so
//! an operator can answer "which variant served this occurrence" (or would
//! serve it) without triggering anything — plus the variant lifecycle writes
//! (share patch, list replace, promote, abort) and the per-variant setup
//! health read.

use crate::{
    audit_branch, ensure_permission,
    error::ApiError,
    execution::variant::{self, SplitKey},
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::event::{EventVariant, EventVariantMode};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::db::get_event_from_db;

fn variant_mode_label(mode: &EventVariantMode) -> &'static str {
    match mode {
        EventVariantMode::Live { .. } => "live",
        EventVariantMode::Shadow { .. } => "shadow",
    }
}

fn variant_audit_entry(variant: &EventVariant) -> serde_json::Value {
    let share_field = match variant.mode {
        EventVariantMode::Live { .. } => "weight",
        EventVariantMode::Shadow { .. } => "sample_rate",
    };
    serde_json::json!({
        "name": variant.name,
        "mode": variant_mode_label(&variant.mode),
        "board_id": variant.board_id,
        "board_version": variant.board_version.map(super::dotted_version_key),
        "default_page_id": variant.default_page_id,
        share_field: variant.mode.share(),
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CanaryExplainQuery {
    /// The split-key value to explain: an Idempotency-Key, trace id, caller
    /// subject or run id — or a variant name when `source` is `pin`
    pub key: String,
    /// The key's channel; `pin` treats `key` as a variant name, every other
    /// channel hashes identically
    pub source: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CanaryExplainResponse {
    /// The live variant serving this key; `null` when the primary serves it
    pub variant_name: Option<String>,
    /// The `[lo, hi)` slice of the unit interval owned by this key's target
    pub share_bounds: [f64; 2],
}

/// GET /apps/{app_id}/events/{event_id}/canary/explain
///
/// Recompute which live variant a split key resolves to. Assignments are a
/// pure hash of the event identity and the key, so any past or hypothetical
/// assignment can be recomputed on any replica.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/canary/explain",
    tag = "events",
    description = "Explain which canary variant serves a given split key.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("key" = String, Query, description = "Split-key value to explain (idempotency key, trace id, user id, run id, or variant name for source=pin)"),
        ("source" = Option<String>, Query, description = "Key channel: pin, idempotency-key, trace, subject or run-id")
    ),
    responses(
        (status = 200, description = "The variant serving this key and its share bounds", body = CanaryExplainResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/canary/explain",
    skip(state, user, query)
)]
pub async fn explain_canary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<CanaryExplainQuery>,
) -> Result<Json<CanaryExplainResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    let key = SplitKey {
        source: variant::static_split_source(query.source.as_deref()),
        value: query.key,
    };
    let assignment = variant::explain(&event, &key);

    Ok(Json(CanaryExplainResponse {
        variant_name: assignment.variant_name,
        share_bounds: [assignment.share_bounds.0, assignment.share_bounds.1],
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CanaryStatsQuery {
    /// Aggregation window: `24h` (default) or `7d`
    pub window: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventVariantStats {
    /// The `EventVariant.name` that served these runs; `null` for the primary
    pub variant_name: Option<String>,
    pub requests: u64,
    pub errors: u64,
    /// Microseconds, like every other run-duration surface
    pub p50_duration_us: u64,
    pub p95_duration_us: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventVariantStatsResponse {
    /// The window the aggregates cover, echoed back
    pub window: String,
    pub variants: Vec<EventVariantStats>,
}

/// Live-traffic rows only: shadow and regression runs never enter the
/// promote-or-abort comparison.
const STATS_ROW_CAP: u64 = 50_000;

fn nearest_rank_us(sorted: &[i64], quantile: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((quantile * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].max(0) as u64
}

/// GET /apps/{app_id}/events/{event_id}/canary/stats
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/canary/stats",
    tag = "events",
    description = "Per-variant request, error and latency aggregates for a rolling window.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("window" = Option<String>, Query, description = "Aggregation window: 24h (default) or 7d")
    ),
    responses(
        (status = 200, description = "Per-variant aggregates", body = EventVariantStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/canary/stats",
    skip(state, user, query)
)]
pub async fn canary_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<CanaryStatsQuery>,
) -> Result<Json<EventVariantStatsResponse>, ApiError> {
    use crate::entity::execution_run;
    use crate::entity::sea_orm_active_enums::{RunStatus, RunVariant};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

    ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let window = match query.window.as_deref() {
        None | Some("24h") => "24h",
        Some("7d") => "7d",
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown stats window '{other}'; expected 24h or 7d"
            )));
        }
    };
    let since = chrono::Utc::now().naive_utc()
        - match window {
            "7d" => chrono::Duration::days(7),
            _ => chrono::Duration::hours(24),
        };

    let rows: Vec<(
        RunVariant,
        Option<String>,
        RunStatus,
        Option<chrono::NaiveDateTime>,
        Option<chrono::NaiveDateTime>,
    )> = execution_run::Entity::find()
        .select_only()
        .column_as(execution_run::Column::RunVariant, "run_variant")
        .column_as(execution_run::Column::VariantName, "variant_name")
        .column_as(execution_run::Column::Status, "status")
        .column_as(execution_run::Column::StartedAt, "started_at")
        .column_as(execution_run::Column::CompletedAt, "completed_at")
        .filter(execution_run::Column::AppId.eq(&app_id))
        .filter(execution_run::Column::EventId.eq(&event_id))
        .filter(execution_run::Column::CreatedAt.gte(since))
        .filter(execution_run::Column::RunVariant.is_in([RunVariant::Primary, RunVariant::Canary]))
        .order_by_desc(execution_run::Column::CreatedAt)
        .limit(STATS_ROW_CAP)
        .into_tuple()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let mut grouped: std::collections::BTreeMap<Option<String>, (u64, u64, Vec<i64>)> =
        std::collections::BTreeMap::new();
    for (run_variant, variant_name, status, started_at, completed_at) in rows {
        let key = match run_variant {
            RunVariant::Canary => variant_name,
            _ => None,
        };
        let entry = grouped.entry(key).or_default();
        entry.0 += 1;
        if matches!(
            status,
            RunStatus::Failed | RunStatus::Cancelled | RunStatus::Timeout
        ) {
            entry.1 += 1;
        }
        if let (Some(start), Some(end)) = (started_at, completed_at) {
            entry.2.push((end - start).num_microseconds().unwrap_or(0));
        }
    }

    let variants = grouped
        .into_iter()
        .map(|(variant_name, (requests, errors, mut durations))| {
            durations.sort_unstable();
            EventVariantStats {
                variant_name,
                requests,
                errors,
                p50_duration_us: nearest_rank_us(&durations, 0.5),
                p95_duration_us: nearest_rank_us(&durations, 0.95),
            }
        })
        .collect();

    Ok(Json(EventVariantStatsResponse {
        window: window.to_string(),
        variants,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CanarySharePatch {
    /// The variant to adjust (`canary` addresses the legacy single canary)
    pub name: String,
    /// Live variants: share of traffic replaced, `[0, 1]`
    pub weight: Option<f32>,
    /// Shadow variants: share of traffic mirrored, `[0, 1]`
    pub sample_rate: Option<f32>,
}

fn validated_share(value: Option<f32>, field: &str) -> Result<f32, ApiError> {
    let value =
        value.ok_or_else(|| ApiError::bad_request(format!("this variant requires `{field}`")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ApiError::bad_request(format!(
            "`{field}` must be within [0, 1], got {value}"
        )));
    }
    Ok(value)
}

/// PATCH /apps/{app_id}/events/{event_id}/canary
///
/// The slider path: adjusts exactly one variant's traffic share, writing both
/// the bucket artifact and the database row. `content_equal` excludes shares,
/// so this never cuts an event version — and it never re-runs REST/MCP setup,
/// so the live registration key is never rewritten.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/events/{event_id}/canary",
    tag = "events",
    description = "Adjust one variant's traffic share without versioning or re-running setup.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = CanarySharePatch,
    responses(
        (status = 200, description = "The updated event", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Unknown variant")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/events/{event_id}/canary",
    skip(state, user, patch)
)]
pub async fn patch_canary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(patch): Json<CanarySharePatch>,
) -> Result<Json<flow_like::flow::event::Event>, ApiError> {
    use flow_like::flow::event::filter_event_secrets;

    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    let db_event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    if db_event.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let mut event = freshest_event(&app, db_event).await;

    let share_change = if let Some(variant) = event
        .variants
        .iter_mut()
        .find(|variant| variant.name == patch.name)
    {
        let previous = variant.mode.share();
        match &mut variant.mode {
            EventVariantMode::Live { weight } => {
                *weight = validated_share(patch.weight, "weight")?;
            }
            EventVariantMode::Shadow { sample_rate } => {
                *sample_rate = validated_share(patch.sample_rate, "sample_rate")?;
            }
        }
        variant.updated_at = std::time::SystemTime::now();
        Some((previous, variant.mode.share()))
    } else if event.variants.is_empty()
        && patch.name == "canary"
        && let Some(canary) = event.canary.as_mut()
    {
        let previous = canary.weight;
        canary.weight = validated_share(patch.weight, "weight")?;
        canary.updated_at = std::time::SystemTime::now();
        Some((previous, canary.weight))
    } else {
        None
    };
    let Some((share_before, share_after)) = share_change else {
        return Err(ApiError::not_found(format!(
            "no variant named '{}' on this event",
            patch.name
        )));
    };

    // Share-only change: content_equal ignores it, so no version is cut.
    let event = app
        .upsert_event(event, None, Some(true))
        .await
        .map_err(|e| ApiError::bad_request(format!("saving the share change failed: {e}")))?;
    app.save()
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    super::db::sync_event_with_sink_tokens(&state.db, &state, &app_id, &event, None, None, None)
        .await?;

    audit_branch!(
        state,
        user,
        app_id,
        "event.canary.share",
        "Event",
        event_id,
        "Canary share changed",
        serde_json::json!({
            "variant": patch.name,
            "from": share_before,
            "to": share_after,
        })
    );

    Ok(Json(filter_event_secrets(event)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutVariantsBody {
    #[schema(value_type = Vec<Object>)]
    pub variants: Vec<flow_like::flow::event::EventVariant>,
}

/// PUT /apps/{app_id}/events/{event_id}/variants
///
/// Full-list replace. Target changes are content changes (a version is cut);
/// setup is NOT run here — a REST/MCP variant serves no inbound traffic until
/// per-variant setup lands.
#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/variants",
    tag = "events",
    description = "Replace the event's canary/shadow variant list.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = PutVariantsBody,
    responses(
        (status = 200, description = "The updated event", body = String, content_type = "application/json"),
        (status = 400, description = "Validation failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/events/{event_id}/variants",
    skip(state, user, body)
)]
pub async fn put_event_variants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<PutVariantsBody>,
) -> Result<Json<flow_like::flow::event::Event>, ApiError> {
    use flow_like::flow::event::{filter_event_secrets, preserve_event_secrets};

    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    let db_event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    if db_event.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let stored = freshest_event(&app, db_event).await;

    let mut updated = stored.clone();
    updated.variants = body.variants;
    // Clients round-trip blanked secret values inside variant variables.
    preserve_event_secrets(&mut updated, &stored);

    let event = app
        .upsert_event(updated, None, Some(true))
        .await
        .map_err(|e| ApiError::bad_request(format!("variant validation failed: {e}")))?;
    app.save()
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    super::db::sync_event_with_sink_tokens(&state.db, &state, &app_id, &event, None, None, None)
        .await?;

    // A removed or renamed variant's bucket would otherwise keep serving its
    // routes from the orphaned `EventSetup`/registration/auth rows, and a
    // target change makes the bucket describe a board the variant no longer
    // points at — drop those buckets; a retargeted variant serves again after
    // an explicit re-setup.
    let after = event.variant_set();
    let before = stored.variant_set();
    for old in &before {
        let invalidated = match after.iter().find(|new| new.name == old.name) {
            None => true,
            Some(new) => {
                new.board_id != old.board_id
                    || new.board_version != old.board_version
                    || new.node_id != old.node_id
                    || new.default_page_id != old.default_page_id
            }
        };
        if invalidated
            && let Err(error) =
                delete_variant_setup_rows(&state, &app_id, &event.id, &old.name).await
        {
            tracing::warn!(
                app_id = %app_id,
                event_id = %event.id,
                variant = %old.name,
                %error,
                "failed to drop a replaced variant's setup/registration rows"
            );
        }
    }

    audit_branch!(
        state,
        user,
        app_id,
        "event.canary.variants",
        "Event",
        event_id,
        "Canary variants replaced",
        serde_json::json!({
            "variants": after.iter().map(variant_audit_entry).collect::<Vec<_>>(),
            "previous": before.iter().map(variant_audit_entry).collect::<Vec<_>>(),
        })
    );

    Ok(Json(filter_event_secrets(event)))
}

/// The freshest editable copy of the event: the bucket artifact — rewriting
/// the bucket from the DB mirror would silently revert a concurrently saved
/// artifact-side edit — falling back to the DB-derived copy for mirror-only
/// events. The DB row only proves existence and scoping.
async fn freshest_event(
    app: &flow_like::app::App,
    db_event: flow_like::flow::event::Event,
) -> flow_like::flow::event::Event {
    match app.get_event(&db_event.id, None).await {
        Ok(event) => event,
        Err(error) => {
            tracing::debug!(
                event_id = %db_event.id,
                %error,
                "event artifact not readable; editing the database mirror copy"
            );
            db_event
        }
    }
}

/// A variant's dispatch target as extracted by [`take_variant`], covering both
/// the `EventVariant` list and the legacy single canary.
struct PromotedTarget {
    board_id: String,
    board_version: Option<(u32, u32, u32)>,
    node_id: String,
    default_page_id: Option<String>,
    variables: std::collections::HashMap<String, flow_like::flow::variable::Variable>,
}

/// Remove the named variant from the event, returning its target. When
/// `variants` is empty, the conventional name "canary" addresses the legacy
/// single canary. When the removal leaves no variants, a leftover legacy
/// `canary` is cleared too — `variant_set` falls back to it once `variants`
/// is empty, and a promote/abort must never resurrect a hidden target.
fn take_variant(
    event: &mut flow_like::flow::event::Event,
    name: &str,
) -> Result<PromotedTarget, ApiError> {
    let target = if let Some(position) = event
        .variants
        .iter()
        .position(|variant| variant.name == name)
    {
        let variant = event.variants.remove(position);
        PromotedTarget {
            board_id: variant.board_id,
            board_version: variant.board_version,
            node_id: variant.node_id,
            default_page_id: variant.default_page_id,
            variables: variant.variables,
        }
    } else if event.variants.is_empty()
        && name == "canary"
        && let Some(canary) = event.canary.take()
    {
        PromotedTarget {
            board_id: canary.board_id,
            board_version: canary.board_version,
            node_id: canary.node_id,
            default_page_id: None,
            variables: canary.variables,
        }
    } else {
        return Err(ApiError::not_found(format!(
            "no variant named '{name}' on this event"
        )));
    };
    if event.variants.is_empty() {
        event.canary = None;
    }
    Ok(target)
}

/// The retired variant's inbound rows, dropped after a promote, an abort, or
/// a list replace that removed/renamed/retargeted it: the `(event, variant)`
/// `EventSetup` pointer plus every registration/auth row across event
/// versions. The stable bucket is unreachable here — "stable" is refused as a
/// variant name at upsert.
async fn delete_variant_setup_rows(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    variant: &str,
) -> Result<(), sea_orm::DbErr> {
    use crate::entity::{event_remote_auth, event_remote_registration, event_setup};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    state
        .transaction(|txn| {
            let app_id = app_id.to_string();
            let event_id = event_id.to_string();
            let variant = variant.to_string();
            Box::pin(async move {
                event_setup::Entity::delete_many()
                    .filter(event_setup::Column::AppId.eq(&app_id))
                    .filter(event_setup::Column::EventId.eq(&event_id))
                    .filter(event_setup::Column::Variant.eq(&variant))
                    .exec(txn)
                    .await?;
                event_remote_registration::Entity::delete_many()
                    .filter(event_remote_registration::Column::AppId.eq(&app_id))
                    .filter(event_remote_registration::Column::EventId.eq(&event_id))
                    .filter(event_remote_registration::Column::Variant.eq(&variant))
                    .exec(txn)
                    .await?;
                event_remote_auth::Entity::delete_many()
                    .filter(event_remote_auth::Column::AppId.eq(&app_id))
                    .filter(event_remote_auth::Column::EventId.eq(&event_id))
                    .filter(event_remote_auth::Column::Variant.eq(&variant))
                    .exec(txn)
                    .await?;
                Ok::<_, sea_orm::DbErr>(())
            })
        })
        .await
}

/// Non-fatal stable setup re-run after a promote of a rest/mcp event. It
/// re-reads the synced row, so it computes the bumped event version and
/// writes a fresh registration bucket; inbound keeps serving the old stable
/// bucket until the in-transaction pointer advances. A failure lands in the
/// response as `setup_status` — the promote is never rolled back.
async fn rerun_stable_setup(
    state: &AppState,
    sub: &str,
    app_id: &str,
    event_id: &str,
    user_context: flow_like::flow::execution::UserExecutionContext,
) -> String {
    match super::setup_event::run_event_setup(
        state.clone(),
        sub.to_string(),
        app_id.to_string(),
        event_id.to_string(),
        super::setup_event::SetupEventRequest {
            payload: None,
            profile_id: None,
            timeout_seconds: None,
            force: false,
            variant: None,
        },
        user_context,
    )
    .await
    {
        Ok(response) => match response.error {
            Some(detail) if response.status != "ok" => {
                format!("{}: {}", response.status, detail)
            }
            _ => response.status,
        },
        Err(error) => {
            let detail = error.to_string();
            tracing::warn!(
                app_id = %app_id,
                event_id = %event_id,
                error = %detail,
                "stable re-setup after promote failed; promote kept"
            );
            format!("error: {detail}")
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CanaryPromoteBody {
    /// The variant to promote (`canary` addresses the legacy single canary)
    pub variant: String,
    /// Version cut for the promoted event (default: patch)
    #[schema(value_type = Option<String>)]
    #[serde(default)]
    pub version_type: Option<flow_like::flow::board::VersionType>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CanaryPromoteResponse {
    /// The event after the promote, secrets blanked
    #[schema(value_type = Object)]
    pub event: flow_like::flow::event::Event,
    /// rest/mcp only: outcome of the non-fatal stable setup re-run — `ok`,
    /// or `{status}: {detail}` when the re-setup failed (the promote holds
    /// either way; inbound serves the previous registration set until a
    /// setup succeeds)
    pub setup_status: Option<String>,
    /// Regression-gate outcome for the promoted target, present when the
    /// event's suite gate is `Warn` or `Block`. A `Block` gate with a `fail`
    /// verdict refuses the promote instead (409); `not_run` never blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<PromotionGateSummary>,
}

/// The regression gate as surfaced on a promote response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PromotionGateSummary {
    /// `Warn` or `Block`.
    pub gate_mode: String,
    /// `pass`, `fail` or `not_run`.
    pub verdict: String,
    /// The suite run the verdict came from, when one exists.
    pub suite_run_id: Option<String>,
    /// Regressed case count of a `fail` verdict.
    pub regressed: Option<i32>,
}

fn promotion_gate_summary(
    gate: &crate::execution::regression::gate::PromotionGate,
) -> PromotionGateSummary {
    use crate::execution::regression::gate::GateVerdict;
    let (verdict, suite_run_id, regressed) = match &gate.verdict {
        GateVerdict::Pass { suite_run_id } => ("pass", Some(suite_run_id.clone()), None),
        GateVerdict::Fail {
            suite_run_id,
            regressed,
        } => ("fail", Some(suite_run_id.clone()), Some(*regressed)),
        GateVerdict::NotRun => ("not_run", None, None),
    };
    PromotionGateSummary {
        gate_mode: crate::execution::regression::gate_mode_as_str(gate.mode).to_string(),
        verdict: verdict.to_string(),
        suite_run_id,
        regressed,
    }
}

/// POST /apps/{app_id}/events/{event_id}/canary/promote
///
/// The variant's target becomes the event's primary and the variant is
/// removed. Both stores are written in order: bucket artifact first, then the
/// Postgres row every dispatch path reads.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/canary/promote",
    tag = "events",
    description = "Promote a variant: its target becomes the event's primary and the variant is removed.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = CanaryPromoteBody,
    responses(
        (status = 200, description = "The promoted event and, for REST/MCP events, the setup outcome", body = CanaryPromoteResponse),
        (status = 400, description = "Validation failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Unknown event or variant"),
        (status = 409, description = "Promotion blocked by the regression gate")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/canary/promote",
    skip(state, user, body)
)]
pub async fn promote_canary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<CanaryPromoteBody>,
) -> Result<Json<CanaryPromoteResponse>, ApiError> {
    use flow_like::flow::{
        board::VersionType,
        event::{filter_event_secrets, preserve_event_secrets},
    };

    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let user_context = permission.to_user_context();

    let stored = get_event_from_db(&state.db, &event_id, &app_id)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    if stored.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    // Live and Shadow both promote — promoting a shadow makes its target
    // primary. Variables merge over the event's, variant wins per key.
    let mut updated = stored.clone();
    let promoted = take_variant(&mut updated, &body.variant)?;

    // Regression gate, consulted before anything mutates: Block refuses a
    // `fail` verdict with the failing suite run named; Warn only surfaces it
    // on the response; Off (or no suite) is a no-op.
    let gate = crate::execution::regression::gate::promotion_gate(
        &state,
        &app_id,
        &event_id,
        &promoted.board_id,
        promoted.board_version,
    )
    .await?;
    if let Some(gate) = &gate
        && gate.blocks()
        && let crate::execution::regression::gate::GateVerdict::Fail {
            suite_run_id,
            regressed,
        } = &gate.verdict
    {
        return Err(ApiError::conflict(format!(
            "Promotion blocked by the regression gate: suite run {suite_run_id} recorded {regressed} regressed case(s) against the promoted board version. Fix the regressions, re-run the suite, or lower the suite's gate mode.",
        )));
    }

    updated.board_id = promoted.board_id;
    updated.board_version = promoted.board_version;
    updated.node_id = promoted.node_id;
    if promoted.default_page_id.is_some() {
        updated.default_page_id = promoted.default_page_id;
    }
    updated.variables.extend(promoted.variables);
    preserve_event_secrets(&mut updated, &stored);

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    // An explicit version type forces the cut even when the promoted content
    // matches the primary — the promote must be addressable in the archive.
    let event = app
        .upsert_event(
            updated,
            Some(body.version_type.unwrap_or(VersionType::Patch)),
            Some(true),
        )
        .await
        .map_err(|e| {
            ApiError::bad_request(format!("promoting variant '{}' failed: {e}", body.variant))
        })?;
    app.save()
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    // The row every dispatch path reads moves here; the Nones preserve the
    // sink's stored PAT/OAuth/profile.
    super::db::sync_event_with_sink_tokens(&state.db, &state, &app_id, &event, None, None, None)
        .await?;

    let setup_status = if matches!(event.event_type.as_str(), "rest" | "mcp") {
        Some(rerun_stable_setup(&state, &sub, &app_id, &event.id, user_context).await)
    } else {
        None
    };

    if let Err(error) = delete_variant_setup_rows(&state, &app_id, &event.id, &body.variant).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event.id,
            variant = %body.variant,
            %error,
            "failed to drop the promoted variant's setup/registration rows"
        );
    }

    super::upsert_event::prune_versions_after_save(&state, &app_id, &app, Some(&stored), &event)
        .await;

    audit_branch!(
        state,
        user,
        app_id,
        "event.canary.promote",
        "Event",
        event_id,
        "Canary variant promoted to primary",
        serde_json::json!({
            "variant": body.variant,
            "from": {
                "board_id": stored.board_id,
                "board_version": stored.board_version.map(super::dotted_version_key),
                "default_page_id": stored.default_page_id,
                "event_version": super::dotted_version_key(stored.event_version),
            },
            "to": {
                "board_id": event.board_id,
                "board_version": event.board_version.map(super::dotted_version_key),
                "default_page_id": event.default_page_id,
                "event_version": super::dotted_version_key(event.event_version),
            },
            "gate": gate.as_ref().map(promotion_gate_summary),
        })
    );

    Ok(Json(CanaryPromoteResponse {
        event: filter_event_secrets(event),
        setup_status,
        gate: gate.as_ref().map(promotion_gate_summary),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CanaryAbortBody {
    /// The variant to remove (`canary` addresses the legacy single canary)
    pub variant: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CanaryAbortResponse {
    /// The event after the abort, secrets blanked
    #[schema(value_type = Object)]
    pub event: flow_like::flow::event::Event,
}

/// POST /apps/{app_id}/events/{event_id}/canary/abort
///
/// Removes the variant so its traffic share returns to the primary. Writes
/// both stores — without the Postgres row write the canary would keep carrying
/// its share. Succeeds even when the variant's setup is broken.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/canary/abort",
    tag = "events",
    description = "Abort a variant: remove it and return its traffic share to the primary.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = CanaryAbortBody,
    responses(
        (status = 200, description = "The event without the aborted variant", body = CanaryAbortResponse),
        (status = 400, description = "Validation failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Unknown event or variant")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/canary/abort",
    skip(state, user, body)
)]
pub async fn abort_canary(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<CanaryAbortBody>,
) -> Result<Json<CanaryAbortResponse>, ApiError> {
    use flow_like::flow::event::{filter_event_secrets, preserve_event_secrets};

    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;

    let stored = get_event_from_db(&state.db, &event_id, &app_id)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;
    if stored.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    let aborted_mode = stored
        .variant_set()
        .iter()
        .find(|variant| variant.name == body.variant)
        .map(|variant| variant_mode_label(&variant.mode));
    let mut updated = stored.clone();
    take_variant(&mut updated, &body.variant)?;
    preserve_event_secrets(&mut updated, &stored);

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    // Removing a variant is a content change, so a patch cut happens on its
    // own — fine, inbound serves by pointer, not by version equality.
    let event = app
        .upsert_event(updated, None, Some(true))
        .await
        .map_err(|e| {
            ApiError::bad_request(format!("aborting variant '{}' failed: {e}", body.variant))
        })?;
    app.save()
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    super::db::sync_event_with_sink_tokens(&state.db, &state, &app_id, &event, None, None, None)
        .await?;

    if let Err(error) = delete_variant_setup_rows(&state, &app_id, &event.id, &body.variant).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event.id,
            variant = %body.variant,
            %error,
            "failed to drop the aborted variant's setup/registration rows"
        );
    }

    audit_branch!(
        state,
        user,
        app_id,
        "event.canary.abort",
        "Event",
        event_id,
        "Canary variant removed",
        serde_json::json!({
            "variant": body.variant,
            "mode": aborted_mode,
        })
    );

    Ok(Json(CanaryAbortResponse {
        event: filter_event_secrets(event),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventSetupInfo {
    /// `stable` for the primary target, else the `EventVariant.name`
    pub variant: String,
    /// The event version whose registration bucket this variant serves
    pub event_version: String,
    pub board_id: String,
    /// Dotted `major.minor.patch`; `null` floats on latest
    pub board_version: Option<String>,
    /// `ok`, `running` or `error`
    pub setup_status: Option<String>,
    #[schema(value_type = Option<String>)]
    pub last_setup_at: Option<chrono::NaiveDateTime>,
    pub last_setup_error: Option<String>,
}

/// GET /apps/{app_id}/events/{event_id}/setups
///
/// Per-variant setup health from the `EventSetup` pointer rows, so the UI can
/// show which variants have a live inbound surface. Events whose stable setup
/// predates the pointer table may return no `stable` row — the event row's
/// legacy `setup_status`/`last_setup_*` fields still cover it.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/setups",
    tag = "events",
    description = "Per-variant REST/MCP setup health for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "One entry per variant with a setup on record", body = Vec<EventSetupInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/setups",
    skip(state, user)
)]
pub async fn list_event_setups(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
) -> Result<Json<Vec<EventSetupInfo>>, ApiError> {
    use crate::entity::event_setup;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let rows = event_setup::Entity::find()
        .filter(event_setup::Column::AppId.eq(&app_id))
        .filter(event_setup::Column::EventId.eq(&event_id))
        .order_by_asc(event_setup::Column::Variant)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    Ok(Json(
        rows.into_iter()
            .map(|row| EventSetupInfo {
                variant: row.variant,
                event_version: row.event_version,
                board_id: row.board_id,
                board_version: row.board_version,
                setup_status: row.setup_status,
                last_setup_at: row.last_setup_at,
                last_setup_error: row.last_setup_error,
            })
            .collect(),
    ))
}
