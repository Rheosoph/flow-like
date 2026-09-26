use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    audit::export, audit_branch, ensure_permission, entity::audit_export_target, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
    utils::crypto::encrypt_secret,
};

const MAX_URL_LENGTH: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditWebhookView {
    pub url: String,
    /// Deliveries stop after 50 consecutive failures; saving the webhook resumes them.
    pub active: bool,
    /// Consecutive failed deliveries.
    pub failures: i32,
    /// HTTP status or error class of the last failed delivery.
    pub last_error: Option<String>,
    pub last_delivered_at_ms: Option<i64>,
    pub next_attempt_at_ms: Option<i64>,
    /// Sequence of the last delivered seal of the app's evidence chain.
    pub evidence_cursor: i64,
    /// Sequence of the last delivered seal of the app's activity chain.
    pub activity_cursor: i64,
    pub created_at_ms: i64,
    /// Signing secret, returned only by the call that created the webhook.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

impl From<&audit_export_target::Model> for AuditWebhookView {
    fn from(target: &audit_export_target::Model) -> Self {
        Self {
            url: target.url.clone(),
            active: target.active,
            failures: target.failures,
            last_error: target.last_error.clone(),
            last_delivered_at_ms: target.last_delivered_at.map(|at| at.timestamp_millis()),
            next_attempt_at_ms: target.next_attempt_at.map(|at| at.timestamp_millis()),
            evidence_cursor: target.evidence_cursor,
            activity_cursor: target.activity_cursor,
            created_at_ms: target.created_at.timestamp_millis(),
            secret: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct PutAuditWebhookBody {
    /// `https` URL that receives each delivery as a signed NDJSON POST.
    pub url: String,
    /// Defaults to true.
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AuditWebhookSecret {
    /// Verify `X-FlowLike-Audit-Signature: v1=<hex>` as HMAC-SHA256 with this secret over
    /// `"<X-FlowLike-Audit-Timestamp>.<body>"`. Shown once.
    pub secret: String,
}

fn no_webhook() -> ApiError {
    ApiError::not_found("No audit webhook is configured for this app")
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/audit/webhook",
    tag = "audit",
    description = "The app's audit webhook and its delivery status. The signing secret is never returned here. Requires Owner on the app.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Webhook configuration and delivery status", body = AuditWebhookView),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No webhook is configured")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/audit/webhook", skip(state, user))]
pub async fn get_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AuditWebhookView>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let target = audit_export_target::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or_else(no_webhook)?;
    Ok(Json(AuditWebhookView::from(&target)))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/audit/webhook",
    tag = "audit",
    description = "Create or replace the app's audit webhook. The audit worker then POSTs the app's sealed records to it as signed NDJSON within minutes of sealing. The URL must be https and resolve to a public address. Saving resets the failure count and resumes a stopped webhook. The signing secret is returned only when this call creates the webhook. Requires Owner on the app.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = PutAuditWebhookBody,
    responses(
        (status = 200, description = "Saved webhook; `secret` is present only when it was created", body = AuditWebhookView),
        (status = 400, description = "URL not allowed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/audit/webhook", skip(state, user, body))]
pub async fn put_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<PutAuditWebhookBody>,
) -> Result<Json<AuditWebhookView>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let url = body.url.trim();
    if url.len() > MAX_URL_LENGTH {
        return Err(ApiError::bad_request(format!(
            "audit webhook URL is {} characters long; at most {MAX_URL_LENGTH} are allowed",
            url.len()
        )));
    }
    export::check_url(url)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let active = body.active.unwrap_or(true);
    let now = Utc::now().trunc_subsecs(3).fixed_offset();

    let existing = audit_export_target::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?;
    let (target, secret) = match existing {
        Some(existing) => {
            let target = audit_export_target::Entity::update_many()
                .set(audit_export_target::ActiveModel {
                    url: Set(url.to_owned()),
                    active: Set(active),
                    failures: Set(0),
                    next_attempt_at: Set(None),
                    last_error: Set(None),
                    ..Default::default()
                })
                .col_expr(
                    audit_export_target::Column::UpdatedAt,
                    export::configuration_revision(now),
                )
                .filter(audit_export_target::Column::AppId.eq(&app_id))
                .filter(audit_export_target::Column::Secret.eq(existing.secret))
                .exec_with_returning(&state.db)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| ApiError::conflict("The audit webhook changed; save it again"))?;
            (target, None)
        }
        None => {
            let secret = export::new_secret()?;
            let target = audit_export_target::ActiveModel {
                app_id: Set(app_id.clone()),
                url: Set(url.to_owned()),
                secret: Set(encrypt_secret(&secret, &state.encryption_key)),
                active: Set(active),
                evidence_cursor: Set(0),
                activity_cursor: Set(0),
                failures: Set(0),
                next_attempt_at: Set(None),
                last_delivered_at: Set(None),
                last_error: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };
            (target.insert(&state.db).await?, Some(secret))
        }
    };

    audit_branch!(
        state,
        user,
        app_id,
        "audit.export.webhook.set",
        "AuditExportTarget",
        app_id,
        serde_json::json!({ "created": secret.is_some(), "active": active })
    );

    let mut view = AuditWebhookView::from(&target);
    view.secret = secret;
    Ok(Json(view))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/audit/webhook/rotate",
    tag = "audit",
    description = "Replace the signing secret of the app's audit webhook. The new secret is returned once; deliveries from now on are signed with it. Requires Owner on the app.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "The new signing secret", body = AuditWebhookSecret),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No webhook is configured")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/audit/webhook/rotate", skip(state, user))]
pub async fn rotate_webhook_secret(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AuditWebhookSecret>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let secret = export::new_secret()?;
    let updated = audit_export_target::Entity::update_many()
        .col_expr(
            audit_export_target::Column::Secret,
            Expr::value(encrypt_secret(&secret, &state.encryption_key)),
        )
        .col_expr(
            audit_export_target::Column::UpdatedAt,
            export::configuration_revision(Utc::now().fixed_offset()),
        )
        .filter(audit_export_target::Column::AppId.eq(&app_id))
        .exec(&state.db)
        .await?;
    if updated.rows_affected == 0 {
        return Err(no_webhook());
    }

    audit_branch!(
        state,
        user,
        app_id,
        "audit.export.webhook.rotate",
        "AuditExportTarget",
        app_id
    );

    Ok(Json(AuditWebhookSecret { secret }))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/audit/webhook",
    tag = "audit",
    description = "Remove the app's audit webhook. Deliveries stop; the audit trail itself is unchanged and stays available through the export endpoint. Requires Owner on the app.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 204, description = "Webhook removed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No webhook is configured")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/audit/webhook", skip(state, user))]
pub async fn delete_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let deleted = audit_export_target::Entity::delete_by_id(&app_id)
        .exec(&state.db)
        .await?;
    if deleted.rows_affected == 0 {
        return Err(no_webhook());
    }

    audit_branch!(
        state,
        user,
        app_id,
        "audit.export.webhook.delete",
        "AuditExportTarget",
        app_id
    );

    Ok(StatusCode::NO_CONTENT)
}
