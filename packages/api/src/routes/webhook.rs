use crate::{error::ApiError, state::AppState};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use flow_like_types::anyhow;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use stripe::{Event, EventObject, EventType, Webhook};

fn get_stripe_webhook_secret() -> Option<String> {
    std::env::var("STRIPE_WEBHOOK_SECRET").ok()
}

#[tracing::instrument(name = "POST /webhook/stripe", skip(state, headers, payload))]
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or(anyhow!("Stripe not configured"))?;

    let webhook_secret =
        get_stripe_webhook_secret().ok_or(anyhow!("Webhook secret not configured"))?;

    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(anyhow!("Missing stripe-signature header"))?;

    let payload_str =
        std::str::from_utf8(&payload).map_err(|_| anyhow!("Invalid UTF-8 in payload"))?;

    let event = Webhook::construct_event(payload_str, signature, &webhook_secret)
        .map_err(|e| anyhow!("Failed to verify webhook signature: {}", e))?;

    let event_id = event.id.to_string();
    if is_event_processed(&state, &event_id).await? {
        tracing::info!(event_id = %event_id, "Duplicate event, skipping");
        return Ok(StatusCode::OK);
    }

    match handle_stripe_event(&state, stripe_client, &event).await {
        Ok(_) => {
            mark_event_processed(&state, &event_id, &event.type_.to_string()).await?;
            Ok(StatusCode::OK)
        }
        Err(e) => {
            tracing::error!(event_id = %event_id, "Failed to process webhook");
            Err(e)
        }
    }
}

async fn is_event_processed(state: &AppState, event_id: &str) -> Result<bool, ApiError> {
    use crate::entity::stripe_event;

    let existing = stripe_event::Entity::find_by_id(event_id)
        .one(&state.db)
        .await?;

    Ok(existing.is_some())
}

async fn mark_event_processed(
    state: &AppState,
    event_id: &str,
    event_type: &str,
) -> Result<(), ApiError> {
    use crate::entity::stripe_event;

    let new_event = stripe_event::ActiveModel {
        id: Set(event_id.to_string()),
        event_type: Set(event_type.to_string()),
        processed_at: Set(chrono::Utc::now().naive_utc()),
    };

    new_event.insert(&state.db).await?;
    Ok(())
}

async fn handle_stripe_event(
    state: &AppState,
    _stripe_client: &stripe::Client,
    event: &Event,
) -> Result<(), ApiError> {
    match event.type_ {
        EventType::CheckoutSessionCompleted => {
            if let EventObject::CheckoutSession(session) = &event.data.object {
                handle_checkout_completed(state, session).await?;
            }
        }
        EventType::CheckoutSessionExpired => {
            if let EventObject::CheckoutSession(session) = &event.data.object {
                handle_checkout_expired(state, session).await?;
            }
        }
        EventType::CustomerSubscriptionCreated
        | EventType::CustomerSubscriptionUpdated
        | EventType::CustomerSubscriptionDeleted => {
            if let EventObject::Subscription(subscription) = &event.data.object {
                handle_subscription_change(state, subscription, &event.type_).await?;
            }
        }
        EventType::PaymentIntentSucceeded => {
            if let EventObject::PaymentIntent(intent) = &event.data.object {
                handle_payment_intent_succeeded(state, intent).await?;
            }
        }
        EventType::PaymentIntentPaymentFailed => {
            if let EventObject::PaymentIntent(intent) = &event.data.object {
                handle_payment_intent_failed(state, intent).await?;
            }
        }
        _ => {
            tracing::debug!(event_type = %event.type_, "Unhandled event type");
        }
    }

    Ok(())
}

