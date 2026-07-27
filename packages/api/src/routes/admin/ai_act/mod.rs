//! Admin EU AI Act inventory router, mounted under `/admin/ai-act`.
//!
//! Provides the platform-wide AI inventory (apps + assessments + attached
//! models), model registry management, model reconciliation, drift
//! acknowledgement, the governance assist endpoint and a CSV/JSON export. All
//! endpoints are feature-gated on `features.ai_act` and require the
//! `ReadPublishing` (read) or `WritePublishing` (write) global permission.

pub mod reconcile;

use crate::{
    entity::{
        ai_act_assessment, ai_act_model_observation, ai_act_model_registry, app_board_score,
        embedding_usage_tracking, llm_usage_tracking, meta, sea_orm_active_enums::AiGpaiPosture,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{get, post, put},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/inventory", get(list_inventory))
        .route("/inventory/export", get(export_inventory))
        .route("/inventory/{app_id}", get(get_inventory_detail))
        .route(
            "/inventory/{app_id}/assessment",
            put(put_inventory_assessment),
        )
        .route(
            "/inventory/{app_id}/reconcile-models",
            post(reconcile_models),
        )
        .route(
            "/inventory/{app_id}/models/{model_id}/acknowledge",
            post(acknowledge_model),
        )
        .route("/models", get(list_models).put(upsert_model))
        .route("/assist/{app_id}", post(assist))
}

fn ensure_feature(state: &AppState) -> Result<(), ApiError> {
    if !state.platform_config.features.ai_act {
        return Err(ApiError::bad_request(
            "The EU AI Act conformity feature is not enabled on this platform.".to_string(),
        ));
    }
    Ok(())
}

/// Rank for sorting: HIGH risk first, then by worst conformity score.
fn risk_rank(category: &str) -> i32 {
    match category {
        "PROHIBITED" => 0,
        "HIGH" => 1,
        "UNDETERMINED" => 2,
        "LIMITED" => 3,
        "MINIMAL" => 4,
        _ => 5,
    }
}

// ---------------------------------------------------------------------------
// GET /inventory
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItem {
    pub app_id: String,
    pub app_name: Option<String>,
    pub risk_category: String,
    pub status: String,
    pub conformity_score: Option<i32>,
    pub conformity_band: Option<String>,
    pub model_count: i64,
    pub unvetted_model_count: i64,
    pub drift_count: i64,
    /// Aggregated worst (minimum) board score across the six quality
    /// categories, or `None` when the app has no computed board scores.
    pub worst_score: Option<i32>,
    pub security_score: Option<i32>,
    pub privacy_score: Option<i32>,
    pub governance_score: Option<i32>,
    pub board_count: i64,
    pub updated_at: String,
}

/// Aggregated board scores per app, joined into the inventory listing.
#[derive(sea_orm::FromQueryResult)]
struct AppScoreAgg {
    app_id: String,
    security: i32,
    privacy: i32,
    governance: i32,
    worst_score: i32,
    board_count: i64,
    updated_at: Option<chrono::NaiveDateTime>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ModelCounts {
    model_count: i64,
    unvetted_model_count: i64,
    drift_count: i64,
    updated_at: Option<chrono::NaiveDateTime>,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryResponse {
    pub items: Vec<InventoryItem>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Deserialize, Debug, IntoParams, ToSchema)]
pub struct InventoryQuery {
    /// Filter by risk category (PROHIBITED/HIGH/LIMITED/MINIMAL/UNDETERMINED).
    pub risk: Option<String>,
    /// Filter by status (UNASSESSED/DRAFT/SUBMITTED/APPROVED/REJECTED/BLOCKED).
    pub status: Option<String>,
    pub search: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/admin/ai-act/inventory",
    tag = "admin",
    description = "Platform-wide EU AI Act inventory. High-risk apps surface first. Requires ReadPublishing permission.",
    params(InventoryQuery),
    responses(
        (status = 200, description = "AI inventory", body = InventoryResponse),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /admin/ai-act/inventory", skip_all)]
pub async fn list_inventory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<InventoryQuery>,
) -> Result<Json<InventoryResponse>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);

    let items = load_inventory_items(&state, &query).await?;

    let total = items.len() as u64;
    let start = ((page - 1) * limit) as usize;
    let paged: Vec<InventoryItem> = items.into_iter().skip(start).take(limit as usize).collect();
    let has_more = (start as u64 + paged.len() as u64) < total;

    Ok(Json(InventoryResponse {
        items: paged,
        total,
        page,
        limit,
        has_more,
    }))
}

async fn load_inventory_items(
    state: &AppState,
    query: &InventoryQuery,
) -> Result<Vec<InventoryItem>, ApiError> {
    let assessment_rows = ai_act_assessment::Entity::find()
        .order_by_desc(ai_act_assessment::Column::Version)
        .all(&state.db)
        .await?;
    let mut assessments: HashMap<String, ai_act_assessment::Model> = HashMap::new();
    for assessment in assessment_rows {
        assessments
            .entry(assessment.app_id.clone())
            .or_insert(assessment);
    }

    // Aggregated board governance/quality scores per app (MIN per category =
    // worst board) so the inventory surfaces security posture even before an
    // EU AI Act questionnaire has been completed.
    let score_aggs: Vec<AppScoreAgg> = app_board_score::Entity::find()
        .select_only()
        .column_as(app_board_score::Column::AppId, "app_id")
        .column_as(
            Expr::col(app_board_score::Column::Security).min(),
            "security",
        )
        .column_as(Expr::col(app_board_score::Column::Privacy).min(), "privacy")
        .column_as(
            Expr::col(app_board_score::Column::Governance).min(),
            "governance",
        )
        .column_as(
            Expr::col(app_board_score::Column::WorstScore).min(),
            "worst_score",
        )
        .column_as(
            Expr::col(app_board_score::Column::BoardId).count(),
            "board_count",
        )
        .column_as(
            Expr::col(app_board_score::Column::UpdatedAt).max(),
            "updated_at",
        )
        .group_by(app_board_score::Column::AppId)
        .into_model::<AppScoreAgg>()
        .all(&state.db)
        .await?;
    let scores: HashMap<String, AppScoreAgg> = score_aggs
        .into_iter()
        .map(|agg| (agg.app_id.clone(), agg))
        .collect();

    let observations = ai_act_model_observation::Entity::find()
        .all(&state.db)
        .await?;
    let mut model_counts: HashMap<String, ModelCounts> = HashMap::new();
    for obs in &observations {
        let entry = model_counts.entry(obs.app_id.clone()).or_default();
        entry.model_count += 1;
        if !obs.vetted {
            entry.unvetted_model_count += 1;
        }
        if obs.drift_flagged {
            entry.drift_count += 1;
        }
        entry.updated_at = Some(match entry.updated_at {
            Some(current) => current.max(obs.last_seen_at),
            None => obs.last_seen_at,
        });
    }

    let mut app_ids: HashSet<String> = HashSet::new();
    app_ids.extend(assessments.keys().cloned());
    app_ids.extend(scores.keys().cloned());
    app_ids.extend(model_counts.keys().cloned());
    let app_ids: Vec<String> = app_ids.into_iter().collect();
    let names = load_app_names(state, &app_ids).await;

    let mut items: Vec<InventoryItem> = app_ids
        .into_iter()
        .map(|app_id| {
            let assessment = assessments.get(&app_id);
            let score = scores.get(&app_id);
            let models = model_counts.get(&app_id).copied().unwrap_or_default();
            let updated_at = assessment
                .map(|a| a.updated_at.to_string())
                .or_else(|| score.and_then(|s| s.updated_at).map(|ts| ts.to_string()))
                .or_else(|| models.updated_at.map(|ts| ts.to_string()))
                .unwrap_or_default();

            InventoryItem {
                app_name: names.get(&app_id).cloned().flatten(),
                risk_category: assessment
                    .map(|a| format!("{:?}", a.risk_category).to_uppercase())
                    .unwrap_or_else(|| "UNDETERMINED".to_string()),
                status: assessment
                    .map(|a| format!("{:?}", a.status).to_uppercase())
                    .unwrap_or_else(|| "UNASSESSED".to_string()),
                conformity_score: assessment.and_then(|a| a.conformity_score),
                conformity_band: assessment.and_then(|a| a.conformity_band.clone()),
                model_count: models.model_count,
                unvetted_model_count: models.unvetted_model_count,
                drift_count: models.drift_count,
                worst_score: score.map(|s| s.worst_score),
                security_score: score.map(|s| s.security),
                privacy_score: score.map(|s| s.privacy),
                governance_score: score.map(|s| s.governance),
                board_count: score.map(|s| s.board_count).unwrap_or(0),
                updated_at,
                app_id,
            }
        })
        .collect();

    if let Some(search) = query.search.as_ref().map(|s| s.trim().to_lowercase()) {
        if !search.is_empty() {
            items.retain(|item| {
                item.app_id.to_lowercase().contains(&search)
                    || item
                        .app_name
                        .as_ref()
                        .map(|name| name.to_lowercase().contains(&search))
                        .unwrap_or(false)
            });
        }
    }

    if let Some(risk) = query.risk.as_deref() {
        if parse_risk(risk).is_some() {
            let risk = risk.to_uppercase();
            items.retain(|item| item.risk_category == risk);
        }
    }

    if let Some(status) = query.status.as_deref() {
        let status = status.to_uppercase();
        if status == "UNASSESSED" || parse_status(&status).is_some() {
            items.retain(|item| item.status == status);
        }
    }

    // Sort: high-risk first, then lowest conformity score, then lowest
    // governance score so unassessed scored apps are still actionable.
    items.sort_by(|a, b| {
        risk_rank(&a.risk_category)
            .cmp(&risk_rank(&b.risk_category))
            .then(
                a.conformity_score
                    .unwrap_or(101)
                    .cmp(&b.conformity_score.unwrap_or(101)),
            )
            .then(
                a.worst_score
                    .unwrap_or(11)
                    .cmp(&b.worst_score.unwrap_or(11)),
            )
            .then_with(|| {
                a.app_name
                    .as_deref()
                    .unwrap_or(a.app_id.as_str())
                    .cmp(b.app_name.as_deref().unwrap_or(b.app_id.as_str()))
            })
    });

    Ok(items)
}

async fn load_app_names(state: &AppState, app_ids: &[String]) -> HashMap<String, Option<String>> {
    let mut names = HashMap::new();
    if app_ids.is_empty() {
        return names;
    }
    // Best-effort: read the English meta name per app.
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(app_ids.to_vec()))
        .all(&state.db)
        .await
        .unwrap_or_default();
    for m in metas {
        if let Some(app_id) = m.app_id {
            names.entry(app_id).or_insert(Some(m.name));
        }
    }
    names
}

