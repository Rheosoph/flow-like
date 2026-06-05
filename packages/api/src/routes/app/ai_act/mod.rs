//! Owner-facing EU AI Act endpoints, nested at `/apps/{app_id}/ai-act`.
//!
//! All endpoints are feature-gated on `features.ai_act` and require owner
//! permission on the app. The classifier is always recomputed server-side so
//! the stored risk/score cannot be tampered with by the client. See
//! todo/EU-AI.md §5 and §7.

pub mod board_scan;
pub mod questionnaire;
pub mod signals;

use crate::{
    entity::{ai_act_assessment, sea_orm_active_enums::AiActAssessmentStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use flow_like_types::create_id;
use questionnaire::{
    Classification, QuestionnaireSchema, RiskCategory, classify, questionnaire_schema,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use signals::Signals;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/questionnaire", get(get_questionnaire))
        .route("/classify", post(classify_preview))
        .route("/assessment", get(get_assessment).put(put_assessment))
        .route("/assessment/suggest", post(suggest_assessment))
}

/// Reject the request when the AI Act feature is disabled for the platform.
fn ensure_feature(state: &AppState) -> Result<(), ApiError> {
    if !state.platform_config.features.ai_act {
        return Err(ApiError::bad_request(
            "The EU AI Act conformity feature is not enabled on this platform.".to_string(),
        ));
    }
    Ok(())
}

/// Resolve a human display name + email for a platform user id (best effort).
/// Prefers the profile name, then the username, then the preferred username.
pub(crate) async fn load_user_identity(
    state: &AppState,
    user_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    let user = crate::entity::user::Entity::find_by_id(user_id.to_string())
        .one(&state.db)
        .await
        .ok()
        .flatten()?;
    let non_empty = |value: Option<String>| value.filter(|s| !s.trim().is_empty());
    let name = non_empty(user.name)
        .or_else(|| non_empty(user.username))
        .or_else(|| non_empty(user.preferred_username));
    Some((name, non_empty(user.email)))
}

/// Full contact card for the responsible person, surfaced to admins so they can
/// reach out to the accountable owner from the governance inventory.
#[derive(Clone, Serialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResponsiblePerson {
    pub user_id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub avatar: Option<String>,
    pub description: Option<String>,
}

/// Load the full contact card for a user id (best effort).
pub(crate) async fn load_user_contact(
    state: &AppState,
    user_id: &str,
) -> Option<ResponsiblePerson> {
    let user = crate::entity::user::Entity::find_by_id(user_id.to_string())
        .one(&state.db)
        .await
        .ok()
        .flatten()?;
    let non_empty = |value: Option<String>| value.filter(|s| !s.trim().is_empty());
    let name = non_empty(user.name.clone())
        .or_else(|| non_empty(user.username.clone()))
        .or_else(|| non_empty(user.preferred_username.clone()));
    Some(ResponsiblePerson {
        user_id: user.id,
        name,
        email: non_empty(user.email),
        username: non_empty(user.username).or_else(|| non_empty(user.preferred_username)),
        avatar: non_empty(user.avatar),
        description: non_empty(user.description),
    })
}

/// Find the owner user id for an app — the first member whose role carries the
/// `Owner` permission bit (best effort). Used to default the responsible person.
pub(crate) async fn resolve_app_owner(state: &AppState, app_id: &str) -> Option<String> {
    let owner_role_ids: Vec<String> = crate::entity::role::Entity::find()
        .filter(crate::entity::role::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|role| {
            RolePermissions::from_bits_truncate(role.permissions).contains(RolePermissions::Owner)
        })
        .map(|role| role.id)
        .collect();
    if owner_role_ids.is_empty() {
        return None;
    }
    crate::entity::membership::Entity::find()
        .filter(crate::entity::membership::Column::AppId.eq(app_id))
        .filter(crate::entity::membership::Column::RoleId.is_in(owner_role_ids))
        .order_by_asc(crate::entity::membership::Column::CreatedAt)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|membership| membership.user_id)
}

/// Resolve the canonical responsible person for an app — always the app owner
/// (falling back to the acting user when no owner membership is found). Returns
/// the resolved user id plus a best-effort display name and email so the
/// responsible person is hard-linked to ownership and never hand-edited.
pub(crate) async fn resolve_responsible_person(
    state: &AppState,
    app_id: &str,
    fallback_user_id: &str,
) -> (String, Option<String>, Option<String>) {
    let user_id = resolve_app_owner(state, app_id)
        .await
        .unwrap_or_else(|| fallback_user_id.to_string());
    let (name, email) = load_user_identity(state, &user_id)
        .await
        .unwrap_or((None, None));
    (user_id, name, email)
}

