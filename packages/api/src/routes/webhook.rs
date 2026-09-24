use crate::{
    audit::{AuditRecordInput, record::package_scope},
    error::ApiError,
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use flow_like_secrets::{ExposeSecret, SecretRef};
use flow_like_types::anyhow;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
    Statement,
};
use serde_json::{Value, json};
use stripe::{Event, EventObject, EventType};

const STRIPE_ACTOR: &str = "system:stripe";

/// Filter the new payment namespace before decoding with the legacy SDK or claiming an event.
fn legacy_event_supported(event_type: &str, account: Option<&str>, object: &Value) -> bool {
    if account.is_some()
        || object.pointer("/metadata/flowlike_kind").is_some()
        || object
            .get("client_reference_id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with("mkt:"))
    {
        return false;
    }
    match event_type {
        "checkout.session.completed"
        | "checkout.session.async_payment_succeeded"
        | "checkout.session.expired" => {
            object.get("mode").and_then(Value::as_str) == Some("payment")
        }
        "customer.subscription.created"
        | "customer.subscription.updated"
        | "customer.subscription.deleted" => true,
        "payment_intent.succeeded" | "payment_intent.payment_failed" => {
            object.get("application").is_none_or(Value::is_null)
                && object.get("transfer_data").is_none_or(Value::is_null)
                && object
                    .get("transfer_group")
                    .and_then(Value::as_str)
                    .is_none_or(|id| !id.starts_with("mkt:"))
        }
        _ => false,
    }
}

#[tracing::instrument(name = "POST /webhook/stripe", skip(state, headers, payload))]
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let webhook_secret = state
        .secrets
        .get_secret_string(&SecretRef::new("STRIPE_WEBHOOK_SECRET"))
        .await
        .map_err(|_| ApiError::service_unavailable("Webhook secret not configured"))?;
    let signature = headers
        .get("stripe-signature")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("Missing Stripe signature"))?;
    let envelope = crate::stripe_connect::verify_webhook(
        &payload,
        signature,
        &[webhook_secret.expose_secret()],
        chrono::Utc::now().timestamp().max(0) as u64,
        300,
    )
    .map_err(|_| ApiError::bad_request("Invalid Stripe webhook signature or envelope"))?;
    if !legacy_event_supported(
        &envelope.event_type,
        envelope.account.as_deref(),
        &envelope.data.object,
    ) {
        return Ok(StatusCode::OK);
    }
    let scope = crate::payments::platform_scope(&state).await?;
    if envelope.livemode != scope.livemode {
        return Ok(StatusCode::OK);
    }
    let event: Event = serde_json::from_slice(&payload).map_err(|_| {
        ApiError::bad_request("Webhook payload does not match the legacy Stripe API version")
    })?;
    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("Stripe not configured"))?;
    match (&event.type_, &event.data.object) {
        (
            EventType::CheckoutSessionCompleted | EventType::CheckoutSessionAsyncPaymentSucceeded,
            EventObject::CheckoutSession(session),
        ) => {
            if session.payment_status == stripe::CheckoutSessionPaymentStatus::Paid {
                handle_checkout_completed(&state, session, &event).await?;
            }
        }
        (EventType::CheckoutSessionExpired, EventObject::CheckoutSession(session)) => {
            handle_checkout_expired(&state, session, &event).await?
        }
        (
            EventType::CustomerSubscriptionCreated
            | EventType::CustomerSubscriptionUpdated
            | EventType::CustomerSubscriptionDeleted,
            EventObject::Subscription(subscription),
        ) => {
            if !is_event_processed(&state, event.id.as_str()).await? {
                handle_subscription_change(
                    &state,
                    stripe_client,
                    subscription,
                    event.created,
                    event.id.as_str(),
                )
                .await?;
                mark_event_processed(&state, event.id.as_str(), &event.type_.to_string()).await?;
            }
        }
        (EventType::PaymentIntentSucceeded, EventObject::PaymentIntent(intent)) => {
            handle_payment_intent_succeeded(&state, intent).await?;
        }
        (EventType::PaymentIntentPaymentFailed, EventObject::PaymentIntent(intent)) => {
            handle_payment_intent_failed(&state, intent).await?;
        }
        _ => {}
    }
    Ok(StatusCode::OK)
}

async fn is_event_processed(state: &AppState, event_id: &str) -> Result<bool, ApiError> {
    Ok(crate::entity::stripe_event::Entity::find_by_id(event_id)
        .one(&state.db)
        .await?
        .is_some())
}

async fn claim_event<C: ConnectionTrait>(
    db: &C,
    event_id: &str,
    event_type: &str,
) -> Result<bool, ApiError> {
    let result = db.execute_raw(Statement::from_sql_and_values(db.get_database_backend(),
        r#"INSERT INTO "StripeEvent" ("id", "eventType", "processedAt") VALUES ($1, $2, $3) ON CONFLICT ("id") DO NOTHING"#,
        [event_id.into(), event_type.into(), chrono::Utc::now().fixed_offset().into()],
    )).await?;
    Ok(result.rows_affected() == 1)
}