async fn handle_checkout_completed(
    state: &AppState,
    session: &stripe::CheckoutSession,
) -> Result<(), ApiError> {
    use crate::entity::solution_request;

    let session_id = session.id.to_string();

    tracing::info!(
        session_id = %session_id,
        client_reference_id = ?session.client_reference_id,
        mode = ?session.mode,
        payment_status = ?session.payment_status,
        "Processing checkout.session.completed"
    );

    let client_ref = session
        .client_reference_id
        .as_ref()
        .ok_or(anyhow!("Missing client_reference_id"))?;

    // Check if this is an app purchase (format: "app_purchase:{user_id}:{app_id}")
    if client_ref.starts_with("app_purchase:") {
        return handle_app_purchase_completed(state, session, client_ref).await;
    }

    // Check if this is a WASM package purchase (format: "wasm_purchase:{user_id}:{package_id}")
    if client_ref.starts_with("wasm_purchase:") {
        return handle_wasm_purchase_completed(state, session, client_ref).await;
    }

    // Otherwise, handle as solution request
    let submission_id = client_ref;

    let existing = solution_request::Entity::find_by_id(submission_id.clone())
        .one(&state.db)
        .await?;

    if let Some(solution) = existing {
        let mut active: solution_request::ActiveModel = solution.into();

        active.stripe_checkout_session_id = Set(Some(session_id.clone()));

        if let Some(pi) = &session.payment_intent {
            active.stripe_payment_intent_id = Set(Some(pi.id().to_string()));
        }

        if let Some(si) = &session.setup_intent {
            active.stripe_setup_intent_id = Set(Some(si.id().to_string()));
        }

        // Mark deposit as paid and move to PendingReview (in queue)
        active.paid_deposit = Set(true);
        active.status = Set(crate::entity::sea_orm_active_enums::SolutionStatus::PendingReview);
        active.updated_at = Set(chrono::Utc::now().naive_utc());

        active.update(&state.db).await?;

        tracing::info!(
            submission_id = %submission_id,
            "Solution request updated: paid_deposit=true, status=PENDING_REVIEW"
        );
    } else {
        tracing::warn!(
            submission_id = %submission_id,
            "Solution request not found for checkout session"
        );
    }

    Ok(())
}

/// Handle app purchase completion - create membership and send notification
async fn handle_app_purchase_completed(
    state: &AppState,
    session: &stripe::CheckoutSession,
    client_ref: &str,
) -> Result<(), ApiError> {
    use crate::entity::{
        app, app_purchase, membership, meta, notification,
        sea_orm_active_enums::{NotificationType, PurchaseStatus},
    };

    let session_id = session.id.to_string();

    // Parse client_ref: "app_purchase:{user_id}:{app_id}"
    let parts: Vec<&str> = client_ref.split(':').collect();
    if parts.len() != 3 {
        tracing::error!(
            client_ref = %client_ref,
            "Invalid app_purchase client_reference_id format"
        );
        return Err(anyhow!("Invalid client_reference_id format").into());
    }

    let user_id = parts[1];
    let app_id = parts[2];

    tracing::info!(
        session_id = %session_id,
        user_id = %user_id,
        app_id = %app_id,
        "Processing app purchase completion"
    );

    // Check if purchase already exists (idempotency)
    let existing_purchase = app_purchase::Entity::find()
        .filter(app_purchase::Column::StripeSessionId.eq(&session_id))
        .one(&state.db)
        .await?;

    if existing_purchase.is_some() {
        tracing::info!(
            session_id = %session_id,
            "Purchase already recorded, skipping (idempotent)"
        );
        return Ok(());
    }

    // Check if user already has membership (idempotency)
    let existing_membership = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::UserId.eq(user_id))
        .one(&state.db)
        .await?;

    if existing_membership.is_some() {
        tracing::info!(
            user_id = %user_id,
            app_id = %app_id,
            "User already has membership, skipping creation (idempotent)"
        );
        return Ok(());
    }

    // Get the app to find the default role and price
    let app_model = app::Entity::find_by_id(app_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| {
            tracing::error!(app_id = %app_id, "App not found for purchase");
            anyhow!("App not found")
        })?;

    let default_role_id = app_model.default_role_id.clone().ok_or_else(|| {
        tracing::error!(app_id = %app_id, "App has no default role");
        anyhow!("App has no default role")
    })?;

    // Extract price info from session
    let amount_total = session.amount_total.unwrap_or(0);
    let amount_subtotal = session.amount_subtotal.unwrap_or(amount_total);
    let discount_amount = amount_subtotal - amount_total;
    let currency = session
        .currency
        .map(|c| c.to_string().to_uppercase())
        .unwrap_or_else(|| "EUR".to_string());

    // Create purchase record
    let purchase_id = flow_like_types::create_id();
    let now = chrono::Utc::now().naive_utc();

    let new_purchase = app_purchase::ActiveModel {
        id: Set(purchase_id.clone()),
        user_id: Set(user_id.to_string()),
        app_id: Set(app_id.to_string()),
        price_paid: Set(amount_total),
        original_price: Set(amount_subtotal),
        discount_amount: Set(discount_amount.max(0)),
        discount_id: Set(None), // Could be enhanced to extract discount code from session
        currency: Set(currency),
        stripe_session_id: Set(session_id.clone()),
        stripe_payment_intent_id: Set(session
            .payment_intent
            .as_ref()
            .map(|pi| pi.id().to_string())),
        status: Set(PurchaseStatus::Completed),
        completed_at: Set(Some(now)),
        refunded_at: Set(None),
        refund_reason: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    new_purchase.insert(&state.db).await?;

    tracing::info!(
        purchase_id = %purchase_id,
        user_id = %user_id,
        app_id = %app_id,
        amount = %amount_total,
        "Created purchase record"
    );

    // Create membership
    let membership_id = flow_like_types::create_id();
    let new_membership = membership::ActiveModel {
        id: Set(membership_id.clone()),
        user_id: Set(user_id.to_string()),
        app_id: Set(app_id.to_string()),
        role_id: Set(default_role_id),
        created_at: Set(now),
        updated_at: Set(now),
        joined_via: Set(Some(format!("purchase:{}", session_id))),
    };

    new_membership.insert(&state.db).await?;

    tracing::info!(
        membership_id = %membership_id,
        user_id = %user_id,
        app_id = %app_id,
        "Created membership from purchase"
    );

    // Get app name for notification
    let app_name = meta::Entity::find()
        .filter(meta::Column::AppId.eq(Some(app_id.to_string())))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
        .map(|m| m.name)
        .unwrap_or_else(|| "the app".to_string());

    // Build app link URL for notification - link directly to "use" page since they own it now
    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let app_link_url = format!("{}/use?id={}", frontend_url, app_id);

    // Create notification for the user
    let notification_id = flow_like_types::create_id();
    let notification_model = notification::ActiveModel {
        id: Set(notification_id.clone()),
        user_id: Set(user_id.to_string()),
        app_id: Set(Some(app_id.to_string())),
        title: Set(format!("Purchase Complete: {}", app_name)),
        description: Set(Some(format!(
            "You now have access to {}. Click to start using the app.",
            app_name
        ))),
        icon: Set(Some("shopping-bag".to_string())),
        link: Set(Some(app_link_url)),
        r#type: Set(NotificationType::System),
        read: Set(false),
        source_run_id: Set(None),
        source_node_id: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
        read_at: Set(None),
    };

    notification_model.insert(&state.db).await?;

    tracing::info!(
        notification_id = %notification_id,
        user_id = %user_id,
        app_id = %app_id,
        "Created purchase notification"
    );

    Ok(())
}