/// Map the classifier risk category to the persisted entity enum.
fn to_entity_risk(category: RiskCategory) -> crate::entity::sea_orm_active_enums::AiRiskCategory {
    use crate::entity::sea_orm_active_enums::AiRiskCategory as E;
    match category {
        RiskCategory::Prohibited => E::Prohibited,
        RiskCategory::High => E::High,
        RiskCategory::Limited => E::Limited,
        RiskCategory::Minimal => E::Minimal,
        RiskCategory::Undetermined => E::Undetermined,
    }
}

// ---------------------------------------------------------------------------
// GET /questionnaire
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuestionnaireResponse {
    /// Canonical questionnaire schema (serialised as JSON).
    pub schema: serde_json::Value,
    /// Auto-derived signals from the static board scan (serialised as JSON).
    pub signals: serde_json::Value,
    /// Existing stored answers, if an assessment exists.
    pub answers: serde_json::Value,
    /// Live classification preview for the current answers (serialised JSON).
    pub classification: serde_json::Value,
    /// Prioritised, weighted tips for improving the conformity score — the same
    /// list the platform admin sees in the governance inventory.
    pub recommendations: serde_json::Value,
    /// Hard-linked responsible person (the app owner). Surfaced read-only so the
    /// owner and admin always see the same accountable contact.
    pub responsible_name: Option<String>,
    /// Hard-linked responsible person email (the app owner).
    pub responsible_email: Option<String>,
    /// Whether a prior assessment exists for this app.
    pub has_assessment: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/ai-act/questionnaire",
    tag = "ai-act",
    description = "Get the EU AI Act questionnaire schema, auto-derived board signals and a live classification preview. Requires owner permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Questionnaire schema and signals", body = QuestionnaireResponse),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/ai-act/questionnaire", skip(state, user))]