async fn mark_event_processed(
    state: &AppState,
    event_id: &str,
    event_type: &str,
) -> Result<(), ApiError> {
    claim_event(&state.db, event_id, event_type).await?;
    Ok(())
}

fn purchase_reference(reference: &str) -> Option<(&str, &str, &str)> {
    let (kind, rest) = reference.split_once(':')?;
    if !matches!(kind, "app_purchase" | "wasm_purchase") {
        return None;
    }
    let (user, item) = rest.rsplit_once(':')?;
    if user.is_empty() || item.is_empty() {
        return None;
    }
    Some((kind, user, item))
}

fn purchase_money(session: &stripe::CheckoutSession) -> Result<(i64, i64, String), ApiError> {
    let amount = session
        .amount_total
        .filter(|amount| *amount > 0)
        .ok_or_else(|| ApiError::bad_request("Paid purchase is missing its positive amount"))?;
    let original = session.amount_subtotal.unwrap_or(amount);
    if original < amount {
        return Err(ApiError::bad_request("Invalid legacy purchase subtotal"));
    }
    let currency = session
        .currency
        .ok_or_else(|| ApiError::bad_request("Purchase currency is missing"))?
        .to_string()
        .to_uppercase();
    Ok((amount, original, currency))
}

async fn handle_checkout_completed(
    state: &AppState,
    session: &stripe::CheckoutSession,
    event: &Event,
) -> Result<(), ApiError> {
    let Some(reference) = session.client_reference_id.as_deref() else {
        return Ok(());
    };
    if let Some((kind, user, item)) = purchase_reference(reference) {
        return settle_legacy_purchase(
            state,
            session,
            event.id.as_str(),
            &event.type_.to_string(),
            kind,
            user,
            item,
        )
        .await;
    }
    settle_solution_checkout(state, session, event, false).await
}

async fn settle_solution_checkout(
    state: &AppState,
    session: &stripe::CheckoutSession,
    event: &Event,
    expired: bool,
) -> Result<(), ApiError> {
    use crate::entity::{sea_orm_active_enums::SolutionStatus, solution_request};
    let session = session.clone();
    let event_id = event.id.to_string();
    let event_type = event.type_.to_string();
    state
        .transaction(|txn| {
            let session = session.clone();
            let event_id = event_id.clone();
            let event_type = event_type.clone();
            Box::pin(async move {
                let Some(reference) = session.client_reference_id.as_deref() else {
                    return Ok::<_, ApiError>(());
                };
                crate::db::coordination::coordinate(txn, "legacy-solution", &[reference]).await?;
                let Some(solution) = solution_request::Entity::find_by_id(reference)
                    .one(txn)
                    .await?
                else {
                    return Ok(());
                };
                let session_id = session.id.as_str();
                let bound = solution.stripe_checkout_session_id.as_deref() == Some(session_id);
                let historical = solution.stripe_checkout_session_id.is_none()
                    && session
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("submission_id"))
                        .map(String::as_str)
                        == Some(reference);
                if !bound && !historical {
                    return Ok(());
                }
                if !expired
                    && (session.amount_total != Some(solution.deposit_cents)
                        || session.currency != Some(stripe::Currency::EUR))
                {
                    return Err(ApiError::bad_request(
                        "Solution deposit amount or currency does not match",
                    ));
                }
                if !claim_event(txn, &event_id, &event_type).await? {
                    return Ok(());
                }
                if expired && solution.paid_deposit {
                    return Ok(());
                }
                let mut active: solution_request::ActiveModel = solution.into();
                active.stripe_checkout_session_id = Set(Some(session_id.to_string()));
                if expired {
                    active.status = Set(SolutionStatus::Cancelled);
                } else {
                    active.stripe_payment_intent_id = Set(session
                        .payment_intent
                        .as_ref()
                        .map(|pi| pi.id().to_string()));
                    active.paid_deposit = Set(true);
                    active.status = Set(SolutionStatus::PendingReview);
                }
                active.updated_at = Set(chrono::Utc::now().fixed_offset());
                active.update(txn).await?;
                if !expired {
                    crate::audit::record::write(
                        txn,
                        AuditRecordInput::system(
                            STRIPE_ACTOR,
                            "solution.deposit.paid",
                            "SolutionRequest",
                            &event_id,
                        )
                        .with_details(
                            json!({"solution_id": reference, "stripe_session_id": session_id}),
                        ),
                        crate::audit::WriteMode::Once,
                    )
                    .await?;
                }
                Ok(())
            })
        })
        .await
}

