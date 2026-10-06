use crate::{auth::AppUser, error::ApiError, state::AppState};
use axum::{
    Extension, Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SolutionUploadResponse {
    pub key: String,
    pub content_type: String,
    pub upload_url: String,
    pub upload_expires_at: String,
    pub download_url: String,
    pub download_expires_at: String,
    pub size_limit_bytes: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SolutionSubmission {
    pub name: String,
    pub email: String,
    pub company: String,
    pub application_type: String,
    pub data_security: String,
    pub description: String,
    pub example_input: String,
    pub expected_output: String,
    pub files: Vec<UploadedFile>,
    pub user_count: String,
    pub user_type: String,
    pub technical_level: String,
    pub timeline: Option<String>,
    pub additional_notes: Option<String>,
    pub pricing_tier: String,
    pub pay_deposit: bool,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub name: String,
    pub key: String,
    pub download_url: String,
    pub size: u64,
}

#[derive(Deserialize, Debug, ToSchema, utoipa::IntoParams)]
pub struct UploadParams {
    pub extension: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionResponse {
    pub success: bool,
    pub id: String,
    pub tracking_token: String,
    pub message: String,
    pub checkout_url: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/upload", get(get_upload_url))
        .route("/", post(submit_solution))
        .route("/track/{token}", get(track_solution))
}

/// New requests are closed; existing submissions remain available through tracking.
#[utoipa::path(
    get,
    path = "/solution/upload",
    tag = "solution",
    responses((status = 410, description = "Solution intake is no longer available"))
)]
#[tracing::instrument(name = "GET /solution/upload", skip_all)]
pub async fn get_upload_url() -> Result<Json<SolutionUploadResponse>, ApiError> {
    Err(ApiError::gone("Solution intake is no longer available"))
}

#[utoipa::path(
    post,
    path = "/solution",
    tag = "solution",
    responses((status = 410, description = "Solution intake is no longer available"))
)]
#[tracing::instrument(name = "POST /solution", skip_all)]
pub async fn submit_solution() -> Result<Json<SubmissionResponse>, ApiError> {
    Err(ApiError::gone("Solution intake is no longer available"))
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicSolutionStatus {
    pub id: String,
    pub company: String,
    pub status: String,
    pub status_label: String,
    pub status_description: String,
    pub paid_deposit: bool,
    pub priority: bool,
    pub pricing_tier: String,
    pub total_cents: i64,
    pub deposit_cents: i64,
    pub remainder_cents: i64,
    pub delivered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub logs: Vec<PublicSolutionLog>,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicSolutionLog {
    pub action: String,
    pub created_at: String,
}

fn get_status_label(status: &str) -> String {
    match status {
        "AWAITING_DEPOSIT" => "Awaiting Deposit".to_string(),
        "PENDING_REVIEW" => "Pending Review".to_string(),
        "IN_QUEUE" => "In Queue".to_string(),
        "ONBOARDING_DONE" => "Onboarding Done".to_string(),
        "IN_PROGRESS" => "In Progress".to_string(),
        "DELIVERED" => "Delivered".to_string(),
        "AWAITING_PAYMENT" => "Awaiting Payment".to_string(),
        "PAID" => "Paid".to_string(),
        "CANCELLED" => "Cancelled".to_string(),
        "REFUNDED" => "Refunded".to_string(),
        _ => status.to_string(),
    }
}

fn get_status_description(status: &str) -> String {
    match status {
        "AWAITING_DEPOSIT" => "Awaiting priority deposit payment".to_string(),
        "PENDING_REVIEW" => "Your request has been submitted and is pending review".to_string(),
        "IN_QUEUE" => "Your request has been approved and is in the queue".to_string(),
        "ONBOARDING_DONE" => "Onboarding has been completed".to_string(),
        "IN_PROGRESS" => "Your solution is actively being worked on".to_string(),
        "DELIVERED" => "Your solution has been delivered".to_string(),
        "AWAITING_PAYMENT" => "Awaiting final payment".to_string(),
        "PAID" => "Payment completed. Thank you!".to_string(),
        "CANCELLED" => "This request has been cancelled".to_string(),
        "REFUNDED" => "Payment has been refunded".to_string(),
        _ => "Status unknown".to_string(),
    }
}

#[utoipa::path(
    get,
    path = "/solution/track/{token}",
    tag = "solution",
    params(
        ("token" = String, Path, description = "Tracking token for the solution request")
    ),
    responses(
        (status = 200, description = "Solution status retrieved successfully", body = PublicSolutionStatus),
        (status = 404, description = "Solution not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[tracing::instrument(name = "GET /solution/track/{token}", skip_all)]
pub async fn track_solution(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> Result<Json<PublicSolutionStatus>, ApiError> {
    use crate::entity::{solution_log, solution_request};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    user.sub()?;
    let solution = solution_request::Entity::find()
        .filter(solution_request::Column::TrackingToken.eq(&token))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let logs = solution_log::Entity::find()
        .filter(solution_log::Column::SolutionId.eq(&solution.id))
        .order_by_desc(solution_log::Column::CreatedAt)
        .all(&state.db)
        .await?
        .into_iter()
        .map(|log| PublicSolutionLog {
            action: log.action,
            created_at: log.created_at.to_rfc3339(),
        })
        .collect();

    let status_str = status_to_string(&solution.status);

    Ok(Json(PublicSolutionStatus {
        id: solution.id,
        company: solution.company,
        status: status_str.clone(),
        status_label: get_status_label(&status_str),
        status_description: get_status_description(&status_str),
        paid_deposit: solution.paid_deposit,
        priority: solution.priority,
        pricing_tier: format!("{:?}", solution.pricing_tier).to_lowercase(),
        total_cents: solution.total_cents,
        deposit_cents: solution.deposit_cents,
        remainder_cents: solution.remainder_cents,
        delivered_at: solution
            .delivered_at
            .map(|d: chrono::DateTime<chrono::FixedOffset>| d.to_rfc3339()),
        created_at: solution.created_at.to_rfc3339(),
        updated_at: solution.updated_at.to_rfc3339(),
        logs,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn retired_intake_needs_no_authentication_or_service_state() {
        let router = Router::new()
            .route("/solution/upload", get(get_upload_url))
            .route("/solution", post(submit_solution));
        for (method, path, body) in [
            ("GET", "/solution/upload?extension=pdf", ""),
            ("POST", "/solution", "not JSON"),
            ("POST", "/solution", "{}"),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::GONE);
        }
    }
}