pub async fn get_questionnaire(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<QuestionnaireResponse>, ApiError> {
    ensure_feature(&state)?;
    let permission = crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;
    let signals = board_scan::scan_app_signals(&state, &sub, &app_id, &app)
        .await
        .unwrap_or_default();

    let existing = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    let answers: Value = existing
        .as_ref()
        .map(|a| a.answers.clone())
        .unwrap_or_else(|| prefill_answers(&signals));

    let classification = classify(&answers, &signals);
    let recommendations =
        questionnaire::recommendations(&answers, &signals, &classification);
    let schema: QuestionnaireSchema = questionnaire_schema();

    // The responsible person is hard-linked to the app owner. Prefer the stored
    // contact when present, otherwise resolve it live so the field is never
    // empty and always matches the admin governance view.
    let (responsible_name, responsible_email) = match existing
        .as_ref()
        .and_then(|a| a.responsible_name.clone().map(|n| (Some(n), a.responsible_email.clone())))
    {
        Some(pair) => pair,
        None => {
            let (_, name, email) = resolve_responsible_person(&state, &app_id, &sub).await;
            (name, email)
        }
    };

    Ok(Json(QuestionnaireResponse {
        schema: serde_json::to_value(&schema).unwrap_or(Value::Null),
        signals: serde_json::to_value(&signals).unwrap_or(Value::Null),
        answers,
        classification: serde_json::to_value(&classification).unwrap_or(Value::Null),
        recommendations: serde_json::to_value(&recommendations)
            .unwrap_or(Value::Array(Vec::new())),
        responsible_name,
        responsible_email,
        has_assessment: existing.is_some(),
    }))
}

/// Build a conservative prefilled answer set from the auto-derived signals so
/// the owner confirms rather than types. Pivotal/prohibited questions are left
/// blank — they must be answered explicitly.
pub(crate) fn prefill_answers(signals: &Signals) -> Value {
    use questionnaire::keys;
    let mut map = serde_json::Map::new();
    let yn = |b: bool| {
        Value::String(if b {
            "yes".to_string()
        } else {
            "no".to_string()
        })
    };

    map.insert(
        keys::CHATBOT.to_string(),
        yn(signals.capabilities.has_chatbot),
    );
    map.insert(keys::GENAI.to_string(), yn(signals.capabilities.has_genai));
    map.insert(
        keys::EMOTION_BIOMETRIC.to_string(),
        yn(signals.capabilities.has_emotion_biometric),
    );
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// POST /classify  (non-persisting live preview)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyBody {
    /// Current questionnaire answers to classify.
    pub answers: serde_json::Value,
}

#[derive(Clone, Serialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResponse {
    /// Authoritative classification for the supplied answers (serialised JSON).
    pub classification: serde_json::Value,
    /// Prioritised improvement tips for the supplied answers, so the wizard can
    /// update the "how to improve" guidance live as the owner edits answers.
    pub recommendations: serde_json::Value,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/ai-act/classify",
    tag = "ai-act",
    description = "Run the deterministic classifier over the supplied answers without persisting. Used by the publishing wizard for a live, authoritative preview. Requires owner permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ClassifyBody,
    responses(
        (status = 200, description = "Live classification", body = ClassifyResponse),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/ai-act/classify", skip(state, user, body))]
pub async fn classify_preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<ClassifyBody>,
) -> Result<Json<ClassifyResponse>, ApiError> {
    ensure_feature(&state)?;
    let permission = crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;
    let signals = board_scan::scan_app_signals(&state, &sub, &app_id, &app)
        .await
        .unwrap_or_default();

    let classification = classify(&body.answers, &signals);
    let recommendations =
        questionnaire::recommendations(&body.answers, &signals, &classification);
    Ok(Json(ClassifyResponse {
        classification: serde_json::to_value(&classification).unwrap_or(Value::Null),
        recommendations: serde_json::to_value(&recommendations)
            .unwrap_or(Value::Array(Vec::new())),
    }))
}

// ---------------------------------------------------------------------------
// GET /assessment
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentResponse {
    pub id: String,
    pub app_id: String,
    pub version: i32,
    pub status: String,
    pub risk_category: String,
    pub conformity_score: Option<i32>,
    pub conformity_band: Option<String>,
    pub answers: serde_json::Value,
    pub signals: Option<serde_json::Value>,
    pub transparency_obligations: Option<serde_json::Value>,
    pub responsible_name: Option<String>,
    pub responsible_email: Option<String>,
    pub submitted_at: Option<String>,
    pub reviewed_at: Option<String>,
    pub review_note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ai_act_assessment::Model> for AssessmentResponse {
    fn from(m: ai_act_assessment::Model) -> Self {
        AssessmentResponse {
            id: m.id,
            app_id: m.app_id,
            version: m.version,
            status: format!("{:?}", m.status).to_uppercase(),
            risk_category: format!("{:?}", m.risk_category).to_uppercase(),
            conformity_score: m.conformity_score,
            conformity_band: m.conformity_band,
            answers: m.answers,
            signals: m.signals,
            transparency_obligations: m.transparency_obligations,
            responsible_name: m.responsible_name,
            responsible_email: m.responsible_email,
            submitted_at: m.submitted_at.map(|d| d.to_string()),
            reviewed_at: m.reviewed_at.map(|d| d.to_string()),
            review_note: m.review_note,
            created_at: m.created_at.to_string(),
            updated_at: m.updated_at.to_string(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/ai-act/assessment",
    tag = "ai-act",
    description = "Get the latest EU AI Act assessment for an app. Requires owner permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Latest assessment (or null)", body = Option<AssessmentResponse>),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/ai-act/assessment", skip(state, user))]
pub async fn get_assessment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Option<AssessmentResponse>>, ApiError> {
    ensure_feature(&state)?;
    crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let existing = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    Ok(Json(existing.map(AssessmentResponse::from)))
}

// ---------------------------------------------------------------------------
// PUT /assessment
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertAssessmentBody {
    pub answers: serde_json::Value,
    /// When true the assessment is marked SUBMITTED (ready for review).
    #[serde(default)]
    pub submit: bool,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/ai-act/assessment",
    tag = "ai-act",
    description = "Create or update the EU AI Act assessment. The risk category and score are always recomputed server-side. Prohibited practices return a BLOCKED assessment. Requires owner permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = UpsertAssessmentBody,
    responses(
        (status = 200, description = "Stored assessment", body = AssessmentResponse),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/ai-act/assessment", skip(state, user, body))]
pub async fn put_assessment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpsertAssessmentBody>,
) -> Result<Json<AssessmentResponse>, ApiError> {
    ensure_feature(&state)?;
    let permission = crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;

    let app = state.master_app(&sub, &app_id, &state).await?;
    let signals = board_scan::scan_app_signals(&state, &sub, &app_id, &app)
        .await
        .unwrap_or_default();

    let classification = classify(&body.answers, &signals);
    let now = chrono::Utc::now().naive_utc();

    // Determine status: prohibited -> BLOCKED; submit -> SUBMITTED; else DRAFT.
    let status = if classification.blocked {
        AiActAssessmentStatus::Blocked
    } else if body.submit {
        AiActAssessmentStatus::Submitted
    } else {
        AiActAssessmentStatus::Draft
    };

    let signals_json = serde_json::to_value(&signals).ok();
    let obligations_json = serde_json::to_value(&classification.transparency_obligations).ok();
    let conformity_band = classification
        .conformity_band
        .map(|b| b.as_str().to_string());

    // The responsible person is hard-linked to the app owner — it is never
    // taken from the request body, so it cannot be changed by the client
    // (Regulation (EU) 2024/1689, Art. 26 accountability).
    let (responsible_user_id, responsible_name, responsible_email) =
        resolve_responsible_person(&state, &app_id, &sub).await;

    let existing = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(&app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    let submitted_at = if matches!(status, AiActAssessmentStatus::Submitted) {
        Some(now)
    } else {
        None
    };

    let stored = if let Some(existing) = existing {
        // Update the latest assessment in place (versioning bumps on review).
        let mut active: ai_act_assessment::ActiveModel = existing.into();
        active.status = Set(status);
        active.risk_category = Set(to_entity_risk(classification.risk_category));
        active.conformity_score = Set(classification.conformity_score);
        active.conformity_band = Set(conformity_band);
        active.answers = Set(body.answers.clone());
        active.signals = Set(signals_json);
        active.transparency_obligations = Set(obligations_json);
        active.responsible_user_id = Set(Some(responsible_user_id.clone()));
        active.responsible_name = Set(responsible_name.clone());
        active.responsible_email = Set(responsible_email.clone());
        if submitted_at.is_some() {
            active.submitted_at = Set(submitted_at);
        }
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = ai_act_assessment::ActiveModel {
            id: Set(create_id()),
            app_id: Set(app_id.clone()),
            version: Set(1),
            status: Set(status),
            risk_category: Set(to_entity_risk(classification.risk_category)),
            conformity_score: Set(classification.conformity_score),
            conformity_band: Set(conformity_band),
            answers: Set(body.answers.clone()),
            signals: Set(signals_json),
            transparency_obligations: Set(obligations_json),
            responsible_user_id: Set(Some(responsible_user_id.clone())),
            responsible_name: Set(responsible_name.clone()),
            responsible_email: Set(responsible_email.clone()),
            submitted_at: Set(submitted_at),
            reviewed_by_id: Set(None),
            reviewed_at: Set(None),
            review_note: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(AssessmentResponse::from(stored)))
}

// ---------------------------------------------------------------------------
// POST /assessment/suggest  (governance FlowPilot agent)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestBody {
    /// Optional model id override; defaults to the platform copilot model.
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Clone, Serialize, Debug, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestResponse {
    /// Governance agent suggestion (serialised JSON).
    pub suggestion: serde_json::Value,
    /// The signals the suggestion was based on (serialised JSON).
    pub signals: serde_json::Value,
    /// The model that produced the suggestion (e.g. "gpt-4o").
    pub model: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/ai-act/assessment/suggest",
    tag = "ai-act",
    description = "Run the governance FlowPilot agent over the app's boards (in their FlowScript state) to propose questionnaire answers. Read-only. Requires owner permission.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = SuggestBody,
    responses(
        (status = 200, description = "Suggested answers", body = SuggestResponse),
        (status = 400, description = "Feature disabled"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/ai-act/assessment/suggest",
    skip(state, user, body)
)]
pub async fn suggest_assessment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<SuggestBody>,
) -> Result<Json<SuggestResponse>, ApiError> {
    ensure_feature(&state)?;
    let permission = crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;

    let (suggestion, signals, model) =
        crate::routes::ai::governance::run_governance_agent(&state, &sub, &app_id, body.model_id)
            .await?;

    Ok(Json(SuggestResponse {
        suggestion: serde_json::to_value(&suggestion).unwrap_or(Value::Null),
        signals: serde_json::to_value(&signals).unwrap_or(Value::Null),
        model,
    }))
}

/// Re-export of the live classification preview helper for the publishing flow.
#[allow(unused_imports)]
pub use questionnaire::classify as classify_assessment;
#[allow(unused_imports)]
pub use signals::Signals as AiActSignals;
pub type AiActClassification = Classification;