async fn settle_legacy_purchase(
    state: &AppState,
    session: &stripe::CheckoutSession,
    event_id: &str,
    event_type: &str,
    kind: &str,
    user_id: &str,
    item_id: &str,
) -> Result<(), ApiError> {
    use crate::entity::{
        access_grant, app, app_purchase, membership, payment_entitlement,
        sea_orm_active_enums::PurchaseStatus, user, wasm_package, wasm_package_purchase,
        wasm_package_user,
    };
    let (amount, original, currency) = purchase_money(session)?;
    let scope = crate::payments::platform_scope(state).await?;
    let session_id = session.id.to_string();
    let intent_id = session
        .payment_intent
        .as_ref()
        .map(|pi| pi.id().to_string())
        .ok_or_else(|| ApiError::bad_request("Paid purchase has no PaymentIntent"))?;
    let customer_id = session
        .customer
        .as_ref()
        .map(|customer| customer.id().to_string());
    let event_id = event_id.to_owned();
    let event_type = event_type.to_owned();
    let session = session.clone();
    let user_id = user_id.to_owned();
    let item_id = item_id.to_owned();
    let kind = kind.to_owned();
    let purchase_id = format!(
        "legacy_{}",
        blake3::hash(format!("{kind}:{session_id}").as_bytes()).to_hex()
    );
    state.transaction(|txn| {
        let (session_id, intent_id, customer_id, event_id, event_type, user_id, item_id, kind, purchase_id, currency) =
            (session_id.clone(), intent_id.clone(), customer_id.clone(), event_id.clone(), event_type.clone(), user_id.clone(), item_id.clone(), kind.clone(), purchase_id.clone(), currency.clone());
        let session = session.clone();
        let scope = scope.clone();
        Box::pin(async move {
            crate::db::coordination::coordinate(txn, "legacy-purchase", &[&kind, &user_id, &item_id]).await?;
            let entitlement_kind = if kind == "app_purchase" {
                crate::payments::marketplace::ItemKind::App
            } else {
                crate::payments::marketplace::ItemKind::Package
            };
            crate::payments::marketplace::coordinate_entitlement(txn, &user_id, entitlement_kind, &item_id).await?;
            let item_kind = if kind == "app_purchase" { "APP" } else { "PACKAGE" };
            let blocked = payment_entitlement::Entity::find()
                .filter(payment_entitlement::Column::UserId.eq(&user_id))
                .filter(payment_entitlement::Column::ItemKind.eq(item_kind))
                .filter(payment_entitlement::Column::ItemId.eq(&item_id))
                .one(txn).await?.is_some_and(|row| row.blocked);
            let buyer = user::Entity::find_by_id(&user_id).one(txn).await?;
            if buyer.as_ref().is_some_and(|buyer| buyer.stripe_id != customer_id) || customer_id.is_none() {
                return Err(ApiError::bad_request("Purchase customer does not match its buyer"));
            }
            if !claim_event(txn, &event_id, &event_type).await? { return Ok::<_, ApiError>(()); }
            crate::payments::legacy_checkout::settled(txn, &session).await?;
            let now = chrono::Utc::now().fixed_offset();
            let orphan;
            if kind == "app_purchase" {
                if app_purchase::Entity::find().filter(app_purchase::Column::StripeSessionId.eq(&session_id)).one(txn).await?.is_some() {
                    return Ok(());
                }
                let item = app::Entity::find_by_id(&item_id).one(txn).await?;
                let existing_access = membership::Entity::find().filter(membership::Column::AppId.eq(&item_id)).filter(membership::Column::UserId.eq(&user_id)).one(txn).await?;
                orphan = blocked || buyer.is_none() || item.as_ref().and_then(|item| item.default_role_id.as_ref()).is_none() || existing_access.is_some();
                app_purchase::ActiveModel {
                    id: Set(purchase_id.clone()), payment_order_id: Set(None), charge_type: Set(Some("legacy_platform".into())), user_id: Set(user_id.clone()), app_id: Set(item_id.clone()),
                    price_paid: Set(amount), original_price: Set(original), discount_amount: Set(original - amount), discount_id: Set(None), currency: Set(currency.clone()),
                    stripe_session_id: Set(session_id.clone()), stripe_payment_intent_id: Set(Some(intent_id.clone())), status: Set(PurchaseStatus::Completed),
                    completed_at: Set(Some(now)), refunded_at: Set(None), refund_reason: Set(None), created_at: Set(now), updated_at: Set(now),
                }.insert(txn).await?;
                if !orphan {
                    membership::ActiveModel {
                        id: Set(flow_like_types::create_id()), user_id: Set(user_id.clone()), app_id: Set(item_id.clone()),
                        role_id: Set(item.and_then(|item| item.default_role_id).ok_or(ApiError::NOT_FOUND)?),
                        joined_via: Set(Some(format!("purchase:{session_id}"))), created_at: Set(now), updated_at: Set(now),
                    }.insert(txn).await?;
                }
            } else {
                if wasm_package_purchase::Entity::find().filter(wasm_package_purchase::Column::StripeSessionId.eq(&session_id)).one(txn).await?.is_some() { return Ok(()); }
                let item = wasm_package::Entity::find_by_id(&item_id).one(txn).await?;
                let access = wasm_package_user::Entity::find().filter(wasm_package_user::Column::PackageId.eq(&item_id)).filter(wasm_package_user::Column::UserId.eq(&user_id)).one(txn).await?;
                orphan = blocked || buyer.is_none() || item.is_none() || access.is_some();
                wasm_package_purchase::ActiveModel {
                    id: Set(purchase_id.clone()), payment_order_id: Set(None), charge_type: Set(Some("legacy_platform".into())), user_id: Set(user_id.clone()), package_id: Set(item_id.clone()),
                    price_paid: Set(amount), original_price: Set(original), discount_amount: Set(original - amount), currency: Set(currency.clone()),
                    stripe_session_id: Set(session_id.clone()), stripe_payment_intent_id: Set(Some(intent_id.clone())), status: Set(PurchaseStatus::Completed),
                    completed_at: Set(Some(now)), refunded_at: Set(None), refund_reason: Set(None), created_at: Set(now), updated_at: Set(now),
                }.insert(txn).await?;
                if !orphan {
                    wasm_package_user::ActiveModel {
                        id: Set(flow_like_types::create_id()), user_id: Set(user_id.clone()), package_id: Set(item_id.clone()),
                        permission: Set(crate::permission::wasm_package_permission::WasmPackagePermission::Buyer.bits()),
                        granted_by: Set(Some(format!("purchase:{session_id}"))), granted_at: Set(now),
                    }.insert(txn).await?;
                }
            }
            if !orphan {
                access_grant::ActiveModel {
                    id: Set(format!("legacy-purchase:{purchase_id}")), user_id: Set(user_id.clone()),
                    item_kind: Set(item_kind.into()), item_id: Set(item_id.clone()),
                    source_type: Set("LEGACY_PAYMENT".into()), source_id: Set(purchase_id.clone()),
                    status: Set("ACTIVE".into()), reason: Set(None), granted_by: Set(None), revision: Set(0),
                    created_at: Set(now.timestamp_millis()), updated_at: Set(now.timestamp_millis()),
                }.insert(txn).await?;
            }
            let effect = if orphan { "legacy_purchase_orphan_refund" } else { "legacy_purchase_completed" };
            crate::payments::outbox::enqueue(txn, &format!("{purchase_id}:{effect}"), effect, &kind, &purchase_id,
                json!({"kind": kind, "user_id": user_id, "item_id": item_id, "purchase_id": purchase_id, "session_id": session_id,
                    "payment_intent_id": intent_id, "amount": amount, "currency": currency, "scope": scope})).await?;
            Ok(())
        })
    }).await?;
    state.invalidate_permission(&user_id, &item_id);
    state.invalidate_wasm_permission(&user_id, &item_id);
    if kind == "wasm_purchase" {
        crate::package_license::refresh_package_access(state, &user_id, &item_id).await;
    }
    Ok(())
}

