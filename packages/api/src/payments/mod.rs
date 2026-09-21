use std::sync::Arc;

use axum::http::StatusCode;
use flow_like_secrets::{ExposeSecret, SecretRef};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::{
    error::ApiError,
    state::AppState,
    stripe_connect::{HttpStripeGateway, STRIPE_API_VERSION, StripeScope},
};

pub mod accounts;
pub mod admin;
pub mod domain;
pub mod earnings;
pub mod inbox;
pub mod legacy_checkout;
pub mod mail_confirmations;
pub mod marketplace;
pub mod node;
pub mod operations;
pub mod outbox;
pub mod tax;
pub mod worker;

pub const LEGACY_STRIPE_VERSION: &str = "2023-10-16";

pub(crate) fn sql(statement: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, statement, values)
}

pub fn error(code: &str, message: &str) -> ApiError {
    ApiError::coded(StatusCode::CONFLICT, code, message)
}

pub(crate) fn stripe_error(error: crate::stripe_connect::StripeError) -> ApiError {
    tracing::warn!(error = %error, "Stripe payment operation failed");
    ApiError::coded(
        StatusCode::BAD_GATEWAY,
        "PAYMENT_PROVIDER_ERROR",
        "The payment provider could not complete this operation. Check its status before retrying.",
    )
}

pub async fn gateway_for_version(
    state: &AppState,
    version: &str,
) -> Result<Arc<HttpStripeGateway>, ApiError> {
    let cell = match version {
        LEGACY_STRIPE_VERSION => &state.legacy_payments_gateway,
        STRIPE_API_VERSION => &state.payments_gateway,
        _ => {
            return Err(error(
                "PAYMENT_API_VERSION_UNSUPPORTED",
                "Unsupported payment API version",
            ));
        }
    };
    let gateway = cell
        .get_or_try_init(|| async {
            let secret = state
                .secrets
                .get_secret_string(&SecretRef::new("STRIPE_SECRET_KEY"))
                .await
                .map_err(|_| {
                    error(
                        "PAYMENTS_DISABLED",
                        "The payment provider is not configured",
                    )
                })?;
            let key = secret.expose_secret();
            let livemode = key.starts_with("sk_live_") || key.starts_with("rk_live_");
            if version == STRIPE_API_VERSION && livemode != state.platform_config.payments.livemode
            {
                return Err(error(
                    "PAYMENT_MODE_MISMATCH",
                    "The payment key and configuration modes differ",
                ));
            }
            let gateway = HttpStripeGateway::discover(key, livemode, version.to_owned())
                .await
                .map_err(stripe_error)?;
            if version == STRIPE_API_VERSION
                && state
                    .platform_config
                    .payments
                    .platform_account_id
                    .as_deref()
                    .is_some_and(|account| account != gateway.platform_account_id())
            {
                return Err(error(
                    "PAYMENT_ACCOUNT_MISMATCH",
                    "The payment key belongs to another platform account",
                ));
            }
            Ok(Arc::new(gateway))
        })
        .await?;
    Ok(Arc::clone(gateway))
}

pub async fn platform_scope(state: &AppState) -> Result<StripeScope, ApiError> {
    let gateway = gateway_for_version(state, LEGACY_STRIPE_VERSION).await?;
    Ok(StripeScope::platform(
        gateway.platform_account_id(),
        gateway.livemode(),
    ))
}

pub async fn connect_scope(state: &AppState) -> Result<StripeScope, ApiError> {
    let gateway = gateway_for_version(state, STRIPE_API_VERSION).await?;
    Ok(StripeScope::platform(
        gateway.platform_account_id(),
        gateway.livemode(),
    ))
}

pub async fn payee_for_app<C: ConnectionTrait>(db: &C, app_id: &str) -> Result<String, ApiError> {
    let rows = db.query_all_raw(sql(
        r#"SELECT m."userId" FROM "App" a JOIN "Membership" m ON m."appId"=a.id AND m."roleId"=a."ownerRoleId" WHERE a.id=$1 LIMIT 2"#,
        vec![app_id.into()],
    )).await?;
    if rows.len() != 1 {
        return Err(error(
            "PAYMENT_OWNER_INVALID",
            "The app must have exactly one owner",
        ));
    }
    Ok(rows[0].try_get("", "userId")?)
}

pub async fn ensure_app_owner<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    if payee_for_app(db, app_id).await? != user_id {
        return Err(ApiError::forbidden(
            "Only the app owner can manage its payment settings",
        ));
    }
    Ok(())
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/user/payments/connect", get(accounts::get_account))
        .route("/user/payments/connect/countries", get(accounts::countries))
        .route(
            "/user/payments/connect/onboarding",
            post(accounts::onboarding),
        )
        .route("/user/payments/connect/resume", post(accounts::resume))
        .route(
            "/user/payments/connect/disconnect",
            post(accounts::disconnect),
        )
        .route(
            "/user/payments/connect/reconnect",
            post(accounts::reconnect),
        )
        .route("/user/payments/earnings", get(earnings::history))
        .route("/user/payments/balance", get(earnings::balance))
        .route("/admin/payments/queue", get(admin::queue))
        .route(
            "/admin/payments/sellers/{user_id}/block",
            post(admin::block_seller),
        )
        .route(
            "/admin/payments/effects/{id}/resume",
            post(admin::retry_effect),
        )
        .route(
            "/apps/{app_id}/payments/readiness",
            get(accounts::readiness),
        )
        .route(
            "/apps/{app_id}/payments/settings",
            get(accounts::get_settings).patch(accounts::update_settings),
        )
        .route("/payments/terms", get(accounts::terms))
        .route("/webhook/stripe/connect", post(inbox::connect))
        .route("/webhook/stripe/marketplace", post(inbox::marketplace))
        .route("/maintenance/payments", post(worker::maintenance))
        .merge(marketplace::router())
        .merge(node::router())
}
