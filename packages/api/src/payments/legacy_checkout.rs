//! Durable creation and reuse for platform checkouts issued before Connect cutover.

use crate::{
    entity::legacy_checkout,
    error::ApiError,
    state::AppState,
    stripe_connect::{StripeRequest, StripeScope},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

pub const LEGACY_API_VERSION: &str = "2023-10-16";

pub fn checkout_urls(
    frontend: &str,
    package: bool,
    item_id: &str,
) -> Result<(String, String), ApiError> {
    let mut url = flow_like_types::reqwest::Url::parse(frontend)
        .map_err(|_| ApiError::internal("Invalid FRONTEND_URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::internal("Invalid FRONTEND_URL"));
    }
    url.set_path(if package { "/store/packages" } else { "/store" });
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("id", item_id)
        .append_pair("checkout", "submitted");
    let success = url.to_string();
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("id", item_id)
        .append_pair("checkout", "canceled");
    Ok((success, url.to_string()))
}

/// The digest excludes generated expiry/operation identity, which belong to the persisted row.
fn checkout_digest(scope: &StripeScope, parameters: &Value) -> String {
    blake3::hash(
        serde_json::to_string(&json!({"scope": scope, "parameters": parameters}))
            .expect("JSON values serialize")
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

pub async fn start(
    state: &AppState,
    kind: &str,
    user_id: &str,
    item_id: &str,
    parameters: Value,
) -> Result<Option<String>, ApiError> {
    let config = &state.platform_config.payments;
    if kind == "app_purchase" && config.marketplace_enabled
        || config
            .legacy_checkout_until
            .is_some_and(|until| chrono::Utc::now().timestamp_millis() >= until)
    {
        return Err(crate::payments::error(
            "CHECKOUT_UPGRADE_REQUIRED",
            "This checkout has moved. Update FlowLike and reopen the app listing.",
        ));
    }
    let scope = crate::payments::platform_scope(state).await?;
    let open_key = format!(
        "{}:{}:{kind}:{}",
        scope.platform_account_id,
        scope.livemode,
        blake3::hash(format!("{user_id}:{item_id}").as_bytes()).to_hex()
    );
    let digest = checkout_digest(&scope, &parameters);
    let id = flow_like_types::create_id();
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = now + 60 * 60 * 1000;
    let kind = kind.to_owned();
    let user_id = user_id.to_owned();
    let item_id = item_id.to_owned();
    let mut persisted_parameters = parameters;
    persisted_parameters["expires_at"] = json!(expires_at / 1000);
    persisted_parameters["metadata"]["flowlike_checkout_id"] = json!(id);
    let checkout = state
        .transaction(|txn| {
            let scope = scope.clone();
            let (open_key, digest, id, kind, user_id, item_id, parameters) = (
                open_key.clone(),
                digest.clone(),
                id.clone(),
                kind.clone(),
                user_id.clone(),
                item_id.clone(),
                persisted_parameters.clone(),
            );
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "legacy-checkout", &[&open_key]).await?;
                if let Some(existing) = legacy_checkout::Entity::find()
                    .filter(legacy_checkout::Column::OpenKey.eq(&open_key))
                    .one(txn)
                    .await?
                {
                    return Ok::<_, ApiError>(existing);
                }
                let mut request = StripeRequest::post(
                    scope,
                    "/v1/checkout/sessions",
                    parameters.clone(),
                    format!("legacy:{id}:checkout"),
                );
                request.api_version = LEGACY_API_VERSION.into();
                crate::payments::operations::prepare(
                    txn,
                    "LEGACY_CHECKOUT",
                    &id,
                    "checkout_create",
                    &request,
                )
                .await?;
                legacy_checkout::ActiveModel {
                    id: Set(id),
                    open_key: Set(Some(open_key)),
                    kind: Set(kind),
                    user_id: Set(user_id),
                    item_id: Set(item_id),
                    parameters: Set(parameters),
                    request_digest: Set(digest),
                    stripe_session_id: Set(None),
                    checkout_url: Set(None),
                    status: Set("OPENING".into()),
                    expires_at: Set(expires_at),
                    failure_reason: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(txn)
                .await
                .map_err(ApiError::from)
            })
        })
        .await?;

    if checkout.status == "OPEN" && checkout.expires_at > now && checkout.request_digest == digest {
        return Ok(checkout.checkout_url);
    }
    if matches!(checkout.status.as_str(), "CANCELING" | "REVIEW") {
        return Err(ApiError::conflict(
            "The previous checkout is being reconciled. Retry shortly.",
        ));
    }
    if let Some(session_id) = checkout.stripe_session_id.as_deref() {
        if checkout.request_digest != digest || checkout.expires_at <= now {
            resolve_previous(state, &checkout, &scope, session_id).await?;
            return Err(ApiError::conflict(
                "The previous checkout has been resolved. Retry to use the current offer.",
            ));
        }
    }

    // An unresolved create always uses the original parameters, even after an offer changes.
    let mut request = StripeRequest::post(
        scope,
        "/v1/checkout/sessions",
        checkout.parameters.clone(),
        format!("legacy:{}:checkout", checkout.id),
    );
    request.api_version = LEGACY_API_VERSION.into();
    let response = crate::payments::operations::execute(
        state,
        "LEGACY_CHECKOUT",
        &checkout.id,
        "checkout_create",
        request,
    )
    .await?;
    let session_id = response
        .body
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| crate::stripe_connect::request::valid_id(id, "cs_"))
        .ok_or_else(|| {
            ApiError::service_unavailable("Stripe checkout response is awaiting reconciliation")
        })?
        .to_owned();
    let checkout_url = response
        .body
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let updated = state
        .transaction(|txn| {
            let (id, open_key, session_id, checkout_url) = (
                checkout.id.clone(),
                open_key.clone(),
                session_id.clone(),
                checkout_url.clone(),
            );
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "legacy-checkout", &[&open_key]).await?;
                let current = legacy_checkout::Entity::find_by_id(id)
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                if current.status != "OPENING" {
                    return Ok::<_, ApiError>(current);
                }
                let id = current.id;
                legacy_checkout::Entity::update_many()
                    .set(legacy_checkout::ActiveModel {
                        stripe_session_id: Set(Some(session_id)),
                        checkout_url: Set(checkout_url),
                        status: Set("OPEN".into()),
                        updated_at: Set(chrono::Utc::now().timestamp_millis()),
                        ..Default::default()
                    })
                    .filter(legacy_checkout::Column::Id.eq(&id))
                    .filter(legacy_checkout::Column::Status.eq("OPENING"))
                    .exec(txn)
                    .await?;
                legacy_checkout::Entity::find_by_id(id)
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)
            })
        })
        .await?;
    if updated.request_digest != digest
        || updated.expires_at <= chrono::Utc::now().timestamp_millis()
    {
        return Err(ApiError::conflict(
            "The original checkout is being resolved. Retry shortly.",
        ));
    }
    Ok(updated.checkout_url)
}