async fn handle_checkout_expired(
    state: &AppState,
    session: &stripe::CheckoutSession,
    event: &Event,
) -> Result<(), ApiError> {
    settle_solution_checkout(state, session, event, true).await
}

/// Reconcile a legacy checkout retrieved by its persisted Stripe id.
pub(crate) async fn reconcile_legacy_checkout(
    state: &AppState,
    session: &stripe::CheckoutSession,
) -> Result<(), ApiError> {
    if session.mode != stripe::CheckoutSessionMode::Payment
        || session.payment_status != stripe::CheckoutSessionPaymentStatus::Paid
    {
        return Ok(());
    }
    if let Some((kind, user, item)) = session
        .client_reference_id
        .as_deref()
        .and_then(purchase_reference)
    {
        settle_legacy_purchase(
            state,
            session,
            &format!("legacy-reconcile:{}", session.id),
            "checkout.session.reconciled",
            kind,
            user,
            item,
        )
        .await?;
    }
    Ok(())
}

fn effect_string<'a>(payload: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::internal(format!("Legacy payment effect is missing {key}")))
}

/// Called by the durable payments outbox. Every retry retains the original financial identity.
pub(crate) async fn deliver_legacy_purchase_effect(
    state: &AppState,
    effect: &str,
    payload: &Value,
) -> Result<(), ApiError> {
    use crate::entity::{
        app_purchase, meta,
        sea_orm_active_enums::{NotificationType, PurchaseStatus},
        user, wasm_package, wasm_package_purchase,
    };
    let kind = effect_string(payload, "kind")?;
    let purchase_id = effect_string(payload, "purchase_id")?;
    let user_id = effect_string(payload, "user_id")?;
    let item_id = effect_string(payload, "item_id")?;
    let session_id = effect_string(payload, "session_id")?;
    let intent_id = effect_string(payload, "payment_intent_id")?;
    if effect == "legacy_purchase_orphan_refund" {
        let scope = serde_json::from_value::<crate::stripe_connect::StripeScope>(
            payload
                .get("scope")
                .cloned()
                .ok_or_else(|| ApiError::internal("Legacy refund is missing its account scope"))?,
        )?;
        let mut request = crate::stripe_connect::StripeRequest::post(
            scope,
            "/v1/refunds",
            json!({"payment_intent": intent_id, "reason": "duplicate"}),
            format!("legacy:{purchase_id}:orphan-refund"),
        );
        request.api_version = crate::payments::legacy_checkout::LEGACY_API_VERSION.into();
        let response = crate::payments::operations::execute(
            state,
            "LEGACY_PURCHASE",
            purchase_id,
            "orphan_refund",
            request,
        )
        .await?;
        let refund_id = response
            .body
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::service_unavailable("Refund is awaiting reconciliation"))?;
        let client = state
            .stripe_client
            .as_ref()
            .ok_or_else(|| ApiError::service_unavailable("Stripe not configured"))?;
        let refund = stripe::Refund::retrieve(
            client,
            &refund_id
                .parse()
                .map_err(|_| ApiError::internal("Invalid stored refund id"))?,
            &[],
        )
        .await?;
        if refund.status.as_deref() != Some("succeeded") {
            return Err(ApiError::service_unavailable(
                "Legacy duplicate refund is pending or needs support",
            ));
        }
        state
            .transaction(|txn| {
                let purchase_id = purchase_id.to_owned();
                let kind = kind.to_owned();
                let refund_id = refund_id.to_owned();
                Box::pin(async move {
                    let now = chrono::Utc::now().fixed_offset();
                    if kind == "app_purchase" {
                        if let Some(row) = app_purchase::Entity::find_by_id(&purchase_id)
                            .one(txn)
                            .await?
                        {
                            let mut active: app_purchase::ActiveModel = row.into();
                            active.status = Set(PurchaseStatus::Refunded);
                            active.refunded_at = Set(Some(now));
                            active.refund_reason = Set(Some(format!("duplicate:{refund_id}")));
                            active.updated_at = Set(now);
                            active.update(txn).await?;
                        }
                    } else if let Some(row) =
                        wasm_package_purchase::Entity::find_by_id(&purchase_id)
                            .one(txn)
                            .await?
                    {
                        let mut active: wasm_package_purchase::ActiveModel = row.into();
                        active.status = Set(PurchaseStatus::Refunded);
                        active.refunded_at = Set(Some(now));
                        active.refund_reason = Set(Some(format!("duplicate:{refund_id}")));
                        active.updated_at = Set(now);
                        active.update(txn).await?;
                    }
                    Ok::<_, ApiError>(())
                })
            })
            .await?;
        return Ok(());
    }
    if effect != "legacy_purchase_completed" {
        return Err(ApiError::internal("Unknown legacy purchase effect"));
    }
    // A removed buyer cannot receive a notification; financial evidence remains retained.
    let Some(buyer) = user::Entity::find_by_id(user_id).one(&state.db).await? else {
        return Ok(());
    };
    let is_app = kind == "app_purchase";
    let name = if is_app {
        meta::Entity::find()
            .filter(meta::Column::AppId.eq(item_id))
            .filter(meta::Column::Lang.eq("en"))
            .one(&state.db)
            .await?
            .map(|m| m.name)
    } else {
        wasm_package::Entity::find_by_id(item_id)
            .one(&state.db)
            .await?
            .map(|p| p.name)
    }
    .unwrap_or_else(|| "your purchase".into());
    let frontend =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".into());
    let (link, _) = crate::payments::legacy_checkout::checkout_urls(&frontend, !is_app, item_id)?;
    let scope = if is_app {
        item_id.to_owned()
    } else {
        package_scope(item_id)
    };
    let audit = AuditRecordInput::system(STRIPE_ACTOR, if is_app {"membership.purchase"} else {"registry.access.purchase"}, if is_app {"membership"} else {"WasmPackage"}, purchase_id)
        .on_scope(&scope).with_details(json!({"user_id": user_id, "item_id": item_id, "purchase_id": purchase_id, "stripe_session_id": session_id}));
    if crate::audit::records(&state.platform_config.audit, &audit.action) {
        crate::audit::record::write(&state.db, audit, crate::audit::WriteMode::Once).await?;
    }
    crate::push_notifications::dispatch_notification_idempotent(
        state,
        &format!("purchase:{purchase_id}"),
        crate::push_notifications::DispatchNotificationInput {
            user_id: user_id.to_owned(),
            app_id: None,
            title: format!("Purchase Complete: {name}"),
            description: Some(format!("You now have access to {name}.")),
            icon: Some(if is_app { "shopping-bag" } else { "package" }.into()),
            image: None,
            link: Some(link.clone()),
            notification_type: NotificationType::System,
            source_run_id: None,
            source_node_id: None,
        },
    )
    .await?;
    if let (Some(mail), Some(email_addr)) = (&state.mail_client, buyer.email) {
        let client = state
            .stripe_client
            .as_ref()
            .ok_or_else(|| ApiError::service_unavailable("Stripe not configured"))?;
        let intent = stripe::PaymentIntent::retrieve(
            client,
            &intent_id
                .parse()
                .map_err(|_| ApiError::internal("Invalid stored PaymentIntent"))?,
            &["latest_charge"],
        )
        .await?;
        let receipt_url = match intent.latest_charge {
            Some(stripe::Expandable::Object(charge)) => charge.receipt_url,
            _ => None,
        };
        let amount = payload
            .get("amount")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApiError::internal("Missing purchase amount"))?;
        let currency = effect_string(payload, "currency")?;
        let display = format!("{}.{:02}", amount / 100, amount % 100);
        let (html, text) = crate::mail::templates::purchase_confirmation(
            &name,
            &link,
            &display,
            currency,
            receipt_url.as_deref(),
        );
        mail.send(crate::mail::EmailMessage {
            to: email_addr,
            subject: format!("Purchase Confirmed: {name}"),
            body_html: Some(html),
            body_text: Some(text),
        })
        .await
        .map_err(|_| ApiError::service_unavailable("Purchase email delivery needs a retry"))?;
    }
    Ok(())
}