fn parse_risk(s: &str) -> Option<crate::entity::sea_orm_active_enums::AiRiskCategory> {
    use crate::entity::sea_orm_active_enums::AiRiskCategory as R;
    match s.to_uppercase().as_str() {
        "PROHIBITED" => Some(R::Prohibited),
        "HIGH" => Some(R::High),
        "LIMITED" => Some(R::Limited),
        "MINIMAL" => Some(R::Minimal),
        "UNDETERMINED" => Some(R::Undetermined),
        _ => None,
    }
}

fn parse_status(s: &str) -> Option<crate::entity::sea_orm_active_enums::AiActAssessmentStatus> {
    use crate::entity::sea_orm_active_enums::AiActAssessmentStatus as S;
    match s.to_uppercase().as_str() {
        "DRAFT" => Some(S::Draft),
        "SUBMITTED" => Some(S::Submitted),
        "APPROVED" => Some(S::Approved),
        "REJECTED" => Some(S::Rejected),
        "BLOCKED" => Some(S::Blocked),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// GET /inventory/{app_id}
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelObservationItem {
    pub id: String,
    pub model_id: String,
    pub provider: Option<String>,
    pub source: String,
    pub posture: String,
    pub hosted: bool,
    pub open_licence: bool,
    pub systemic_risk: bool,
    pub vetted: bool,
    pub dynamic_selector: bool,
    pub drift_flagged: bool,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventoryDetailResponse {
    pub app_id: String,
    pub app_name: Option<String>,
    pub assessment: Option<serde_json::Value>,
    pub models: Vec<ModelObservationItem>,
    /// Canonical questionnaire schema (serialised JSON) so reviewers always see
    /// the full questionnaire, even when the owner has not started one.
    pub schema: serde_json::Value,
    /// Auto-derived signals from the static board scan (serialised JSON).
    pub signals: serde_json::Value,
    /// Current answers: the submitted assessment's answers, or a conservative
    /// prefill derived from the signals when nothing has been submitted yet.
    pub answers: serde_json::Value,
    /// Authoritative live classification for the current answers (serialised
    /// JSON). Recomputed server-side so it always reflects reality.
    pub classification: serde_json::Value,
    /// Ordered, actionable tips for raising the conformity score (serialised
    /// JSON array of recommendations).
    pub recommendations: serde_json::Value,
    /// Whether a stored assessment exists for this app.
    pub has_assessment: bool,
}

impl From<ai_act_model_observation::Model> for ModelObservationItem {
    fn from(m: ai_act_model_observation::Model) -> Self {
        ModelObservationItem {
            id: m.id,
            model_id: m.model_id,
            provider: m.provider,
            source: format!("{:?}", m.source).to_uppercase(),
            posture: format!("{:?}", m.posture).to_uppercase(),
            hosted: m.hosted,
            open_licence: m.open_licence,
            systemic_risk: m.systemic_risk,
            vetted: m.vetted,
            dynamic_selector: m.dynamic_selector,
            drift_flagged: m.drift_flagged,
            first_seen_at: m.first_seen_at.to_string(),
            last_seen_at: m.last_seen_at.to_string(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/admin/ai-act/inventory/{app_id}",
    tag = "admin",
    description = "Detailed EU AI Act inventory for one app: assessment and attached models. Requires ReadPublishing permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Inventory detail", body = InventoryDetailResponse),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /admin/ai-act/inventory/{app_id}", skip(state, user))]
pub async fn get_inventory_detail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<InventoryDetailResponse>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    // Opportunistically reconcile the attached-model list from monitoring +
    // board scan so the detail view always reflects current reality (best
    // effort — never fail the read if the app or scan is unavailable). The
    // resulting signals also drive the questionnaire prefill + classification.
    let sub = user.sub()?;
    let mut signals = crate::routes::app::ai_act::signals::Signals::default();
    if let Ok(app) = state.master_app(&sub, &app_id, &state).await {
        if let Ok(scanned) =
            crate::routes::app::ai_act::board_scan::scan_app_signals(&state, &sub, &app_id, &app)
                .await
        {
            let _ = reconcile::reconcile_app_models(&state, &app_id, &scanned).await;
            signals = scanned;
        }
    }

    let assessment = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    let models = ai_act_model_observation::Entity::find()
        .filter(ai_act_model_observation::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_model_observation::Column::LastSeenAt)
        .all(&state.db)
        .await?;

    let names = load_app_names(&state, &[app_id.clone()]).await;

    let has_assessment = assessment.is_some();
    let answers = assessment
        .as_ref()
        .map(|a| a.answers.clone())
        .unwrap_or_else(|| crate::routes::app::ai_act::prefill_answers(&signals));
    let classification = crate::routes::app::ai_act::questionnaire::classify(&answers, &signals);
    let recommendations = crate::routes::app::ai_act::questionnaire::recommendations(
        &answers,
        &signals,
        &classification,
    );
    let schema = crate::routes::app::ai_act::questionnaire::questionnaire_schema();

    // Resolve the responsible person (defaulting to the app owner for legacy
    // records) and the reviewer's display name so the detail header is complete
    // without requiring a re-submission.
    let assessment_json = match assessment {
        Some(a) => {
            // The responsible person is hard-linked to the app owner. Resolve the
            // full contact card so admins can reach out, falling back to the
            // stored user id then to the resolved owner for legacy records.
            let responsible_user_id = match a.responsible_user_id.clone() {
                Some(uid) => Some(uid),
                None => crate::routes::app::ai_act::resolve_app_owner(&state, &app_id).await,
            };
            let responsible_person = match &responsible_user_id {
                Some(uid) => crate::routes::app::ai_act::load_user_contact(&state, uid).await,
                None => None,
            };
            let responsible_name = responsible_person
                .as_ref()
                .and_then(|p| p.name.clone())
                .or_else(|| a.responsible_name.clone());
            let responsible_email = responsible_person
                .as_ref()
                .and_then(|p| p.email.clone())
                .or_else(|| a.responsible_email.clone());
            let reviewed_by_name = match &a.reviewed_by_id {
                Some(uid) => crate::routes::app::ai_act::load_user_identity(&state, uid)
                    .await
                    .and_then(|(name, _)| name),
                None => None,
            };
            let mut value =
                serde_json::to_value(crate::routes::app::ai_act::AssessmentResponse::from(a))
                    .unwrap_or(serde_json::Value::Null);
            if let serde_json::Value::Object(map) = &mut value {
                map.insert(
                    "responsibleName".into(),
                    serde_json::to_value(&responsible_name).unwrap_or(serde_json::Value::Null),
                );
                map.insert(
                    "responsibleEmail".into(),
                    serde_json::to_value(&responsible_email).unwrap_or(serde_json::Value::Null),
                );
                map.insert(
                    "responsiblePerson".into(),
                    serde_json::to_value(&responsible_person).unwrap_or(serde_json::Value::Null),
                );
                map.insert(
                    "reviewedByName".into(),
                    serde_json::to_value(&reviewed_by_name).unwrap_or(serde_json::Value::Null),
                );
            }
            Some(value)
        }
        None => None,
    };

    Ok(Json(InventoryDetailResponse {
        app_name: names.get(&app_id).cloned().flatten(),
        assessment: assessment_json,
        models: models.into_iter().map(ModelObservationItem::from).collect(),
        schema: serde_json::to_value(&schema).unwrap_or(serde_json::Value::Null),
        signals: serde_json::to_value(&signals).unwrap_or(serde_json::Value::Null),
        answers,
        classification: serde_json::to_value(&classification).unwrap_or(serde_json::Value::Null),
        recommendations: serde_json::to_value(&recommendations)
            .unwrap_or(serde_json::Value::Array(Vec::new())),
        has_assessment,
        app_id,
    }))
}

// ---------------------------------------------------------------------------
// PUT /inventory/{app_id}/assessment  (admin edit / review override)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUpsertAssessmentBody {
    /// Full questionnaire answers map. Risk and score are always recomputed
    /// server-side from these answers and the live signals.
    pub answers: serde_json::Value,
    /// Optional review decision: DRAFT | SUBMITTED | APPROVED | REJECTED.
    /// When APPROVED or REJECTED the record is stamped as reviewed by the
    /// current admin. A prohibited classification always forces BLOCKED.
    pub review_status: Option<String>,
    pub review_note: Option<String>,
}

#[utoipa::path(
    put,
    path = "/admin/ai-act/inventory/{app_id}/assessment",
    tag = "admin",
    description = "Edit or review an app's EU AI Act assessment as an administrator. The risk category and conformity score are always recomputed server-side. Requires WritePublishing permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = AdminUpsertAssessmentBody,
    responses(
        (status = 200, description = "Stored assessment", body = serde_json::Value),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(
    name = "PUT /admin/ai-act/inventory/{app_id}/assessment",
    skip(state, user, body)
)]
pub async fn put_inventory_assessment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<AdminUpsertAssessmentBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use crate::entity::sea_orm_active_enums::AiActAssessmentStatus as S;

    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::WritePublishing)
        .await?;

    let sub = user.sub()?;

    // Recompute signals so the stored classification reflects current reality.
    let mut signals = crate::routes::app::ai_act::signals::Signals::default();
    if let Ok(app) = state.master_app(&sub, &app_id, &state).await {
        if let Ok(scanned) =
            crate::routes::app::ai_act::board_scan::scan_app_signals(&state, &sub, &app_id, &app)
                .await
        {
            signals = scanned;
        }
    }

    let classification =
        crate::routes::app::ai_act::questionnaire::classify(&body.answers, &signals);
    let now = chrono::Utc::now().naive_utc();

    let requested_status = body.review_status.as_deref().and_then(parse_status);
    let is_review_decision = matches!(requested_status, Some(S::Approved) | Some(S::Rejected));

    let existing = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    // Prohibited classification always blocks; otherwise honour the requested
    // decision, else keep the existing status, else default to DRAFT.
    let status = if classification.blocked {
        S::Blocked
    } else if let Some(s) = requested_status.clone() {
        s
    } else if let Some(ref e) = existing {
        e.status.clone()
    } else {
        S::Draft
    };

    let signals_json = serde_json::to_value(&signals).ok();
    let obligations_json = serde_json::to_value(&classification.transparency_obligations).ok();
    let conformity_band = classification
        .conformity_band
        .map(|b| b.as_str().to_string());
    let risk_entity = parse_risk(classification.risk_category.as_str())
        .unwrap_or(crate::entity::sea_orm_active_enums::AiRiskCategory::Undetermined);

    // The responsible person is always hard-linked to the app owner — admins
    // review and adjust answers, but cannot reassign accountability.
    let (responsible_user_id, responsible_name, responsible_email) =
        crate::routes::app::ai_act::resolve_responsible_person(&state, &app_id, &sub).await;

    let stored = if let Some(existing) = existing {
        let mut active: ai_act_assessment::ActiveModel = existing.into();
        active.status = Set(status);
        active.risk_category = Set(risk_entity);
        active.conformity_score = Set(classification.conformity_score);
        active.conformity_band = Set(conformity_band);
        active.answers = Set(body.answers.clone());
        active.signals = Set(signals_json);
        active.transparency_obligations = Set(obligations_json);
        active.responsible_user_id = Set(Some(responsible_user_id.clone()));
        active.responsible_name = Set(responsible_name.clone());
        active.responsible_email = Set(responsible_email.clone());
        if body.review_note.is_some() {
            active.review_note = Set(body.review_note.clone());
        }
        if is_review_decision {
            active.reviewed_by_id = Set(Some(sub.clone()));
            active.reviewed_at = Set(Some(now));
        }
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let submitted_at = matches!(status, S::Submitted | S::Approved).then_some(now);
        let active = ai_act_assessment::ActiveModel {
            id: Set(flow_like_types::create_id()),
            app_id: Set(app_id.clone()),
            version: Set(1),
            status: Set(status),
            risk_category: Set(risk_entity),
            conformity_score: Set(classification.conformity_score),
            conformity_band: Set(conformity_band),
            answers: Set(body.answers.clone()),
            signals: Set(signals_json),
            transparency_obligations: Set(obligations_json),
            responsible_user_id: Set(Some(responsible_user_id.clone())),
            responsible_name: Set(responsible_name.clone()),
            responsible_email: Set(responsible_email.clone()),
            submitted_at: Set(submitted_at),
            reviewed_by_id: Set(is_review_decision.then(|| sub.clone())),
            reviewed_at: Set(is_review_decision.then_some(now)),
            review_note: Set(body.review_note.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    let response = crate::routes::app::ai_act::AssessmentResponse::from(stored);
    Ok(Json(
        serde_json::to_value(&response).unwrap_or(serde_json::Value::Null),
    ))
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileResponse {
    pub app_id: String,
    pub reconciled: usize,
}

#[utoipa::path(
    post,
    path = "/admin/ai-act/inventory/{app_id}/reconcile-models",
    tag = "admin",
    description = "Reconcile the attached-model observations for an app from monitoring + board scan. Requires WritePublishing permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Reconciliation result", body = ReconcileResponse),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(
    name = "POST /admin/ai-act/inventory/{app_id}/reconcile-models",
    skip(state, user)
)]
pub async fn reconcile_models(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<ReconcileResponse>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::WritePublishing)
        .await?;

    let sub = user.sub()?;
    let app = state.master_app(&sub, &app_id, &state).await?;
    let signals =
        crate::routes::app::ai_act::board_scan::scan_app_signals(&state, &sub, &app_id, &app)
            .await
            .unwrap_or_default();

    let reconciled = reconcile::reconcile_app_models(&state, &app_id, &signals).await?;

    Ok(Json(ReconcileResponse { app_id, reconciled }))
}

// ---------------------------------------------------------------------------
// POST /inventory/{app_id}/models/{model_id}/acknowledge
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/admin/ai-act/inventory/{app_id}/models/{model_id}/acknowledge",
    tag = "admin",
    description = "Acknowledge a drift-flagged model observation, clearing the drift flag. Requires WritePublishing permission.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("model_id" = String, Path, description = "Observation ID"),
    ),
    responses(
        (status = 200, description = "Acknowledged"),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(
    name = "POST /admin/ai-act/inventory/{app_id}/models/{model_id}/acknowledge",
    skip(state, user)
)]
pub async fn acknowledge_model(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, observation_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::WritePublishing)
        .await?;

    let obs = ai_act_model_observation::Entity::find_by_id(&observation_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if obs.app_id != app_id {
        return Err(ApiError::NOT_FOUND);
    }

    let mut active: ai_act_model_observation::ActiveModel = obs.into();
    active.drift_flagged = Set(false);
    active.update(&state.db).await?;

    Ok(Json(serde_json::json!({ "acknowledged": true })))
}

// ---------------------------------------------------------------------------
// GET/PUT /models  (registry)
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistryItem {
    pub id: String,
    pub provider: String,
    pub model_id: String,
    pub posture: String,
    pub hosted: bool,
    pub open_licence: bool,
    pub systemic_risk: bool,
    pub vetted: bool,
    pub note: Option<String>,
    pub updated_at: String,
    /// True when this provider/model pair has been observed in a published app.
    pub observed: bool,
    /// True when this row exists in the editable platform registry.
    pub registered: bool,
    /// True when an observed model still needs an explicit GPAI rating/review.
    pub needs_rating: bool,
    /// Number of observation / usage records behind this model row.
    pub observed_count: i64,
}

#[derive(Clone, Debug, sea_orm::FromQueryResult)]
struct ObservedRegistryModel {
    provider: Option<String>,
    model_id: String,
    observed_count: i64,
    last_seen_at: Option<chrono::NaiveDateTime>,
}

fn normalise_registry_provider(provider: Option<&str>) -> String {
    let provider = provider.unwrap_or("unknown").trim();
    if provider.is_empty() {
        "unknown".to_string()
    } else {
        provider.to_string()
    }
}

fn registry_key(provider: Option<&str>, model_id: &str) -> (String, String) {
    (
        normalise_registry_provider(provider),
        model_id.trim().to_string(),
    )
}

fn observation_registry_condition(provider: &str, model_id: &str) -> sea_orm::Condition {
    let provider = normalise_registry_provider(Some(provider));
    let provider_condition = if provider == "unknown" {
        sea_orm::Condition::any()
            .add(ai_act_model_observation::Column::Provider.is_null())
            .add(ai_act_model_observation::Column::Provider.eq("unknown"))
    } else {
        sea_orm::Condition::all().add(ai_act_model_observation::Column::Provider.eq(provider))
    };

    sea_orm::Condition::all()
        .add(ai_act_model_observation::Column::ModelId.eq(model_id.trim().to_string()))
        .add(ai_act_model_observation::Column::DynamicSelector.eq(false))
        .add(provider_condition)
}

fn merge_observed_registry_model(
    observed_by_key: &mut HashMap<(String, String), ObservedRegistryModel>,
    mut model: ObservedRegistryModel,
) {
    model.model_id = model.model_id.trim().to_string();
    if model.model_id.is_empty() {
        return;
    }
    model.provider = Some(normalise_registry_provider(model.provider.as_deref()));
    model.observed_count = model.observed_count.max(1);

    let key = registry_key(model.provider.as_deref(), &model.model_id);
    observed_by_key
        .entry(key)
        .and_modify(|existing| {
            existing.observed_count += model.observed_count;
            existing.last_seen_at = match (existing.last_seen_at, model.last_seen_at) {
                (Some(current), Some(next)) => Some(current.max(next)),
                (None, Some(next)) => Some(next),
                (current, None) => current,
            };
        })
        .or_insert(model);
}

impl RegistryItem {
    fn from_registry(m: ai_act_model_registry::Model, observed: bool, observed_count: i64) -> Self {
        let needs_rating = observed && (m.posture == AiGpaiPosture::Unknown || !m.vetted);
        RegistryItem {
            id: m.id,
            provider: m.provider,
            model_id: m.model_id,
            posture: format!("{:?}", m.posture).to_uppercase(),
            hosted: m.hosted,
            open_licence: m.open_licence,
            systemic_risk: m.systemic_risk,
            vetted: m.vetted,
            note: m.note,
            updated_at: m.updated_at.to_string(),
            observed,
            registered: true,
            needs_rating,
            observed_count,
        }
    }

    fn from_observed(m: ObservedRegistryModel) -> Self {
        let provider = normalise_registry_provider(m.provider.as_deref());
        let observed_count = m.observed_count.max(1);
        let note = if observed_count == 1 {
            "Observed in use; needs GPAI rating.".to_string()
        } else {
            format!("Observed in use across {observed_count} observations; needs GPAI rating.")
        };

        RegistryItem {
            id: format!("observed/{}/{}", provider, m.model_id),
            provider,
            model_id: m.model_id,
            posture: "UNKNOWN".to_string(),
            hosted: false,
            open_licence: false,
            systemic_risk: false,
            vetted: false,
            note: Some(note),
            updated_at: m.last_seen_at.map(|ts| ts.to_string()).unwrap_or_default(),
            observed: true,
            registered: false,
            needs_rating: true,
            observed_count,
        }
    }
}

impl From<ai_act_model_registry::Model> for RegistryItem {
    fn from(m: ai_act_model_registry::Model) -> Self {
        RegistryItem::from_registry(m, false, 0)
    }
}

#[utoipa::path(
    get,
    path = "/admin/ai-act/models",
    tag = "admin",
    description = "List the platform model registry (GPAI posture). Requires ReadPublishing permission.",
    responses(
        (status = 200, description = "Model registry", body = Vec<RegistryItem>),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /admin/ai-act/models", skip(state, user))]
pub async fn list_models(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<RegistryItem>>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let records = ai_act_model_registry::Entity::find()
        .order_by_asc(ai_act_model_registry::Column::Provider)
        .order_by_asc(ai_act_model_registry::Column::ModelId)
        .all(&state.db)
        .await?;

    let observed_models = ai_act_model_observation::Entity::find()
        .filter(ai_act_model_observation::Column::DynamicSelector.eq(false))
        .select_only()
        .column_as(ai_act_model_observation::Column::Provider, "provider")
        .column_as(ai_act_model_observation::Column::ModelId, "model_id")
        .column_as(
            Expr::col(ai_act_model_observation::Column::AppId).count(),
            "observed_count",
        )
        .column_as(
            Expr::col(ai_act_model_observation::Column::LastSeenAt).max(),
            "last_seen_at",
        )
        .group_by(ai_act_model_observation::Column::Provider)
        .group_by(ai_act_model_observation::Column::ModelId)
        .into_model::<ObservedRegistryModel>()
        .all(&state.db)
        .await?;

    let observed_llm_models = llm_usage_tracking::Entity::find()
        .select_only()
        .column_as(llm_usage_tracking::Column::Provider, "provider")
        .column_as(llm_usage_tracking::Column::ModelId, "model_id")
        .column_as(
            Expr::col(llm_usage_tracking::Column::Id).count(),
            "observed_count",
        )
        .column_as(
            Expr::col(llm_usage_tracking::Column::CreatedAt).max(),
            "last_seen_at",
        )
        .group_by(llm_usage_tracking::Column::Provider)
        .group_by(llm_usage_tracking::Column::ModelId)
        .into_model::<ObservedRegistryModel>()
        .all(&state.db)
        .await?;

    let observed_embedding_models = embedding_usage_tracking::Entity::find()
        .select_only()
        .column_as(embedding_usage_tracking::Column::Provider, "provider")
        .column_as(embedding_usage_tracking::Column::ModelId, "model_id")
        .column_as(
            Expr::col(embedding_usage_tracking::Column::Id).count(),
            "observed_count",
        )
        .column_as(
            Expr::col(embedding_usage_tracking::Column::CreatedAt).max(),
            "last_seen_at",
        )
        .group_by(embedding_usage_tracking::Column::Provider)
        .group_by(embedding_usage_tracking::Column::ModelId)
        .into_model::<ObservedRegistryModel>()
        .all(&state.db)
        .await?;

    let mut observed_by_key: HashMap<(String, String), ObservedRegistryModel> = HashMap::new();
    for model in observed_models
        .into_iter()
        .chain(observed_llm_models)
        .chain(observed_embedding_models)
    {
        merge_observed_registry_model(&mut observed_by_key, model);
    }

    let mut registered_keys = HashSet::new();
    let mut items = Vec::new();
    for record in records {
        let key = registry_key(Some(&record.provider), &record.model_id);
        let observed_count = observed_by_key
            .get(&key)
            .map(|m| m.observed_count.max(1))
            .unwrap_or(0);
        let observed = observed_count > 0;
        registered_keys.insert(key);
        items.push(RegistryItem::from_registry(
            record,
            observed,
            observed_count,
        ));
    }

    for (key, observed) in observed_by_key {
        if !registered_keys.contains(&key) {
            items.push(RegistryItem::from_observed(observed));
        }
    }

    items.sort_by(|a, b| {
        b.needs_rating
            .cmp(&a.needs_rating)
            .then_with(|| a.registered.cmp(&b.registered))
            .then_with(|| b.observed.cmp(&a.observed))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    Ok(Json(items))
}

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertModelBody {
    pub provider: String,
    pub model_id: String,
    /// One of UNKNOWN/HOSTED/OPEN_LICENCE/CLOSED/SYSTEMIC.
    pub posture: String,
    #[serde(default)]
    pub hosted: bool,
    #[serde(default)]
    pub open_licence: bool,
    #[serde(default)]
    pub systemic_risk: bool,
    #[serde(default)]
    pub vetted: bool,
    pub note: Option<String>,
}

fn parse_posture(s: &str) -> AiGpaiPosture {
    match s.to_uppercase().as_str() {
        "HOSTED" => AiGpaiPosture::Hosted,
        "OPEN_LICENCE" => AiGpaiPosture::OpenLicence,
        "CLOSED" => AiGpaiPosture::Closed,
        "SYSTEMIC" => AiGpaiPosture::Systemic,
        _ => AiGpaiPosture::Unknown,
    }
}

#[utoipa::path(
    put,
    path = "/admin/ai-act/models",
    tag = "admin",
    description = "Create or update a model registry entry (provider + model_id is the key). Requires WritePublishing permission.",
    request_body = UpsertModelBody,
    responses(
        (status = 200, description = "Stored registry entry", body = RegistryItem),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "PUT /admin/ai-act/models", skip(state, user, body))]
pub async fn upsert_model(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<UpsertModelBody>,
) -> Result<Json<RegistryItem>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::WritePublishing)
        .await?;

    let now = chrono::Utc::now().naive_utc();
    let posture = parse_posture(&body.posture);
    let provider = normalise_registry_provider(Some(&body.provider));
    let model_id = body.model_id.trim().to_string();

    if model_id.is_empty() {
        return Err(ApiError::bad_request("modelId is required"));
    }

    let existing = ai_act_model_registry::Entity::find()
        .filter(ai_act_model_registry::Column::Provider.eq(&provider))
        .filter(ai_act_model_registry::Column::ModelId.eq(&model_id))
        .one(&state.db)
        .await?;

    let stored = if let Some(existing) = existing {
        let mut active: ai_act_model_registry::ActiveModel = existing.into();
        active.posture = Set(posture.clone());
        active.hosted = Set(body.hosted);
        active.open_licence = Set(body.open_licence);
        active.systemic_risk = Set(body.systemic_risk);
        active.vetted = Set(body.vetted);
        active.note = Set(body.note.clone());
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = ai_act_model_registry::ActiveModel {
            id: Set(flow_like_types::create_id()),
            provider: Set(provider),
            model_id: Set(model_id),
            posture: Set(posture.clone()),
            hosted: Set(body.hosted),
            open_licence: Set(body.open_licence),
            systemic_risk: Set(body.systemic_risk),
            vetted: Set(body.vetted),
            note: Set(body.note.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    let mut update_model = <ai_act_model_observation::ActiveModel as Default>::default();
    update_model.posture = Set(stored.posture.clone());
    update_model.hosted = Set(stored.hosted);
    update_model.open_licence = Set(stored.open_licence);
    update_model.systemic_risk = Set(stored.systemic_risk);
    update_model.vetted = Set(stored.vetted);

    let update_res = ai_act_model_observation::Entity::update_many()
        .filter(observation_registry_condition(
            &stored.provider,
            &stored.model_id,
        ))
        .set(update_model)
        .exec(&state.db)
        .await?;
    let observed_count = update_res.rows_affected as i64;

    Ok(Json(RegistryItem::from_registry(
        stored,
        observed_count > 0,
        observed_count,
    )))
}

// ---------------------------------------------------------------------------
// POST /assist/{app_id}  (governance agent)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistBody {
    #[serde(default)]
    pub model_id: Option<String>,
    /// Optional caller profile used to resolve which model bits to use.
    /// When omitted the agent falls back to the platform default model.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub profile: Option<flow_like::profile::Profile>,
}

#[utoipa::path(
    post,
    path = "/admin/ai-act/assist/{app_id}",
    tag = "admin",
    description = "Run the governance FlowPilot agent over an app's boards (FlowScript state) for admin review. Read-only. Requires ReadPublishing permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = AssistBody,
    responses(
        (status = 200, description = "Governance suggestion"),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "POST /admin/ai-act/assist/{app_id}", skip(state, user, body))]
pub async fn assist(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<AssistBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    // App-scoped hosted usage must never silently fall back to user-only
    // accounting. Global publishing access alone does not authorize charging
    // an app, so require app execution membership as well.
    user.execution_app_permission(&app_id, &state).await?;
    let usage_context = Some(flow_like::models::llm::ModelUsageContext {
        app_id: Some(app_id.clone()),
        run_id: None,
    });

    let sub = user.sub()?;
    // Hosted Bit models bill against the caller's token via this server's metered proxy.
    let token = crate::routes::ai::copilot::user_access_token(&user);
    let (suggestion, signals, model) = crate::routes::ai::governance::run_governance_agent(
        &state,
        &sub,
        &app_id,
        body.model_id,
        body.profile,
        token,
        usage_context,
    )
    .await?;

    Ok(Json(serde_json::json!({
        "suggestion": suggestion,
        "signals": signals,
        "model": model,
    })))
}

// ---------------------------------------------------------------------------
// GET /inventory/export  (CSV / JSON)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, IntoParams, ToSchema)]
pub struct ExportQuery {
    /// `csv` (default) or `json`.
    pub format: Option<String>,
}

#[utoipa::path(
    get,
    path = "/admin/ai-act/inventory/export",
    tag = "admin",
    description = "Export the EU AI Act inventory as CSV or JSON. Requires ReadPublishing permission.",
    params(ExportQuery),
    responses(
        (status = 200, description = "Export file"),
        (status = 400, description = "Feature disabled"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /admin/ai-act/inventory/export", skip_all)]
pub async fn export_inventory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ExportQuery>,
) -> Result<axum::response::Response, ApiError> {
    ensure_feature(&state)?;
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let rows = load_inventory_items(
        &state,
        &InventoryQuery {
            risk: None,
            status: None,
            search: None,
            page: None,
            limit: None,
        },
    )
    .await?;

    let format = query.format.as_deref().unwrap_or("csv").to_lowercase();

    if format == "json" {
        let body = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
        return Ok((
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=\"ai-act-inventory.json\"",
                ),
            ],
            body,
        )
            .into_response());
    }

    // CSV
    let mut csv = String::from(concat!(
        "appId,appName,riskCategory,status,conformityScore,conformityBand,",
        "securityScore,privacyScore,worstScore,modelCount,unvettedModelCount,driftCount,updatedAt\n"
    ));
    for row in rows {
        let name = row.app_name.unwrap_or_default();
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&row.app_id),
            csv_escape(&name),
            row.risk_category,
            row.status,
            row.conformity_score
                .map(|s| s.to_string())
                .unwrap_or_default(),
            row.conformity_band.unwrap_or_default(),
            row.security_score
                .map(|s| s.to_string())
                .unwrap_or_default(),
            row.privacy_score.map(|s| s.to_string()).unwrap_or_default(),
            row.worst_score.map(|s| s.to_string()).unwrap_or_default(),
            row.model_count,
            row.unvetted_model_count,
            row.drift_count,
            row.updated_at,
        ));
    }

    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/csv"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"ai-act-inventory.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

/// Minimal CSV field escaping: quote fields containing commas, quotes or line breaks.
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}