async fn resolve_previous(
    state: &AppState,
    checkout: &legacy_checkout::Model,
    scope: &StripeScope,
    session_id: &str,
) -> Result<(), ApiError> {
    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("Stripe not configured"))?;
    let stripe_id = session_id
        .parse()
        .map_err(|_| ApiError::internal("Invalid stored Checkout Session id"))?;
    let session = stripe::CheckoutSession::retrieve(stripe_client, &stripe_id, &[]).await?;
    if session.status == Some(stripe::CheckoutSessionStatus::Complete) {
        crate::routes::webhook::reconcile_legacy_checkout(state, &session).await?;
        return Err(ApiError::conflict(
            "Payment is settling. Check your purchase before starting another checkout.",
        ));
    }
    if session.status != Some(stripe::CheckoutSessionStatus::Expired) {
        let mut request = StripeRequest::post(
            scope.clone(),
            format!("/v1/checkout/sessions/{session_id}/expire"),
            json!({}),
            format!("legacy:{}:expire", checkout.id),
        );
        request.api_version = LEGACY_API_VERSION.into();
        let expiry_result = crate::payments::operations::execute(
            state,
            "LEGACY_CHECKOUT",
            &checkout.id,
            "checkout_expire",
            request,
        )
        .await;
        if let Err(error) = expiry_result {
            let current = stripe::CheckoutSession::retrieve(stripe_client, &stripe_id, &[]).await?;
            if current.status == Some(stripe::CheckoutSessionStatus::Complete) {
                crate::routes::webhook::reconcile_legacy_checkout(state, &current).await?;
                return Err(ApiError::conflict(
                    "Payment is settling. Check your purchase before starting another checkout.",
                ));
            }
            if current.status != Some(stripe::CheckoutSessionStatus::Expired) {
                return Err(error);
            }
        }
    }
    state
        .transaction(|txn| {
            let id = checkout.id.clone();
            let open_key = checkout.open_key.clone();
            Box::pin(async move {
                if let Some(key) = &open_key {
                    crate::db::coordination::coordinate(txn, "legacy-checkout", &[key]).await?;
                }
                let current = legacy_checkout::Entity::find_by_id(id)
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                if current.open_key != open_key {
                    return Ok::<_, ApiError>(());
                }
                let mut active: legacy_checkout::ActiveModel = current.into();
                active.open_key = Set(None);
                active.status = Set("EXPIRED".into());
                active.updated_at = Set(chrono::Utc::now().timestamp_millis());
                active.update(txn).await?;
                Ok(())
            })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn return_urls_are_server_owned_and_encode_identifiers() {
        let (success, cancel) = checkout_urls(
            "https://app.example/base?old=1#fragment",
            true,
            "pkg&next=https://evil.example",
        )
        .unwrap();
        let url = flow_like_types::reqwest::Url::parse(&success).unwrap();
        assert_eq!(url.host_str(), Some("app.example"));
        assert_eq!(url.path(), "/store/packages");
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "id").unwrap().1,
            "pkg&next=https://evil.example"
        );
        assert!(!cancel.contains("purchase=success"));
        assert!(checkout_urls("javascript:alert(1)", false, "app").is_err());
    }
    #[test]
    fn scope_and_commercial_snapshot_change_the_checkout_identity() {
        let scope = StripeScope::platform("acct_platform", false);
        let parameters = json!({"amount": 500, "customer": "cus_buyer"});
        assert_eq!(
            checkout_digest(&scope, &parameters),
            checkout_digest(&scope, &parameters)
        );
        assert_ne!(
            checkout_digest(&scope, &parameters),
            checkout_digest(&StripeScope::platform("acct_platform", true), &parameters)
        );
        assert_ne!(
            checkout_digest(&scope, &parameters),
            checkout_digest(&scope, &json!({"amount": 600,"customer":"cus_buyer"}))
        );
    }
}