fn older_subscription_event(incoming: i64, stored: Option<i64>) -> bool {
    stored.is_some_and(|latest| incoming < latest)
}

fn canonical_subscription<'a>(
    subscriptions: &'a [(
        stripe::Subscription,
        crate::entity::sea_orm_active_enums::UserTier,
    )],
    current_id: Option<&str>,
) -> Option<&'a (
    stripe::Subscription,
    crate::entity::sea_orm_active_enums::UserTier,
)> {
    let is_active = |subscription: &stripe::Subscription| {
        matches!(
            subscription.status,
            stripe::SubscriptionStatus::Active | stripe::SubscriptionStatus::Trialing
        )
    };
    subscriptions
        .iter()
        .filter(|(subscription, _)| is_active(subscription))
        .max_by_key(|(subscription, _)| {
            (
                current_id == Some(subscription.id.as_str()),
                subscription.created,
                subscription.id.as_str(),
            )
        })
        .or_else(|| {
            subscriptions
                .iter()
                .find(|(subscription, _)| current_id == Some(subscription.id.as_str()))
        })
        .or_else(|| {
            subscriptions
                .iter()
                .max_by_key(|(subscription, _)| (subscription.created, subscription.id.as_str()))
        })
}

async fn handle_subscription_change(
    state: &AppState,
    stripe_client: &stripe::Client,
    incoming: &stripe::Subscription,
    event_created: i64,
    event_id: &str,
) -> Result<(), ApiError> {
    use crate::entity::{sea_orm_active_enums::UserTier, user};
    let customer_id = match &incoming.customer {
        stripe::Expandable::Id(id) => id.clone(),
        stripe::Expandable::Object(customer) => customer.id.clone(),
    };
    // Stripe does not order webhook deliveries. Fetch current provider state outside
    // the account lock, then fence that snapshot with the persisted sync revision.
    for _ in 0..3 {
        let Some(snapshot) = user::Entity::find()
            .filter(user::Column::StripeId.eq(customer_id.as_str()))
            .one(&state.db)
            .await?
        else {
            return Ok(());
        };
        if older_subscription_event(event_created, snapshot.subscription_event_created_at) {
            return Ok(());
        }
        let mut cursor = None;
        let mut subscriptions = Vec::new();
        for page in 0..10 {
            let result = stripe::Subscription::list(
                stripe_client,
                &stripe::ListSubscriptions {
                    customer: Some(customer_id.clone()),
                    status: Some(stripe::SubscriptionStatusFilter::All),
                    limit: Some(100),
                    starting_after: cursor.clone(),
                    ..Default::default()
                },
            )
            .await?;
            cursor = result
                .data
                .last()
                .map(|subscription| subscription.id.clone());
            for subscription in result.data {
                if let Some(tier) = determine_tier_from_subscription(state, &subscription) {
                    subscriptions.push((subscription, tier));
                }
            }
            if !result.has_more {
                break;
            }
            if page == 9 || cursor.is_none() {
                return Err(ApiError::service_unavailable(
                    "Subscription history needs billing reconciliation",
                ));
            }
        }
        let Some((subscription, configured_tier)) =
            canonical_subscription(&subscriptions, snapshot.subscription_id.as_deref())
        else {
            return Ok(());
        };
        let new_tier = match subscription.status {
            stripe::SubscriptionStatus::Active | stripe::SubscriptionStatus::Trialing => {
                Some(configured_tier.clone())
            }
            stripe::SubscriptionStatus::Canceled
            | stripe::SubscriptionStatus::Unpaid
            | stripe::SubscriptionStatus::IncompleteExpired => Some(UserTier::Free),
            _ => None,
        };
        let payer_id = snapshot.id;
        let expected_revision = snapshot.subscription_sync_revision.unwrap_or(0);
        let canonical_id = subscription.id.to_string();
        let event_id = event_id.to_owned();
        let cycle_anchor = chrono::DateTime::from_timestamp(subscription.billing_cycle_anchor, 0)
            .map(|time| time.fixed_offset());
        let period_start = chrono::DateTime::from_timestamp(subscription.current_period_start, 0)
            .map(|time| time.fixed_offset());
        let period_end = chrono::DateTime::from_timestamp(subscription.current_period_end, 0)
            .map(|time| time.fixed_offset());
        let applied = crate::db::retry_transaction(
            &state.db,
            state.db_dialect,
            None,
            &crate::db::RetryPolicy::idempotent(),
            move |txn| {
                let payer_id = payer_id.clone();
                let new_tier = new_tier.clone();
                let canonical_id = canonical_id.clone();
                let event_id = event_id.clone();
                Box::pin(async move {
                    crate::db::coordination::coordinate(txn, "account-quota", &[&payer_id]).await?;
                    let Some(user_model) = user::Entity::find_by_id(&payer_id).one(txn).await?
                    else {
                        return Ok::<bool, ApiError>(true);
                    };
                    if older_subscription_event(
                        event_created,
                        user_model.subscription_event_created_at,
                    ) {
                        return Ok::<bool, ApiError>(true);
                    }
                    if user_model.subscription_sync_revision.unwrap_or(0) != expected_revision {
                        return Ok::<bool, ApiError>(false);
                    }
                    let existing_anchor = user_model.billing_period_anchor;
                    let next_revision = expected_revision
                        .checked_add(1)
                        .ok_or_else(|| ApiError::internal("Billing revision overflow"))?;
                    let mut active: user::ActiveModel = user_model.into();
                    active.subscription_id = Set(Some(canonical_id));
                    active.subscription_event_created_at = Set(Some(event_created));
                    active.subscription_event_id = Set(Some(event_id));
                    active.subscription_sync_revision = Set(Some(next_revision));
                    if let Some(new_tier) = new_tier {
                        let is_paid = new_tier != UserTier::Free;
                        active.tier = Set(new_tier);
                        active.billing_period_anchor = Set(if is_paid {
                            existing_anchor.or(cycle_anchor)
                        } else {
                            None
                        });
                        active.subscription_period_start =
                            Set(if is_paid { period_start } else { None });
                        active.subscription_period_end =
                            Set(if is_paid { period_end } else { None });
                    }
                    active.updated_at = Set(chrono::Utc::now().fixed_offset());
                    active.update(txn).await?;
                    Ok::<bool, ApiError>(true)
                })
            },
        )
        .await?;
        if applied {
            return Ok(());
        }
    }
    Err(ApiError::service_unavailable(
        "Concurrent subscription updates need a webhook retry",
    ))
}