/// Handle WASM package purchase completion — create purchase record and grant access
async fn handle_wasm_purchase_completed(
    state: &AppState,
    session: &stripe::CheckoutSession,
    client_ref: &str,
) -> Result<(), ApiError> {
    use crate::entity::{
        notification, user, wasm_package, wasm_package_purchase, wasm_package_user,
        sea_orm_active_enums::{NotificationType, PurchaseStatus},
    };
    use crate::mail::{EmailMessage, templates::purchase_confirmation};

    let session_id = session.id.to_string();

    // Parse "wasm_purchase:{user_id}:{package_id}"
    let parts: Vec<&str> = client_ref.split(':').collect();
    if parts.len() != 3 {
        tracing::error!(client_ref = %client_ref, "Invalid wasm_purchase client_reference_id");
        return Err(anyhow!("Invalid client_reference_id format").into());
    }

    let user_id = parts[1];
    let package_id = parts[2];

    tracing::info!(
        session_id = %session_id,
        user_id = %user_id,
        package_id = %package_id,
        "Processing WASM package purchase completion"
    );

    // Idempotency: check existing purchase
    let existing_purchase = wasm_package_purchase::Entity::find()
        .filter(wasm_package_purchase::Column::StripeSessionId.eq(&session_id))
        .one(&state.db)
        .await?;

    if existing_purchase.is_some() {
        tracing::info!(session_id = %session_id, "WASM purchase already recorded (idempotent)");
        return Ok(());
    }

    // Idempotency: check existing access
    let existing_access = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(package_id))
        .filter(wasm_package_user::Column::UserId.eq(user_id))
        .one(&state.db)
        .await?;

    if existing_access.is_some() {
        tracing::info!(
            user_id = %user_id,
            package_id = %package_id,
            "User already has access (idempotent)"
        );
        return Ok(());
    }

    let amount_total = session.amount_total.unwrap_or(0);
    let amount_subtotal = session.amount_subtotal.unwrap_or(amount_total);
    let discount_amount = amount_subtotal - amount_total;
    let currency = session
        .currency
        .map(|c| c.to_string().to_uppercase())
        .unwrap_or_else(|| "EUR".to_string());

    let now = chrono::Utc::now().naive_utc();
    let purchase_id = flow_like_types::create_id();

    wasm_package_purchase::ActiveModel {
        id: Set(purchase_id.clone()),
        user_id: Set(user_id.to_string()),
        package_id: Set(package_id.to_string()),
        price_paid: Set(amount_total),
        original_price: Set(amount_subtotal),
        discount_amount: Set(discount_amount.max(0)),
        currency: Set(currency.clone()),
        stripe_session_id: Set(session_id.clone()),
        stripe_payment_intent_id: Set(session
            .payment_intent
            .as_ref()
            .map(|pi| pi.id().to_string())),
        status: Set(PurchaseStatus::Completed),
        completed_at: Set(Some(now)),
        refunded_at: Set(None),
        refund_reason: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    tracing::info!(
        purchase_id = %purchase_id,
        user_id = %user_id,
        package_id = %package_id,
        amount = %amount_total,
        "Created WASM purchase record"
    );

    // Grant User-level access
    let access_id = flow_like_types::create_id();
    wasm_package_user::ActiveModel {
        id: Set(access_id.clone()),
        user_id: Set(user_id.to_string()),
        package_id: Set(package_id.to_string()),
        permission: Set(
            crate::permission::wasm_package_permission::WasmPackagePermission::User.bits(),
        ),
        granted_by: Set(Some(format!("purchase:{}", session_id))),
        granted_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    // Invalidate permission cache
    state.invalidate_wasm_permission(user_id, package_id);

    tracing::info!(
        access_id = %access_id,
        user_id = %user_id,
        package_id = %package_id,
        "Granted WASM package access from purchase"
    );

    // Send notification
    let notification_id = flow_like_types::create_id();
    let frontend_url =
        std::env::var("FRONTEND_URL").unwrap_or_else(|_| "https://app.flow-like.com".to_string());
    let link = format!("{}/nodes?id={}", frontend_url, package_id);

    notification::ActiveModel {
        id: Set(notification_id),
        user_id: Set(user_id.to_string()),
        app_id: Set(None),
        title: Set("WASM Package Purchase Complete".to_string()),
        description: Set(Some(
            "You now have access to the purchased WASM package.".to_string(),
        )),
        icon: Set(Some("package".to_string())),
        link: Set(Some(link.clone())),
        r#type: Set(NotificationType::System),
        read: Set(false),
        source_run_id: Set(None),
        source_node_id: Set(None),
        created_at: Set(now),
        read_at: Set(None),
    }
    .insert(&state.db)
    .await?;

    // Send purchase confirmation email
    if let Some(mail_client) = &state.mail_client {
        let buyer_email = user::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .and_then(|u| u.email);

        let pkg_name = wasm_package::Entity::find_by_id(package_id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .map(|p| p.name)
            .unwrap_or_else(|| package_id.to_string());

        if let Some(email_addr) = buyer_email {
            let price_display = format!("{:.2}", amount_total as f64 / 100.0);
            let receipt_url = session
                .payment_intent
                .as_ref()
                .map(|pi| format!("https://dashboard.stripe.com/payments/{}", pi.id()));

            let (html, text) = purchase_confirmation(
                &pkg_name,
                &link,
                &price_display,
                &currency,
                receipt_url.as_deref(),
            );

            let email = EmailMessage {
                to: email_addr,
                subject: format!("Purchase Confirmed — {}", pkg_name),
                body_html: Some(html),
                body_text: Some(text),
            };

            if let Err(e) = mail_client.send(email).await {
                tracing::warn!(error = %e, "Failed to send purchase confirmation email");
            }
        }
    }

    Ok(())
}

async fn handle_checkout_expired(
    state: &AppState,
    session: &stripe::CheckoutSession,
) -> Result<(), ApiError> {
    use crate::entity::solution_request;

    let session_id = session.id.to_string();

    if let Some(submission_id) = &session.client_reference_id {
        let existing = solution_request::Entity::find_by_id(submission_id.clone())
            .one(&state.db)
            .await?;

        if let Some(solution) = existing {
            let mut active: solution_request::ActiveModel = solution.into();
            active.status = Set(crate::entity::sea_orm_active_enums::SolutionStatus::Cancelled);
            active.updated_at = Set(chrono::Utc::now().naive_utc());
            active.update(&state.db).await?;

            tracing::info!(
                submission_id = %submission_id,
                session_id = %session_id,
                "Solution request marked as CANCELLED due to expired checkout"
            );
        }
    }

    Ok(())
}

async fn handle_subscription_change(
    state: &AppState,
    subscription: &stripe::Subscription,
    event_type: &EventType,
) -> Result<(), ApiError> {
    use crate::entity::{sea_orm_active_enums::UserTier, user};

    let customer_id = match &subscription.customer {
        stripe::Expandable::Id(id) => id.to_string(),
        stripe::Expandable::Object(c) => c.id.to_string(),
    };

    tracing::info!(
        subscription_id = %subscription.id,
        customer_id = %customer_id,
        event_type = %event_type,
        status = ?subscription.status,
        "Processing subscription change"
    );

    let user_result = user::Entity::find()
        .filter(user::Column::StripeId.eq(&customer_id))
        .one(&state.db)
        .await?;

    if let Some(user_model) = user_result {
        let new_tier = match subscription.status {
            stripe::SubscriptionStatus::Active | stripe::SubscriptionStatus::Trialing => {
                determine_tier_from_subscription(state, subscription)
            }
            stripe::SubscriptionStatus::Canceled
            | stripe::SubscriptionStatus::Unpaid
            | stripe::SubscriptionStatus::IncompleteExpired => UserTier::Free,
            _ => return Ok(()),
        };

        let mut active: user::ActiveModel = user_model.into();
        active.tier = Set(new_tier.clone());
        active.updated_at = Set(chrono::Utc::now().naive_utc());
        active.update(&state.db).await?;

        tracing::info!(
            customer_id = %customer_id,
            new_tier = ?new_tier,
            "User tier updated based on subscription"
        );
    } else {
        tracing::warn!(
            customer_id = %customer_id,
            "No user found for Stripe customer"
        );
    }

    Ok(())
}

fn determine_tier_from_subscription(
    state: &AppState,
    subscription: &stripe::Subscription,
) -> crate::entity::sea_orm_active_enums::UserTier {
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
                        return match tier_name.to_uppercase().as_str() {
                            "ENTERPRISE" => UserTier::Enterprise,
                            "PRO" => UserTier::Pro,
                            "PREMIUM" => UserTier::Premium,
                            _ => UserTier::Free,
                        };
                    }
                }

                // Fallback: check if product_id contains tier name
                let product_lower = product_id.to_lowercase();
                if product_lower.contains("enterprise") {
                    return UserTier::Enterprise;
                } else if product_lower.contains("pro") {
                    return UserTier::Pro;
                } else if product_lower.contains("premium") {
                    return UserTier::Premium;
                }
            }

            // Also check price metadata for tier info
            if let Some(metadata) = &price.metadata
                && let Some(tier) = metadata.get("tier")
            {
                match tier.to_uppercase().as_str() {
                    "ENTERPRISE" => return UserTier::Enterprise,
                    "PRO" => return UserTier::Pro,
                    "PREMIUM" => return UserTier::Premium,
                    _ => {}
                }
            }
        }
    }

    // Default to Free if no matching tier found
    UserTier::Free
}