pub(crate) async fn settled(
    txn: &sea_orm::DatabaseTransaction,
    session: &stripe::CheckoutSession,
) -> Result<(), ApiError> {
    let checkout = legacy_checkout::Entity::find()
        .filter(legacy_checkout::Column::StripeSessionId.eq(session.id.as_str()))
        .one(txn)
        .await?;
    let checkout = match checkout {
        Some(row) => Some(row),
        None => {
            if let Some(id) = session
                .metadata
                .as_ref()
                .and_then(|m| m.get("flowlike_checkout_id"))
            {
                legacy_checkout::Entity::find_by_id(id).one(txn).await?
            } else {
                None
            }
        }
    };
    let Some(row) = checkout else {
        return Ok(());
    };
    let expected_amount = row
        .parameters
        .pointer("/line_items/0/price_data/unit_amount")
        .and_then(Value::as_i64);
    if row
        .parameters
        .get("client_reference_id")
        .and_then(Value::as_str)
        != session.client_reference_id.as_deref()
        || expected_amount != session.amount_total
    {
        return Err(ApiError::bad_request(
            "Legacy checkout snapshot does not match its payment",
        ));
    }
    let mut active: legacy_checkout::ActiveModel = row.into();
    active.open_key = Set(None);
    active.status = Set("COMPLETED".into());
    active.stripe_session_id = Set(Some(session.id.to_string()));
    active.updated_at = Set(chrono::Utc::now().timestamp_millis());
    active.update(txn).await?;
    Ok(())
}