fn determine_tier_from_subscription(
    state: &AppState,
    subscription: &stripe::Subscription,
) -> Option<crate::entity::sea_orm_active_enums::UserTier> {
    use crate::entity::sea_orm_active_enums::UserTier;

    for item in &subscription.items.data {
        if let Some(price) = &item.price {
            if let Some(product) = &price.product {
                let product_id = match product {
                    stripe::Expandable::Id(id) => id.to_string(),
                    stripe::Expandable::Object(p) => p.id.to_string(),
                };

                // Check product_id against hub config tiers
                for (tier_name, tier_config) in &state.platform_config.tiers {
                    if let Some(config_product_id) = &tier_config.product_id
                        && config_product_id == &product_id
                    {
                        return Some(match tier_name.to_uppercase().as_str() {
                            "ENTERPRISE" => UserTier::Enterprise,
                            "MAX" => UserTier::Max,
                            "PRO" => UserTier::Pro,
                            "PREMIUM" => UserTier::Premium,
                            "FREE" => UserTier::Free,
                            _ => return None,
                        });
                    }
                }
            }
        }
    }

    None
}

async fn handle_payment_intent_succeeded(
    state: &AppState,
    intent: &stripe::PaymentIntent,
) -> Result<(), ApiError> {
    use crate::entity::{solution_request, transaction, user};
    let intent_id = intent.id.to_string();
    let customer_id = intent
        .customer
        .as_ref()
        .map(|customer| customer.id().to_string());
    state
        .transaction(|txn| {
            let intent_id = intent_id.clone();
            let customer_id = customer_id.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "legacy-payment-intent", &[&intent_id])
                    .await?;
                if let Some(sol) = solution_request::Entity::find()
                    .filter(solution_request::Column::StripePaymentIntentId.eq(&intent_id))
                    .one(txn)
                    .await?
                {
                    let mut active: solution_request::ActiveModel = sol.into();
                    active.paid_deposit = Set(true);
                    active.priority = Set(true);
                    active.updated_at = Set(chrono::Utc::now().fixed_offset());
                    active.update(txn).await?;
                }
                if let Some(customer_id) = customer_id {
                    if let Some(buyer) = user::Entity::find()
                        .filter(user::Column::StripeId.eq(customer_id))
                        .one(txn)
                        .await?
                    {
                        if transaction::Entity::find()
                            .filter(transaction::Column::StripeId.eq(&intent_id))
                            .one(txn)
                            .await?
                            .is_none()
                        {
                            let now = chrono::Utc::now().fixed_offset();
                            transaction::ActiveModel {
                                id: Set(format!("stripe:{}", intent_id)),
                                user_id: Set(Some(buyer.id)),
                                stripe_id: Set(intent_id),
                                created_at: Set(now),
                                updated_at: Set(now),
                            }
                            .insert(txn)
                            .await?;
                        }
                    }
                }
                Ok::<_, ApiError>(())
            })
        })
        .await
}