async fn handle_payment_intent_succeeded(
    state: &AppState,
    intent: &stripe::PaymentIntent,
) -> Result<(), ApiError> {
    use crate::entity::{solution_request, transaction};

    let intent_id = intent.id.to_string();

    tracing::info!(
        payment_intent_id = %intent_id,
        amount = ?intent.amount,
        "Processing payment_intent.succeeded"
    );

    let solution = solution_request::Entity::find()
        .filter(solution_request::Column::StripePaymentIntentId.eq(&intent_id))
        .one(&state.db)
        .await?;

    if let Some(sol) = solution {
        let mut active: solution_request::ActiveModel = sol.into();
        active.paid_deposit = Set(true);
        active.priority = Set(true);
        active.updated_at = Set(chrono::Utc::now().naive_utc());
        active.update(&state.db).await?;

        tracing::info!(
            payment_intent_id = %intent_id,
            "Solution request marked as paid with priority"
        );
    }

    if let Some(customer) = &intent.customer {
        let customer_id = match customer {
            stripe::Expandable::Id(id) => id.to_string(),
            stripe::Expandable::Object(c) => c.id.to_string(),
        };

        let user = crate::entity::user::Entity::find()
            .filter(crate::entity::user::Column::StripeId.eq(&customer_id))
            .one(&state.db)
            .await?;

        if let Some(u) = user {
            let existing_tx = transaction::Entity::find()
                .filter(transaction::Column::StripeId.eq(&intent_id))
                .one(&state.db)
                .await?;

            if existing_tx.is_none() {
                let new_tx = transaction::ActiveModel {
                    id: Set(flow_like_types::create_id()),
                    user_id: Set(Some(u.id.clone())),
                    stripe_id: Set(intent_id.clone()),
                    created_at: Set(chrono::Utc::now().naive_utc()),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                };
                new_tx.insert(&state.db).await?;

                tracing::info!(
                    user_id = %u.id,
                    payment_intent_id = %intent_id,
                    "Transaction recorded"
                );
            }
        }
    }

    Ok(())
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
        active.updated_at = Set(chrono::Utc::now().naive_utc());
        active.update(&state.db).await?;
    }

    Ok(())
}
