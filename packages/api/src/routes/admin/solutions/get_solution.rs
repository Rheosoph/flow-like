use crate::{
    error::ApiError, middleware::jwt::AppUser, permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SolutionDetail {
    pub id: String,
    pub name: String,
    pub email: String,
    pub company: String,
    pub description: String,
    pub application_type: String,
    pub data_security: String,
    pub example_input: String,
    pub expected_output: String,
    pub user_count: String,
    pub user_type: String,
    pub technical_level: String,
    pub timeline: Option<String>,
    pub additional_notes: Option<String>,
    pub pricing_tier: String,
    pub paid_deposit: bool,
    pub files: Option<serde_json::Value>,
    pub storage_key: Option<String>,
    pub status: String,
    pub stripe_checkout_session_id: Option<String>,
    pub stripe_payment_intent_id: Option<String>,
    pub stripe_setup_intent_id: Option<String>,
    pub total_cents: i64,
    pub deposit_cents: i64,
    pub remainder_cents: i64,
    pub priority: bool,
    pub admin_notes: Option<String>,
    pub assigned_to: Option<String>,
    pub delivered_at: Option<String>,
    pub tracking_token: String,
    pub logs: Vec<SolutionLogItem>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SolutionLogItem {
    pub id: String,
    pub action: String,
    pub details: Option<String>,
    pub actor: Option<String>,
    pub created_at: String,
}

#[tracing::instrument(name = "GET /admin/solutions/{solution_id}", skip(state, user))]
pub async fn get_solution(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(solution_id): Path<String>,
) -> Result<Json<SolutionDetail>, ApiError> {
    use crate::entity::{solution_log, solution_request};

    user.check_global_permission(&state, GlobalPermission::ReadSolutions)
        .await?;

    let solution = solution_request::Entity::find_by_id(&solution_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("Solution request not found"))?;

    let logs = solution_log::Entity::find()
        .filter(solution_log::Column::SolutionId.eq(&solution_id))
        .order_by_desc(solution_log::Column::CreatedAt)
        .all(&state.db)
        .await?
        .into_iter()
        .map(|log| SolutionLogItem {
            id: log.id,
            action: log.action,
            details: log.details,
            actor: log.actor,
            created_at: log.created_at.to_rfc3339(),
        })
        .collect();

    let detail = SolutionDetail {
        id: solution.id,
        name: solution.name,
        email: solution.email,
        company: solution.company,
        description: solution.description,
        application_type: solution.application_type,
        data_security: solution.data_security,
        example_input: solution.example_input,
        expected_output: solution.expected_output,
        user_count: solution.user_count,
        user_type: solution.user_type,
        technical_level: solution.technical_level,
        timeline: solution.timeline,
        additional_notes: solution.additional_notes,
        pricing_tier: format!("{:?}", solution.pricing_tier).to_lowercase(),
        paid_deposit: solution.paid_deposit,
        files: solution.files,
        storage_key: solution.storage_key,
        status: status_to_string(&solution.status),
        stripe_checkout_session_id: solution.stripe_checkout_session_id,
        stripe_payment_intent_id: solution.stripe_payment_intent_id,
        stripe_setup_intent_id: solution.stripe_setup_intent_id,
        total_cents: solution.total_cents,
        deposit_cents: solution.deposit_cents,
        remainder_cents: solution.remainder_cents,
        priority: solution.priority,
        admin_notes: solution.admin_notes,
        assigned_to: solution.assigned_to,
        delivered_at: solution.delivered_at.map(|d| d.to_rfc3339()),
        tracking_token: solution.tracking_token,
        logs,
        created_at: solution.created_at.to_rfc3339(),
        updated_at: solution.updated_at.to_rfc3339(),
    };

    Ok(Json(detail))
}

fn status_to_string(status: &crate::entity::sea_orm_active_enums::SolutionStatus) -> String {
    use crate::entity::sea_orm_active_enums::SolutionStatus;
    match status {
        SolutionStatus::AwaitingDeposit => "AWAITING_DEPOSIT".to_string(),
        SolutionStatus::PendingReview => "PENDING_REVIEW".to_string(),
        SolutionStatus::InQueue => "IN_QUEUE".to_string(),
        SolutionStatus::OnboardingDone => "ONBOARDING_DONE".to_string(),
        SolutionStatus::InProgress => "IN_PROGRESS".to_string(),
        SolutionStatus::Delivered => "DELIVERED".to_string(),
        SolutionStatus::AwaitingPayment => "AWAITING_PAYMENT".to_string(),
        SolutionStatus::Paid => "PAID".to_string(),
        SolutionStatus::Cancelled => "CANCELLED".to_string(),
        SolutionStatus::Refunded => "REFUNDED".to_string(),
    }
}
