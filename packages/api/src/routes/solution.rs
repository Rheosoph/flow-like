use crate::alerting;
use crate::error::ApiError;
use crate::mail::{EmailMessage, templates};
use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use flow_like_storage::Path as FLPath;
use flow_like_types::tokio::try_join;
use flow_like_types::{
    create_id,
    mime_guess::{self, mime},
};
use mime::Mime;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

const UPLOAD_TTL_SECS: u64 = 60 * 60; // 1 hour
const DOWNLOAD_TTL_SECS: u64 = 60 * 60 * 24 * 7; // 7 days
const SIZE_LIMIT_BYTES: Option<u64> = Some(1024 * 1024 * 50); // 50 MB

const DEPOSIT_AMOUNT_CENTS: i64 = 50000; // €500 deposit for priority queue
const STANDARD_TOTAL_CENTS: i64 = 240000; // €2,400 total
const APPSTORE_TOTAL_CENTS: i64 = 199900; // €1,999 total

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

#[derive(Clone, Serialize, Debug, ToSchema)]
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

#[utoipa::path(
    get,
    path = "/solution/upload",
    tag = "solution",
    params(UploadParams),
    responses(
        (status = 200, description = "Presigned upload URL generated successfully", body = SolutionUploadResponse),
        (status = 500, description = "Internal server error")
    )
)]
#[tracing::instrument(name = "GET /solution/upload", skip_all)]
pub async fn get_upload_url(
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
) -> Result<Json<SolutionUploadResponse>, ApiError> {
    let id = create_id();
    let ext = sanitize_ext(params.extension.as_deref()).unwrap_or_else(|| "bin".to_string());

    let now_utc = Utc::now();
    let date_prefix = now_utc.format("%Y/%m/%d").to_string();
    let file_name = format!("{id}.{ext}");
    let key = format!("solution-requests/{date_prefix}/{file_name}");

    let content_type: Mime = params
        .content_type
        .as_deref()
        .and_then(|s| s.parse::<Mime>().ok())
        .or_else(|| mime_guess::from_ext(&ext).first())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);

    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let path = FLPath::from(key.clone());

    let (download_url, upload_url) = try_join!(
        store.sign("GET", &path, Duration::from_secs(DOWNLOAD_TTL_SECS)),
        store.sign("PUT", &path, Duration::from_secs(UPLOAD_TTL_SECS)),
    )?;

    let download_expires_at =
        (now_utc + ChronoDuration::seconds(DOWNLOAD_TTL_SECS as i64)).to_rfc3339();
    let upload_expires_at =
        (now_utc + ChronoDuration::seconds(UPLOAD_TTL_SECS as i64)).to_rfc3339();

    let response = SolutionUploadResponse {
        key,
        content_type: content_type.to_string(),
        upload_url: upload_url.to_string(),
        upload_expires_at,
        download_url: download_url.to_string(),
        download_expires_at,
        size_limit_bytes: SIZE_LIMIT_BYTES,
    };

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/solution",
    tag = "solution",
    request_body = SolutionSubmission,
    responses(
        (status = 200, description = "Solution submitted successfully", body = SubmissionResponse),
        (status = 400, description = "Invalid submission data"),
        (status = 500, description = "Internal server error")
    )
)]
#[tracing::instrument(name = "POST /solution", skip(state, submission))]
pub async fn submit_solution(
    State(state): State<AppState>,
    Json(submission): Json<SolutionSubmission>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    use crate::entity::{
        sea_orm_active_enums::{SolutionPricingTier, SolutionStatus},
        solution_request,
    };

    if submission.name.trim().is_empty() {
        return Err(ApiError::bad_request("Name is required".to_string()));
    }
    if submission.email.trim().is_empty() || !submission.email.contains('@') {
        return Err(ApiError::bad_request("Valid email is required".to_string()));
    }
    if submission.company.trim().is_empty() {
        return Err(ApiError::bad_request("Company is required".to_string()));
    }
    if submission.description.len() < 50 {
        return Err(ApiError::bad_request(
            "Description must be at least 50 characters".to_string(),
        ));
    }
    if submission.example_input.len() < 20 {
        return Err(ApiError::bad_request(
            "Example input must be at least 20 characters".to_string(),
        ));
    }
    if submission.expected_output.len() < 20 {
        return Err(ApiError::bad_request(
            "Expected output must be at least 20 characters".to_string(),
        ));
    }

    let (tier_name, total_cents, pricing_tier_enum) = match submission.pricing_tier.as_str() {
        "standard" => (
            "24 Hour Solution - Standard",
            STANDARD_TOTAL_CENTS,
            SolutionPricingTier::Standard,
        ),
        "appstore" => (
            "24 Hour Solution - App Store",
            APPSTORE_TOTAL_CENTS,
            SolutionPricingTier::Appstore,
        ),
        _ => {
            return Err(ApiError::bad_request(
                "Invalid pricing tier. Must be 'standard' or 'appstore'".to_string(),
            ));
        }
    };

    let deposit_cents = if submission.pay_deposit {
        DEPOSIT_AMOUNT_CENTS
    } else {
        0
    };
    let remainder_cents = total_cents - deposit_cents;

    let submission_id = create_id();
    let tracking_token = create_id();
    let now_utc = Utc::now();
    let date_prefix = now_utc.format("%Y/%m/%d").to_string();

    let submission_data = serde_json::to_string_pretty(&submission)?;
    let key = format!("solution-requests/{date_prefix}/submissions/{submission_id}.json");

    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let path = FLPath::from(key.clone());

    store
        .as_generic()
        .put(&path, submission_data.into())
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to store submission: {}", e)))?;

    let files_json = serde_json::to_value(&submission.files).ok();

    // If paying deposit (priority), start with AwaitingDeposit
    // If not paying deposit, start directly with PendingReview
    let initial_status = if submission.pay_deposit {
        SolutionStatus::AwaitingDeposit
    } else {
        SolutionStatus::PendingReview
    };

    let new_solution = solution_request::ActiveModel {
        id: Set(submission_id.clone()),
        name: Set(submission.name.clone()),
        email: Set(submission.email.clone()),
        company: Set(submission.company.clone()),
        description: Set(submission.description.clone()),
        application_type: Set(submission.application_type.clone()),
        data_security: Set(submission.data_security.clone()),
        example_input: Set(submission.example_input.clone()),
        expected_output: Set(submission.expected_output.clone()),
        user_count: Set(submission.user_count.clone()),
        user_type: Set(submission.user_type.clone()),
        technical_level: Set(submission.technical_level.clone()),
        timeline: Set(submission.timeline.clone()),
        additional_notes: Set(submission.additional_notes.clone()),
        pricing_tier: Set(pricing_tier_enum),
        paid_deposit: Set(false),
        files: Set(files_json),
        storage_key: Set(Some(key.clone())),
        status: Set(initial_status),
        stripe_checkout_session_id: Set(None),
        stripe_payment_intent_id: Set(None),
        stripe_setup_intent_id: Set(None),
        total_cents: Set(total_cents),
        deposit_cents: Set(deposit_cents),
        remainder_cents: Set(remainder_cents),
        priority: Set(submission.pay_deposit),
        admin_notes: Set(None),
        assigned_to: Set(None),
        delivered_at: Set(None),
        tracking_token: Set(tracking_token.clone()),
        created_at: Set(now_utc.naive_utc()),
        updated_at: Set(now_utc.naive_utc()),
    };

    new_solution.insert(&state.db).await?;
    alerting::send_alert_email(
        &state,
        "New 24-Hour Solution Request",
        format!(
            "A new 24-hour solution request has been submitted by {} ({}).",
            submission.company, submission.email
        ),
    )
    .await
    .ok();

    tracing::info!(
        submission_id = %submission_id,
        email = %submission.email,
        company = %submission.company,
        pricing_tier = %submission.pricing_tier,
        pay_deposit = %submission.pay_deposit,
        "New 24-hour solution request submitted and stored in database"
    );

    let frontend_url = std::env::var("FRONTEND_URL").unwrap_or_else(|_| {
        format!(
            "https://{}",
            state
                .platform_config
                .web
                .clone()
                .unwrap_or_else(|| state.platform_config.domain.clone())
        )
    });

    let tracking_url = format!("{}/track?token={}", frontend_url, tracking_token);

    if let Some(mail_client) = &state.mail_client {
        let (html, text) = templates::solution_submission_confirmation(
            &submission.company,
            &tracking_url,
            &tracking_token,
            &submission.pricing_tier,
            submission.pay_deposit,
        );

        let email = EmailMessage {
            to: submission.email.clone(),
            subject: format!(
                "Your 24-Hour Solution Request for {} – Confirmed!",
                submission.company
            ),
            body_html: Some(html),
            body_text: Some(text),
        };

        if let Err(e) = mail_client.send(email).await {
            tracing::warn!(
                error = %e,
                email = %submission.email,
                "Failed to send confirmation email, but submission was successful"
            );
        }
    }

    let checkout_url = if let Some(stripe_client) = state.stripe_client.as_ref() {
        let success_url = format!("{}/track?token={}", frontend_url, tracking_token);
        let cancel_url = format!("{}/24-hour-solution?canceled=true", frontend_url);

        let total_display = format!("€{:.2}", total_cents as f64 / 100.0);

        // Build metadata
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("submission_id".to_string(), submission_id.clone());
        metadata.insert("company".to_string(), submission.company.clone());
        metadata.insert("pricing_tier".to_string(), submission.pricing_tier.clone());
        metadata.insert("storage_key".to_string(), key.clone());
        metadata.insert("total_cents".to_string(), total_cents.to_string());
        metadata.insert("tracking_token".to_string(), tracking_token.clone());

        let mut params = stripe::CreateCheckoutSession::new();
        params.success_url = Some(&success_url);
        params.cancel_url = Some(&cancel_url);
        params.customer_email = Some(&submission.email);
        params.client_reference_id = Some(&submission_id);

        if submission.pay_deposit {
            // Payment mode: Charge €500 deposit
            params.mode = Some(stripe::CheckoutSessionMode::Payment);

            let remainder_cents = total_cents - DEPOSIT_AMOUNT_CENTS;
            let remainder_display = format!("€{:.2}", remainder_cents as f64 / 100.0);

            let line_item = stripe::CreateCheckoutSessionLineItems {
                price_data: Some(stripe::CreateCheckoutSessionLineItemsPriceData {
                    currency: stripe::Currency::EUR,
                    product_data: Some(
                        stripe::CreateCheckoutSessionLineItemsPriceDataProductData {
                            name: format!("{} (Priority Deposit)", tier_name),
                            description: Some(format!(
                                "Priority deposit for {} | Total: {} | Remaining {} invoiced after delivery",
                                submission.company, total_display, remainder_display
                            )),
                            ..Default::default()
                        },
                    ),
                    unit_amount: Some(DEPOSIT_AMOUNT_CENTS),
                    ..Default::default()
                }),
                quantity: Some(1),
                ..Default::default()
            };
            params.line_items = Some(vec![line_item]);

            metadata.insert(
                "deposit_cents".to_string(),
                DEPOSIT_AMOUNT_CENTS.to_string(),
            );
            metadata.insert("remainder_cents".to_string(), remainder_cents.to_string());
            metadata.insert("priority".to_string(), "true".to_string());
        } else {
            // No deposit - skip Stripe checkout entirely
            // The solution is already saved with PendingReview status
            return Ok(Json(SubmissionResponse {
                success: true,
                id: submission_id,
                tracking_token,
                message: "Your request has been submitted successfully. We'll review and get back to you shortly.".to_string(),
                checkout_url: None,
            }));
        }

        params.metadata = Some(metadata);

        match stripe::CheckoutSession::create(stripe_client, params).await {
            Ok(session) => session.url,
            Err(e) => {
                tracing::error!(error = %e, "Failed to create Stripe checkout session");
                None
            }
        }
    } else {
        tracing::warn!("Stripe client not configured, skipping checkout session creation");
        None
    };

    Ok(Json(SubmissionResponse {
        success: true,
        id: submission_id,
        tracking_token,
        message: if checkout_url.is_some() {
            if submission.pay_deposit {
                "Redirecting to payment...".to_string()
            } else {
                "Redirecting to confirm your request...".to_string()
            }
        } else {
            "Your request has been submitted successfully. We'll review and get back to you within 48 hours.".to_string()
        },
        checkout_url,
    }))
}

fn sanitize_ext(input: Option<&str>) -> Option<String> {
    let mut s = input?.trim().trim_start_matches('.').to_ascii_lowercase();
    if s.is_empty() || s.len() > 16 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(std::mem::take(&mut s))
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
    axum::extract::Path(token): axum::extract::Path<String>,
) -> Result<Json<PublicSolutionStatus>, ApiError> {
    use crate::entity::{solution_log, solution_request};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

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
            created_at: log.created_at.to_string(),
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
            .map(|d: chrono::NaiveDateTime| d.to_string()),
        created_at: solution.created_at.to_string(),
        updated_at: solution.updated_at.to_string(),
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