async fn handle_payment_intent_failed(
    state: &AppState,
    intent: &stripe::PaymentIntent,
) -> Result<(), ApiError> {
    use crate::entity::solution_request;

    let intent_id = intent.id.to_string();

    tracing::warn!(
        payment_intent_id = %intent_id,
        "Processing payment_intent.payment_failed"
    );

    let solution = solution_request::Entity::find()
        .filter(solution_request::Column::StripePaymentIntentId.eq(&intent_id))
        .one(&state.db)
        .await?;

    if let Some(sol) = solution {
        let mut active: solution_request::ActiveModel = sol.into();
        active.admin_notes = Set(Some(format!(
            "Payment failed at {}",
            chrono::Utc::now().to_rfc3339()
        )));
        active.updated_at = Set(chrono::Utc::now().fixed_offset());
        active.update(&state.db).await?;
    }

    Ok(())
}

#[cfg(test)]
mod subscription_ordering_tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::UserTier;
    use std::str::FromStr;

    fn subscription(
        id: &str,
        status: stripe::SubscriptionStatus,
        created: i64,
        tier: UserTier,
    ) -> (stripe::Subscription, UserTier) {
        (
            stripe::Subscription {
                id: stripe::SubscriptionId::from_str(id).unwrap(),
                status,
                created,
                ..Default::default()
            },
            tier,
        )
    }

    #[test]
    fn old_subscription_cancellation_preserves_active_replacement() {
        let subscriptions = [
            subscription(
                "sub_old",
                stripe::SubscriptionStatus::Canceled,
                100,
                UserTier::Premium,
            ),
            subscription(
                "sub_new",
                stripe::SubscriptionStatus::Active,
                200,
                UserTier::Max,
            ),
        ];
        assert_eq!(
            canonical_subscription(&subscriptions, Some("sub_new"))
                .unwrap()
                .0
                .id
                .as_str(),
            "sub_new"
        );
        assert_eq!(
            canonical_subscription(&subscriptions, Some("sub_old"))
                .unwrap()
                .1,
            UserTier::Max
        );
    }

    #[test]
    fn older_update_cannot_override_newer_downgrade_watermark() {
        assert!(older_subscription_event(100, Some(101)));
        assert!(
            !older_subscription_event(101, Some(101)),
            "same-second events require canonical state and a sync-revision fence"
        );
        assert!(!older_subscription_event(101, None));
        let subscriptions = [subscription(
            "sub_current",
            stripe::SubscriptionStatus::Canceled,
            100,
            UserTier::Pro,
        )];
        assert_eq!(
            canonical_subscription(&subscriptions, Some("sub_current"))
                .unwrap()
                .0
                .status,
            stripe::SubscriptionStatus::Canceled
        );
    }

    #[test]
    fn same_second_duplicate_active_subscriptions_keep_the_billing_identity() {
        let subscriptions = [
            subscription(
                "sub_a",
                stripe::SubscriptionStatus::Active,
                100,
                UserTier::Pro,
            ),
            subscription(
                "sub_b",
                stripe::SubscriptionStatus::Active,
                100,
                UserTier::Max,
            ),
        ];
        assert_eq!(
            canonical_subscription(&subscriptions, Some("sub_a"))
                .unwrap()
                .0
                .id
                .as_str(),
            "sub_a"
        );
    }
}