/// Recover legacy creates and paid sessions without requiring the buyer to reopen checkout.
pub async fn reconcile(state: &AppState, limit: u64) -> Result<(), ApiError> {
    use crate::entity::stripe_operation;
    use sea_orm::{QueryOrder, QuerySelect};
    let now = chrono::Utc::now().timestamp_millis();
    let rows = legacy_checkout::Entity::find()
        .filter(legacy_checkout::Column::Status.is_in(["OPENING", "OPEN", "PROCESSING"]))
        .filter(legacy_checkout::Column::UpdatedAt.lte(now - 60_000))
        .order_by_asc(legacy_checkout::Column::UpdatedAt)
        .limit(limit.min(100))
        .all(&state.db)
        .await?;
    let Some(client) = state.stripe_client.as_ref() else {
        return Ok(());
    };
    for row in rows {
        let result: Result<(), ApiError> = async {
            let mut session_id = row.stripe_session_id.clone();
            if session_id.is_none() {
                let operation = stripe_operation::Entity::find()
                    .filter(stripe_operation::Column::SourceType.eq("LEGACY_CHECKOUT"))
                    .filter(stripe_operation::Column::SourceId.eq(&row.id))
                    .filter(stripe_operation::Column::Operation.eq("checkout_create"))
                    .one(&state.db)
                    .await?;
                if let Some(operation) = operation {
                    if operation.status == "SUCCEEDED" {
                        let response: crate::stripe_connect::StripeResponse =
                            serde_json::from_value(operation.response.ok_or_else(|| {
                                ApiError::internal(
                                    "Completed checkout operation is missing its response",
                                )
                            })?)?;
                        session_id = response
                            .body
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        legacy_checkout::Entity::update_many()
                            .set(legacy_checkout::ActiveModel {
                                stripe_session_id: Set(session_id.clone()),
                                checkout_url: Set(response
                                    .body
                                    .get("url")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)),
                                status: Set("OPEN".into()),
                                updated_at: Set(now),
                                ..Default::default()
                            })
                            .filter(legacy_checkout::Column::Id.eq(&row.id))
                            .filter(legacy_checkout::Column::Status.eq("OPENING"))
                            .exec(&state.db)
                            .await?;
                    } else if matches!(
                        operation.status.as_str(),
                        "FAILED" | "MANUAL_REVIEW" | "INDETERMINATE"
                    ) {
                        legacy_checkout::Entity::update_many()
                            .set(legacy_checkout::ActiveModel {
                                failure_reason: Set(Some(format!(
                                    "operation:{}",
                                    operation.status
                                ))),
                                updated_at: Set(now),
                                ..Default::default()
                            })
                            .filter(legacy_checkout::Column::Id.eq(&row.id))
                            .filter(legacy_checkout::Column::Status.eq("OPENING"))
                            .exec(&state.db)
                            .await?;
                    }
                }
            }
            if let Some(session_id) = session_id {
                let session = stripe::CheckoutSession::retrieve(
                    client,
                    &session_id
                        .parse()
                        .map_err(|_| ApiError::internal("Invalid legacy Checkout Session id"))?,
                    &[],
                )
                .await?;
                if session.status == Some(stripe::CheckoutSessionStatus::Complete) {
                    if session.payment_status == stripe::CheckoutSessionPaymentStatus::Paid {
                        crate::routes::webhook::reconcile_legacy_checkout(state, &session).await?;
                    } else {
                        legacy_checkout::Entity::update_many()
                            .set(legacy_checkout::ActiveModel {
                                status: Set("PROCESSING".into()),
                                updated_at: Set(now),
                                ..Default::default()
                            })
                            .filter(legacy_checkout::Column::Id.eq(&row.id))
                            .filter(legacy_checkout::Column::Status.is_in([
                                "OPENING",
                                "OPEN",
                                "PROCESSING",
                            ]))
                            .exec(&state.db)
                            .await?;
                    }
                } else if session.status == Some(stripe::CheckoutSessionStatus::Expired) {
                    legacy_checkout::Entity::update_many()
                        .set(legacy_checkout::ActiveModel {
                            status: Set("EXPIRED".into()),
                            open_key: Set(None),
                            updated_at: Set(now),
                            ..Default::default()
                        })
                        .filter(legacy_checkout::Column::Id.eq(&row.id))
                        .filter(legacy_checkout::Column::Status.is_in(["OPENING", "OPEN"]))
                        .exec(&state.db)
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(checkout_id = %row.id, %error, "Legacy checkout reconciliation needs a retry");
        }
        legacy_checkout::Entity::update_many()
            .set(legacy_checkout::ActiveModel {
                updated_at: Set(now),
                ..Default::default()
            })
            .filter(legacy_checkout::Column::Id.eq(&row.id))
            .exec(&state.db)
            .await?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPurchaseView {
    id: String,
    item_id: String,
    item_kind: String,
    item_name: String,
    status: String,
    amount: i64,
    currency: String,
    created_at: i64,
    receipt_available: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPurchasePage {
    items: Vec<LegacyPurchaseView>,
    next_cursor: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct LegacyHistoryQuery {
    cursor: Option<String>,
}

fn history_cursor(
    cursor: Option<&str>,
) -> Result<Option<(chrono::DateTime<chrono::FixedOffset>, String, String)>, ApiError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let mut parts = cursor.splitn(3, ':');
    let milliseconds = parts
        .next()
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("Invalid purchase history cursor"))?;
    let kind = parts
        .next()
        .filter(|v| matches!(*v, "APP" | "PACKAGE"))
        .ok_or_else(|| ApiError::bad_request("Invalid purchase history cursor"))?;
    let id = parts
        .next()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or_else(|| ApiError::bad_request("Invalid purchase history cursor"))?;
    let time = chrono::DateTime::from_timestamp_millis(milliseconds)
        .ok_or_else(|| ApiError::bad_request("Invalid purchase history cursor"))?
        .fixed_offset();
    Ok(Some((time, kind.to_owned(), id.to_owned())))
}

pub async fn purchase_history(
    state: &AppState,
    buyer: &str,
    cursor: Option<&str>,
) -> Result<LegacyPurchasePage, ApiError> {
    use crate::entity::{app_purchase, meta, wasm_package, wasm_package_purchase};
    use sea_orm::{ActiveEnum, Condition, QueryOrder, QuerySelect};
    let cursor = history_cursor(cursor)?;
    let mut apps = app_purchase::Entity::find()
        .filter(app_purchase::Column::UserId.eq(buyer))
        .filter(app_purchase::Column::PaymentOrderId.is_null());
    let mut packages = wasm_package_purchase::Entity::find()
        .filter(wasm_package_purchase::Column::UserId.eq(buyer))
        .filter(wasm_package_purchase::Column::PaymentOrderId.is_null());
    if let Some((time, kind, id)) = &cursor {
        apps = apps.filter(if kind == "APP" {
            Condition::any()
                .add(app_purchase::Column::CreatedAt.lt(*time))
                .add(
                    Condition::all()
                        .add(app_purchase::Column::CreatedAt.eq(*time))
                        .add(app_purchase::Column::Id.lt(id)),
                )
        } else {
            Condition::all().add(app_purchase::Column::CreatedAt.lte(*time))
        });
        packages = packages.filter(if kind == "PACKAGE" {
            Condition::any()
                .add(wasm_package_purchase::Column::CreatedAt.lt(*time))
                .add(
                    Condition::all()
                        .add(wasm_package_purchase::Column::CreatedAt.eq(*time))
                        .add(wasm_package_purchase::Column::Id.lt(id)),
                )
        } else {
            Condition::all().add(wasm_package_purchase::Column::CreatedAt.lt(*time))
        });
    }
    let apps = apps
        .order_by_desc(app_purchase::Column::CreatedAt)
        .order_by_desc(app_purchase::Column::Id)
        .limit(51)
        .all(&state.db)
        .await?;
    let packages = packages
        .order_by_desc(wasm_package_purchase::Column::CreatedAt)
        .order_by_desc(wasm_package_purchase::Column::Id)
        .limit(51)
        .all(&state.db)
        .await?;
    let mut items = Vec::with_capacity(apps.len() + packages.len());
    for row in apps {
        let title = meta::Entity::find()
            .filter(meta::Column::AppId.eq(&row.app_id))
            .order_by_asc(meta::Column::Lang)
            .one(&state.db)
            .await?
            .map(|m| m.name)
            .unwrap_or_else(|| row.app_id.clone());
        items.push(LegacyPurchaseView {
            id: row.id,
            item_id: row.app_id,
            item_kind: "APP".into(),
            item_name: title,
            status: row.status.to_value(),
            amount: row.price_paid,
            currency: row.currency,
            created_at: row.created_at.timestamp_millis(),
            receipt_available: row.stripe_payment_intent_id.is_some(),
        });
    }
    for row in packages {
        let title = wasm_package::Entity::find_by_id(&row.package_id)
            .one(&state.db)
            .await?
            .map(|m| m.name)
            .unwrap_or_else(|| row.package_id.clone());
        items.push(LegacyPurchaseView {
            id: row.id,
            item_id: row.package_id,
            item_kind: "PACKAGE".into(),
            item_name: title,
            status: row.status.to_value(),
            amount: row.price_paid,
            currency: row.currency,
            created_at: row.created_at.timestamp_millis(),
            receipt_available: row.stripe_payment_intent_id.is_some(),
        });
    }
    items.sort_by(|a, b| {
        (b.created_at, &b.item_kind, &b.id).cmp(&(a.created_at, &a.item_kind, &a.id))
    });
    let more = items.len() > 50;
    items.truncate(50);
    let next_cursor = if more {
        items
            .last()
            .map(|row| format!("{}:{}:{}", row.created_at, row.item_kind, row.id))
    } else {
        None
    };
    Ok(LegacyPurchasePage { items, next_cursor })
}

#[utoipa::path(get,path="/user/payments/legacy-purchases",tag="payments",responses((status=200,description="Legacy platform purchase history")),security(("bearer_auth"=[])))]
pub async fn history(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(user): axum::Extension<crate::auth::AppUser>,
    axum::extract::Query(query): axum::extract::Query<LegacyHistoryQuery>,
) -> Result<axum::Json<LegacyPurchasePage>, ApiError> {
    let buyer = super::accounts::session_user(&user)?;
    purchase_history(&state, buyer, query.cursor.as_deref())
        .await
        .map(axum::Json)
}

#[utoipa::path(get,path="/user/payments/legacy-purchases/{kind}/{id}/receipt",tag="payments",responses((status=200,description="Canonical Stripe receipt when available")),security(("bearer_auth"=[])))]
pub async fn receipt(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(user): axum::Extension<crate::auth::AppUser>,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
) -> Result<axum::Json<Value>, ApiError> {
    use crate::entity::{app_purchase, wasm_package_purchase};
    let buyer = super::accounts::session_user(&user)?;
    let (intent_id, amount, currency) = match kind.as_str() {
        "APP" => {
            let row = app_purchase::Entity::find_by_id(id)
                .filter(app_purchase::Column::UserId.eq(buyer))
                .filter(app_purchase::Column::PaymentOrderId.is_null())
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            (row.stripe_payment_intent_id, row.price_paid, row.currency)
        }
        "PACKAGE" => {
            let row = wasm_package_purchase::Entity::find_by_id(id)
                .filter(wasm_package_purchase::Column::UserId.eq(buyer))
                .filter(wasm_package_purchase::Column::PaymentOrderId.is_null())
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            (row.stripe_payment_intent_id, row.price_paid, row.currency)
        }
        _ => return Err(ApiError::NOT_FOUND),
    };
    let Some(intent_id) = intent_id else {
        return Ok(axum::Json(json!({"url":null})));
    };
    let client = state
        .stripe_client
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("Legacy receipt service is unavailable"))?;
    let intent = stripe::PaymentIntent::retrieve(
        client,
        &intent_id
            .parse()
            .map_err(|_| ApiError::internal("Invalid stored payment reference"))?,
        &["latest_charge"],
    )
    .await?;
    if intent.amount != amount
        || !intent.currency.to_string().eq_ignore_ascii_case(&currency)
        || intent.metadata.contains_key("flowlike_kind")
    {
        return Err(ApiError::conflict("This legacy receipt needs review"));
    }
    let receipt = match intent.latest_charge {
        Some(stripe::Expandable::Object(charge))
            if charge.paid
                && charge.currency.to_string().eq_ignore_ascii_case(&currency)
                && charge
                    .payment_intent
                    .as_ref()
                    .is_some_and(|id| id.id().as_str() == intent_id) =>
        {
            charge.receipt_url
        }
        _ => None,
    }
    .filter(|url| {
        flow_like_types::reqwest::Url::parse(url).is_ok_and(|u| {
            u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
        })
    });
    Ok(axum::Json(json!({"url":receipt})))
}