#[cfg(test)]
mod legacy_payment_tests {
    use super::*;

    #[test]
    fn foreign_and_new_payments_are_filtered_before_legacy_decoding() {
        // This object deliberately cannot deserialize as the old SDK's CheckoutSession.
        let object =
            json!({"mode": "payment", "client_reference_id": "mkt:order", "future_field": true});
        assert!(!legacy_event_supported(
            "checkout.session.completed",
            None,
            &object
        ));
        assert!(!legacy_event_supported(
            "checkout.session.completed",
            Some("acct_other"),
            &json!({"mode":"payment"})
        ));
        for object in [
            json!({"metadata":{"flowlike_kind":"marketplace"}}),
            json!({"application":"ca_platform"}),
            json!({"transfer_data":{"destination":"acct_seller"}}),
            json!({"transfer_group":"mkt:order"}),
        ] {
            assert!(!legacy_event_supported(
                "payment_intent.succeeded",
                None,
                &object
            ));
        }
        assert!(!legacy_event_supported("future.unknown", None, &json!({})));
        assert!(legacy_event_supported(
            "payment_intent.succeeded",
            None,
            &json!({"application":null,"transfer_data":null})
        ));
        assert!(legacy_event_supported(
            "customer.subscription.updated",
            None,
            &json!({})
        ));
    }

    #[test]
    fn purchase_references_preserve_subjects_with_colons_and_reject_other_kinds() {
        assert_eq!(
            purchase_reference("app_purchase:oidc:subject:app"),
            Some(("app_purchase", "oidc:subject", "app"))
        );
        assert_eq!(
            purchase_reference("wasm_purchase:buyer:package"),
            Some(("wasm_purchase", "buyer", "package"))
        );
        for reference in [
            "unknown:user:app",
            "mkt:order",
            "app_purchase::app",
            "app_purchase:user:",
            "app_purchase:user",
            "solution",
        ] {
            assert!(purchase_reference(reference).is_none(), "{reference}");
        }
    }

    #[test]
    fn paid_purchase_requires_amount_and_currency_without_defaulting_missing_money() {
        let mut session = stripe::CheckoutSession {
            amount_total: Some(500),
            amount_subtotal: Some(600),
            currency: Some(stripe::Currency::EUR),
            ..Default::default()
        };
        assert_eq!(purchase_money(&session).unwrap(), (500, 600, "EUR".into()));
        session.amount_total = Some(0);
        assert!(purchase_money(&session).is_err());
        session.amount_total = Some(500);
        session.amount_subtotal = Some(100);
        assert!(purchase_money(&session).is_err());
        session.amount_subtotal = None;
        session.currency = None;
        assert!(purchase_money(&session).is_err());
    }
}
