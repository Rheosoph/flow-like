//! Marketplace orders for apps and registry packages retain their offer and
//! financial payee through servicing.

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::Utc;
use flow_like::hub::{PaymentFeeBasis, PaymentTaxMode};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::{
    accounts, connect_scope, domain, ensure_app_owner, error, operations, outbox, payee_for_app,
    payee_for_package, sql, stripe_error,
};
use crate::{
    entity::{
        app, connected_account, legal_consent, payment_attempt, payment_order,
        sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility},
        wasm_package,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    stripe_connect::{ChargeModel, PaymentMethodPolicy, StripeRequest, StripeScope},
};

#[path = "marketplace_settlement.rs"]
mod settlement;
pub(crate) use settlement::reserve_refund_txn;
pub use settlement::{
    handle_effect, handle_event, observe_refund, reconcile, reconcile_node_tax, reconcile_order,
    reconcile_refund, request_refund, request_seller_refund,
};

pub const SOURCE: &str = "MARKETPLACE";

/// What a marketplace order sells. Stored as `PaymentOrder.kind` and the
/// `itemKind` of entitlements and grants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ItemKind {
    #[default]
    App,
    Package,
}

impl ItemKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::App => "APP",
            Self::Package => "PACKAGE",
        }
    }

    pub(crate) fn of(order: &payment_order::Model) -> Self {
        if order.kind == "PACKAGE" {
            Self::Package
        } else {
            Self::App
        }
    }

    /// Serialises checkout and settlement per product.
    pub(crate) fn lock(self) -> &'static str {
        match self {
            Self::App => "payments-app",
            Self::Package => "payments-package",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Package => "package",
        }
    }
}

#[cfg(test)]
mod checkout_tests {
    use super::*;

    fn offer(platform_owned: bool) -> Offer {
        Offer {
            kind: ItemKind::App,
            item_id: "app".into(),
            buyer: "buyer".into(),
            buyer_email: Some("buyer@example.com".into()),
            payee: "owner".into(),
            platform_owned,
            account_id: (!platform_owned).then(|| "acct_seller".into()),
            account_row_id: (!platform_owned).then(|| "connected-row".into()),
            account_revision: 0,
            amount: 1190,
            fee: if platform_owned { 0 } else { 119 },
            fee_bps: if platform_owned { 0 } else { 1000 },
            fee_basis: PaymentFeeBasis::Gross,
            tax_mode: PaymentTaxMode::PlatformSupplier,
            title: "App access".into(),
            role_id: "member".into(),
            terms_version: "v1".into(),
            terms_hash: "hash".into(),
            terms_text: "Test terms".into(),
            locale: "en".into(),
            withdrawal_waiver: false,
            withdrawal_days: Some(14),
            withdrawal_deadline: 1_800_000,
            method_policy: PaymentMethodPolicy::cards_and_wallets(
                "marketplace-v1",
                if platform_owned {
                    ChargeModel::Platform
                } else {
                    ChargeModel::Destination
                },
            ),
            product_tax_code: Some("txcd_test".into()),
        }
    }

    #[test]
    fn platform_checkout_keeps_proceeds_without_connect_parameters() {
        let offer = offer(true);
        let parameters = checkout_parameters(
            &offer,
            "order",
            1_800_000,
            "https://app.example.com/store",
            "https://app.example.com/store?canceled=1",
        )
        .unwrap();
        let intent = &parameters["payment_intent_data"];
        for field in [
            "application_fee_amount",
            "transfer_data",
            "transfer_group",
            "on_behalf_of",
        ] {
            assert!(intent.get(field).is_none(), "unexpected {field}");
        }
        assert_eq!(
            parameters["line_items"][0]["price_data"]["unit_amount"],
            1190
        );
        assert_eq!(parameters["automatic_tax"]["liability"]["type"], "self");
        assert_eq!(parameters["automatic_tax"]["enabled"], true);
        assert_eq!(parameters["customer_creation"], "always");
        assert_eq!(parameters["billing_address_collection"], "required");
        assert_eq!(
            parameters["line_items"][0]["price_data"]["tax_behavior"],
            "inclusive"
        );
        assert_eq!(
            parameters["invoice_creation"]["invoice_data"]["issuer"]["type"],
            "self"
        );
        assert_eq!(intent["metadata"]["flowlike_id"], "order");
    }

    #[test]
    fn connected_checkout_retains_destination_and_fee() {
        let offer = offer(false);
        let parameters = checkout_parameters(
            &offer,
            "order",
            1_800_000,
            "https://app.example.com/store",
            "https://app.example.com/store?canceled=1",
        )
        .unwrap();
        assert_eq!(
            parameters["payment_intent_data"]["transfer_data"]["destination"],
            "acct_seller"
        );
        assert_eq!(
            parameters["payment_intent_data"]["application_fee_amount"],
            119
        );
        assert_eq!(
            parameters["payment_intent_data"]["transfer_group"],
            "mkt:order"
        );
    }

    #[test]
    fn recorded_connected_offer_does_not_become_platform_owned() {
        let recorded = offer(false);
        let mut snapshot = serde_json::to_value(&recorded).unwrap();
        snapshot.as_object_mut().unwrap().remove("platform_owned");
        let restored: Offer = serde_json::from_value(snapshot).unwrap();
        assert!(!restored.platform_owned);
        assert_eq!(restored.account_id.as_deref(), Some("acct_seller"));
        assert!(same_offer(&recorded, &restored));
        assert!(!same_offer(&recorded, &offer(true)));
    }

    #[test]
    fn product_tax_classification_is_required_and_changes_the_offer() {
        let original = offer(true);
        let mut changed = original.clone();
        changed.product_tax_code = Some("txcd_different".into());
        assert!(!same_offer(&original, &changed));
        changed.product_tax_code = None;
        assert!(
            checkout_parameters(
                &changed,
                "order",
                1_800_000,
                "https://app.example.com/store",
                "https://app.example.com/store"
            )
            .is_err()
        );
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutInput {
    pub terms_version: String,
    pub terms_accepted: bool,
    pub locale: Option<String>,
    #[serde(default)]
    pub withdrawal_waiver: bool,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefundInput {
    pub command_id: String,
    pub amount: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct Confirmation {
    pub confirm: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PurchaseQuery {
    pub before: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Offer {
    #[serde(default)]
    pub kind: ItemKind,
    /// The app or package id. Serialised as `app_id`, which is what snapshots
    /// written before packages were sold carry.
    #[serde(rename = "app_id")]
    pub item_id: String,
    pub buyer: String,
    #[serde(default)]
    pub buyer_email: Option<String>,
    pub payee: String,
    #[serde(default)]
    pub platform_owned: bool,
    pub account_id: Option<String>,
    pub account_row_id: Option<String>,
    pub account_revision: i64,
    pub amount: i64,
    pub fee: i64,
    pub fee_bps: u16,
    pub fee_basis: PaymentFeeBasis,
    pub tax_mode: PaymentTaxMode,
    pub title: String,
    /// The buyer role of an app; empty for packages.
    #[serde(default)]
    pub role_id: String,
    pub terms_version: String,
    pub terms_hash: String,
    pub terms_text: String,
    pub locale: String,
    pub withdrawal_waiver: bool,
    #[serde(default)]
    pub withdrawal_days: Option<u16>,
    pub withdrawal_deadline: i64,
    pub method_policy: PaymentMethodPolicy,
    pub product_tax_code: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/apps/{app_id}/marketplace/checkout", post(checkout))
        .route(
            "/apps/{app_id}/marketplace/terms",
            post(accept_seller_terms),
        )
        .route(
            "/apps/{app_id}/marketplace/requests/{user_id}/approve",
            post(approve),
        )
        .route("/apps/{app_id}/marketplace/comp/{user_id}", post(comp))
        .route("/user/purchases", get(list_purchases))
        .route("/user/purchases/{id}", get(get_purchase))
        .route("/user/purchases/{id}/cancel", post(cancel))
        .route("/user/purchases/{id}/withdraw", post(withdraw))
        .route(
            "/user/payments/legacy-purchases/{kind}/{id}/receipt",
            get(super::legacy_checkout::receipt),
        )
        .route(
            "/user/payments/legacy-purchases",
            get(super::legacy_checkout::history),
        )
        .route("/user/sales", get(list_sales))
        .route("/user/sales/{id}/refund", post(refund_sale))
}

fn payer(user: &AppUser) -> Result<String, ApiError> {
    match user {
        AppUser::OpenID(user) => Ok(user.sub.clone()),
        _ => Err(ApiError::forbidden(
            "Sign in to purchase or manage a payment",
        )),
    }
}

fn now() -> i64 {
    Utc::now().timestamp_millis()
}

pub(super) fn order_scope(order: &payment_order::Model) -> StripeScope {
    StripeScope::platform(&order.platform_account_id, order.livemode)
}

pub(super) async fn load_order<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<payment_order::Model, ApiError> {
    payment_order::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)
}

fn legal_text(
    state: &AppState,
    kind: &str,
    version: &str,
    locale: &str,
) -> Result<String, ApiError> {
    let text = state
        .platform_config
        .payments
        .legal_texts
        .iter()
        .find(|text| text.kind == kind && text.version == version && text.locale == locale)
        .ok_or_else(|| {
            error(
                "PAYMENT_TERMS_REQUIRED",
                "The current payment terms are unavailable",
            )
        })?;
    text.content()
        .map(str::to_owned)
        .map_err(|message| error("PAYMENT_TERMS_REQUIRED", &message))
}

/// What the buyer is about to pay for, read fresh at checkout.
struct Listing {
    price: i64,
    payee: String,
    title: String,
    role_id: String,
}

async fn listing(
    state: &AppState,
    kind: ItemKind,
    item_id: &str,
    locale: &str,
) -> Result<Listing, ApiError> {
    match kind {
        ItemKind::App => {
            let product = app::Entity::find_by_id(item_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            let payee = payee_for_app(&state.db, item_id).await?;
            let title = state.db.query_one_raw(sql(r#"SELECT name FROM "Meta" WHERE "appId"=$1 ORDER BY CASE WHEN lang=$2 THEN 0 ELSE 1 END LIMIT 1"#, vec![item_id.into(),locale.into()])).await?.and_then(|row| row.try_get::<String>("", "name").ok()).unwrap_or_else(|| "App access".into());
            Ok(Listing {
                price: product.price,
                payee,
                title,
                role_id: product
                    .default_role_id
                    .ok_or_else(|| error("LISTING_UNAVAILABLE", "The app has no buyer role"))?,
            })
        }
        ItemKind::Package => {
            let product = wasm_package::Entity::find_by_id(item_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if product.visibility == WasmPackageVisibility::PublicRequestAccess {
                return Err(error(
                    "APPROVAL_REQUIRED",
                    "The package maintainers grant access to this package. Request access instead.",
                ));
            }
            if product.visibility != WasmPackageVisibility::Public
                || product.status != WasmPackageStatus::Active
            {
                return Err(error("LISTING_UNAVAILABLE", "This package is not for sale"));
            }
            let payee = payee_for_package(&state.db, item_id).await?;
            let title = state.db.query_one_raw(sql(r#"SELECT name FROM "Meta" WHERE "wasmPackageId"=$1 ORDER BY CASE WHEN lang=$2 THEN 0 ELSE 1 END LIMIT 1"#, vec![item_id.into(),locale.into()])).await?.and_then(|row| row.try_get::<String>("", "name").ok()).unwrap_or(product.name);
            Ok(Listing {
                price: product.price,
                payee,
                title,
                role_id: String::new(),
            })
        }
    }
}

#[utoipa::path(post, path="/apps/{app_id}/marketplace/checkout", tag="payments", request_body=CheckoutInput, responses((status=200,description="Persisted marketplace order")))]
pub async fn checkout(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(input): Json<CheckoutInput>,
) -> Result<Json<Value>, ApiError> {
    start_checkout(state, user, ItemKind::App, app_id, input).await
}

#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/marketplace/checkout",
    tag = "payments",
    description = "Buy a package. The package owner is paid through their payout account, minus the platform fee.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = CheckoutInput,
    responses((status = 200, description = "Persisted marketplace order")),
    security(("bearer_auth" = []))
)]
pub async fn checkout_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(input): Json<CheckoutInput>,
) -> Result<Json<Value>, ApiError> {
    start_checkout(state, user, ItemKind::Package, package_id, input).await
}

async fn start_checkout(
    state: AppState,
    user: AppUser,
    kind: ItemKind,
    item_id: String,
    input: CheckoutInput,
) -> Result<Json<Value>, ApiError> {
    let buyer = payer(&user)?;
    let config = &state.platform_config.payments;
    if !config.marketplace_enabled {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Marketplace checkout is not enabled",
        ));
    }
    config
        .validate()
        .map_err(|message| error("PAYMENTS_DISABLED", &message))?;
    if !input.terms_accepted
        || config.purchase_terms_version.as_deref() != Some(&input.terms_version)
    {
        return Err(error(
            "PAYMENT_TERMS_REQUIRED",
            "Accept the current purchase terms",
        ));
    }
    // A waiver is a separate legal decision. Until its own versioned contract is enabled,
    // purchases preserve the buyer's withdrawal path.
    if input.withdrawal_waiver {
        return Err(error(
            "PAYMENT_TERMS_REQUIRED",
            "This checkout does not offer a withdrawal waiver",
        ));
    }
    let buyer_email = if config.livemode {
        if state.mail_client.is_none() {
            return Err(error(
                "PAYMENTS_DISABLED",
                "Purchase confirmation email is not configured",
            ));
        }
        let identity = user.user_info(&state).await?;
        if identity.sub != buyer || identity.email_verified != Some(true) {
            return Err(error(
                "PAYMENT_EMAIL_REQUIRED",
                "Verify your email before purchasing",
            ));
        }
        Some(
            identity
                .email
                .filter(|email| !email.trim().is_empty())
                .ok_or_else(|| {
                    error(
                        "PAYMENT_EMAIL_REQUIRED",
                        "A verified email address is required",
                    )
                })?,
        )
    } else {
        crate::entity::user::Entity::find_by_id(&buyer)
            .one(&state.db)
            .await?
            .and_then(|user| user.email)
    };
    let locale = input.locale.clone().unwrap_or_else(|| "en".into());
    let terms = legal_text(&state, "PURCHASE_TERMS", &input.terms_version, &locale)?;
    let product = listing(&state, kind, &item_id, &locale).await?;
    let payee = product.payee.clone();
    if buyer == payee {
        return Err(error(
            "SELF_PURCHASE",
            &format!("You cannot purchase your own {}", kind.noun()),
        ));
    }
    let recipient = accounts::require_can_sell(&state, &payee).await?;
    let platform_owned = matches!(recipient, accounts::PaymentRecipient::Platform);
    let account = match recipient {
        accounts::PaymentRecipient::Platform => None,
        accounts::PaymentRecipient::Connected(account) => Some(account),
    };
    if let Some(account) = &account {
        super::node::require_owner_terms(&state.db, config, &account.id, &payee).await?;
    }
    domain::validate_amount(
        product.price,
        "eur",
        config.marketplace_min_amount,
        config.max_payment_amount,
    )?;
    if !platform_owned {
        let seller_terms = config
            .seller_terms_version
            .as_deref()
            .ok_or_else(|| error("PAYMENT_TERMS_REQUIRED", "Seller terms are not configured"))?;
        let seller_consent = legal_consent::Entity::find()
            .filter(legal_consent::Column::UserId.eq(&payee))
            .filter(legal_consent::Column::Kind.eq("SELLER_TERMS"))
            .filter(legal_consent::Column::TextVersion.eq(seller_terms))
            .filter(legal_consent::Column::Accepted.eq(true))
            .one(&state.db)
            .await?
            .ok_or_else(|| {
                error(
                    "LISTING_UNAVAILABLE",
                    "The seller must accept the current selling terms",
                )
            })?;
        let current_seller_terms =
            legal_text(&state, "SELLER_TERMS", seller_terms, &seller_consent.locale)?;
        if seller_consent.text_hash
            != blake3::hash(current_seller_terms.as_bytes())
                .to_hex()
                .as_str()
        {
            return Err(error(
                "LISTING_UNAVAILABLE",
                "The seller must accept the current selling terms",
            ));
        }
    }
    let scope = connect_scope(&state).await?;
    let offer = Offer {
        kind,
        item_id: item_id.clone(),
        buyer: buyer.clone(),
        buyer_email,
        payee,
        platform_owned,
        account_id: account
            .as_ref()
            .map(|account| account.stripe_account_id.clone().ok_or(ApiError::NOT_FOUND))
            .transpose()?,
        account_row_id: account.as_ref().map(|account| account.id.clone()),
        account_revision: account.as_ref().map_or(0, |account| account.revision),
        amount: product.price,
        fee: if platform_owned {
            0
        } else {
            domain::fee(product.price, config.marketplace_fee_bps)?
        },
        fee_bps: if platform_owned {
            0
        } else {
            config.marketplace_fee_bps
        },
        fee_basis: config.marketplace_fee_basis.clone(),
        tax_mode: config.marketplace_tax_mode.clone(),
        title: product.title,
        role_id: product.role_id,
        terms_version: input.terms_version,
        terms_hash: blake3::hash(terms.as_bytes()).to_hex().to_string(),
        terms_text: terms,
        locale,
        withdrawal_waiver: false,
        withdrawal_days: Some(config.withdrawal_days),
        withdrawal_deadline: 0,
        method_policy: PaymentMethodPolicy {
            version: "marketplace-v1".into(),
            charge_model: if platform_owned {
                ChargeModel::Platform
            } else {
                ChargeModel::Destination
            },
            allowed_methods: config.marketplace_payment_methods.clone(),
            configuration_id: config.marketplace_method_configuration.clone(),
            allow_delayed: false,
        },
        product_tax_code: config.product_tax_code.clone(),
    };
    offer.method_policy.validate().map_err(stripe_error)?;
    if offer.tax_mode == PaymentTaxMode::PlatformSupplier {
        crate::payments::tax::require_ready(
            &state,
            &scope,
            offer.product_tax_code.as_deref().ok_or_else(|| {
                error(
                    "PAYMENT_TAX_REVIEW",
                    "The product tax classification is missing",
                )
            })?,
        )
        .await?;
    }
    // App keys predate packages and keep their shape; package keys name the kind
    // so an app and a package with the same id never share an open order.
    let open_key_parts = match kind {
        ItemKind::App => json!([scope.platform_account_id, scope.livemode, buyer, item_id]),
        ItemKind::Package => json!([
            scope.platform_account_id,
            scope.livemode,
            buyer,
            kind.as_str(),
            item_id
        ]),
    };
    let open_key = blake3::hash(serde_json::to_vec(&open_key_parts)?.as_slice())
        .to_hex()
        .to_string();
    if let Some(existing) = payment_order::Entity::find()
        .filter(payment_order::Column::OpenKey.eq(&open_key))
        .one(&state.db)
        .await?
    {
        let previous: Offer = serde_json::from_value(existing.snapshot.clone())?;
        if !same_offer(&previous, &offer) {
            cancel_order(&state, &existing.id).await?;
            return Err(error(
                "PAYMENT_NOT_PAYABLE",
                "The previous offer is being closed. Check the order before starting again",
            ));
        }
        reconcile_for_view(&state, &existing.id).await?;
        return Ok(Json(order_view(&state, &existing.id).await?));
    }
    let id = flow_like_types::create_id();
    let attempt_id = flow_like_types::create_id();
    let consent_id = flow_like_types::create_id();
    let created = now();
    let expires = created + 3_600_000;
    let mut success = flow_like_types::reqwest::Url::parse(
        config
            .frontend_url
            .as_deref()
            .ok_or_else(|| error("PAYMENTS_DISABLED", "Payment return URL is not configured"))?,
    )
    .map_err(|_| ApiError::internal("Invalid payment return URL"))?;
    success.set_path(match kind {
        ItemKind::App => "/store",
        ItemKind::Package => "/store/packages",
    });
    success.set_query(None);
    success.set_fragment(None);
    success
        .query_pairs_mut()
        .append_pair("id", &item_id)
        .append_pair("order", &id);
    let success_url = success.to_string();
    success.query_pairs_mut().append_pair("canceled", "1");
    let parameters = checkout_parameters(&offer, &id, expires, &success_url, &success.to_string())?;
    let request = StripeRequest::post(
        scope.clone(),
        "/v1/checkout/sessions",
        parameters,
        format!("mkt:{id}:session:1"),
    );
    let daily_cap = config.new_seller_daily_cap;
    let admission_config = config.clone();
    let saved_id = state.transaction(|txn| { let (offer, scope, id, attempt_id, consent_id, open_key, request)=(offer.clone(),scope.clone(),id.clone(),attempt_id.clone(),consent_id.clone(),open_key.clone(),request.clone()); let admission_config=admission_config.clone(); Box::pin(async move {
        crate::db::coordination::coordinate(txn,"payments-owner",&[&offer.payee]).await?;
        crate::db::coordination::coordinate(txn,offer.kind.lock(),&[&offer.item_id]).await?;
        coordinate_entitlement(txn,&offer.buyer,offer.kind,&offer.item_id).await?;
        if let Some(existing)=payment_order::Entity::find().filter(payment_order::Column::OpenKey.eq(&open_key)).one(txn).await? { return Ok::<_,ApiError>(existing.id); }
        validate_admission(txn,&offer).await?;
        if let Some(account_row_id) = &offer.account_row_id {
            super::node::require_owner_terms(txn,&admission_config,account_row_id,&offer.payee).await?;
        }
        reserve_limit(txn,&scope,&offer.payee,&id,offer.amount,daily_cap,expires).await?;
        let operation_id=operations::prepare(txn,SOURCE,&id,"checkout_create",&request).await?;
        txn.execute_raw(sql(r#"INSERT INTO "LegalConsent" (id,"userId",kind,"subjectType","subjectId","textVersion","textHash",locale,accepted,evidence,"createdAt") VALUES ($1,$2,'PURCHASE_TERMS','PAYMENT_ORDER',$3,$4,$5,$6,true,$7,$8)"#,vec![consent_id.clone().into(),offer.buyer.clone().into(),id.clone().into(),offer.terms_version.clone().into(),offer.terms_hash.clone().into(),offer.locale.clone().into(),json!({"text":offer.terms_text,"withdrawalWaiver":false}).into(),created.into()])).await?;
        txn.execute_raw(sql(r#"INSERT INTO "PaymentOrder" (id,kind,"userId","itemId","payeeUserId","connectedAccountId","platformAccountId",livemode,"openKey",status,"chargeType",amount,currency,"applicationFeeAmount","feeBps",snapshot,"consentId","expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,$17,$2,$3,$4,$5,$6,$7,$8,'OPENING',$16,$9,'eur',$10,$11,$12,$13,$14,$15,$15,$15)"#,vec![id.clone().into(),offer.buyer.clone().into(),offer.item_id.clone().into(),offer.payee.clone().into(),offer.account_id.clone().into(),scope.platform_account_id.clone().into(),scope.livemode.into(),open_key.into(),offer.amount.into(),offer.fee.into(),i32::from(offer.fee_bps).into(),serde_json::to_value(&offer)?.into(),consent_id.into(),expires.into(),created.into(),if offer.platform_owned { "PLATFORM" } else { "DESTINATION" }.into(),offer.kind.as_str().into()])).await?;
        let (app_column, package_column) = match offer.kind {
            ItemKind::App => (Some(offer.item_id.clone()), None),
            ItemKind::Package => (None, Some(offer.item_id.clone())),
        };
        txn.execute_raw(sql(r#"INSERT INTO "PaymentAttempt" (id,"sourceType","sourceId",attempt,"platformAccountId","scopeKey",livemode,"operationId","payerUserId","payeeUserId","appId","packageId",amount,currency,"applicationFeeAmount","feeBps",snapshot,"expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,'MARKETPLACE',$2,1,$3,'platform',$4,$5,$6,$7,$8,$15,$9,'eur',$10,$11,$12,$13,$14,$14,$14)"#,vec![attempt_id.into(),id.clone().into(),scope.platform_account_id.into(),scope.livemode.into(),operation_id.into(),offer.buyer.clone().into(),offer.payee.clone().into(),app_column.into(),offer.amount.into(),offer.fee.into(),i32::from(offer.fee_bps).into(),serde_json::to_value(&offer)?.into(),expires.into(),created.into(),package_column.into()])).await?;
        outbox::enqueue(txn,&format!("mkt:{id}:open"),"marketplace_reconcile",SOURCE,&id,json!({})).await?;
        Ok(id)
    }) }).await?;
    let saved = load_order(&state.db, &saved_id).await?;
    let previous: Offer = serde_json::from_value(saved.snapshot)?;
    if !same_offer(&previous, &offer) {
        cancel_order(&state, &saved_id).await?;
        return Err(error(
            "PAYMENT_NOT_PAYABLE",
            "The previous offer is being closed. Check the order before starting again",
        ));
    }
    reconcile_for_view(&state, &saved_id).await?;
    Ok(Json(order_view(&state, &saved_id).await?))
}

fn checkout_parameters(
    offer: &Offer,
    id: &str,
    expires: i64,
    success_url: &str,
    cancel_url: &str,
) -> Result<Value, ApiError> {
    let mut parameters = json!({"mode":"payment","ui_mode":"hosted_page","line_items":[{"quantity":1,"price_data":{"currency":"eur","unit_amount":offer.amount,"product_data":{"name":offer.title}}}],"payment_intent_data":{"metadata":{"flowlike_kind":"marketplace","flowlike_id":id}},"metadata":{"flowlike_kind":"marketplace","flowlike_id":id},"client_reference_id":format!("mkt:{id}"),"expires_at":expires/1000,"success_url":success_url,"cancel_url":cancel_url,"billing_address_collection":"required","locale":offer.locale});
    if offer.platform_owned {
        if offer.account_id.is_some()
            || offer.account_row_id.is_some()
            || offer.fee != 0
            || offer.fee_bps != 0
        {
            return Err(ApiError::internal("Invalid platform payment recipient"));
        }
    } else {
        let destination = offer
            .account_id
            .as_deref()
            .ok_or_else(|| ApiError::internal("Missing seller payment account"))?;
        parameters["payment_intent_data"]["application_fee_amount"] = json!(offer.fee);
        parameters["payment_intent_data"]["transfer_data"] = json!({"destination":destination});
        parameters["payment_intent_data"]["transfer_group"] = json!(format!("mkt:{id}"));
    }
    if let Some(email) = &offer.buyer_email {
        parameters["customer_email"] = json!(email);
    }
    for (key, value) in offer
        .method_policy
        .checkout_parameters()
        .map_err(stripe_error)?
        .as_object()
        .ok_or_else(|| ApiError::internal("Invalid method policy"))?
    {
        parameters[key] = value.clone();
    }
    if offer.tax_mode == PaymentTaxMode::PlatformSupplier {
        let tax_code = offer
            .product_tax_code
            .as_deref()
            .filter(|code| !code.is_empty())
            .ok_or_else(|| {
                error(
                    "PAYMENT_TAX_REVIEW",
                    "The product tax classification is missing",
                )
            })?;
        parameters["line_items"][0]["price_data"]["tax_behavior"] = json!("inclusive");
        parameters["line_items"][0]["price_data"]["product_data"]["tax_code"] = json!(tax_code);
        parameters["automatic_tax"] = json!({"enabled":true,"liability":{"type":"self"}});
        parameters["customer_creation"] = json!("always");
        parameters["invoice_creation"] =
            json!({"enabled":true,"invoice_data":{"issuer":{"type":"self"}}});
    }
    Ok(parameters)
}

async fn reconcile_for_view(state: &AppState, id: &str) -> Result<(), ApiError> {
    if let Err(err) = reconcile_order(state, id).await {
        if matches!(
            err.public_code(),
            "PAYMENT_OPERATION_PENDING" | "PAYMENT_PROVIDER_ERROR" | "PAYMENT_OPERATION_REVIEW"
        ) {
            tracing::warn!(order_id=id,error=%err,"Persisted checkout awaits reconciliation");
        } else {
            return Err(err);
        }
    }
    Ok(())
}

fn same_offer(one: &Offer, two: &Offer) -> bool {
    one.kind == two.kind
        && one.item_id == two.item_id
        && one.buyer == two.buyer
        && one.payee == two.payee
        && one.platform_owned == two.platform_owned
        && one.account_id == two.account_id
        && one.amount == two.amount
        && one.fee == two.fee
        && one.fee_basis == two.fee_basis
        && one.tax_mode == two.tax_mode
        && one.product_tax_code == two.product_tax_code
        && one.terms_version == two.terms_version
        && one.terms_hash == two.terms_hash
        && one.method_policy == two.method_policy
        && one.title == two.title
        && one.locale == two.locale
        && one.role_id == two.role_id
}

pub(crate) async fn coordinate_entitlement(
    txn: &sea_orm::DatabaseTransaction,
    user: &str,
    kind: ItemKind,
    item: &str,
) -> Result<(), ApiError> {
    let kind = kind.as_str();
    crate::db::coordination::coordinate(txn, "payments-entitlement", &[user, kind, item]).await?;
    let id = blake3::hash(serde_json::to_vec(&json!([user, kind, item]))?.as_slice())
        .to_hex()
        .to_string();
    txn.execute_raw(sql(r#"INSERT INTO "PaymentEntitlement" (id,"userId","itemKind","itemId","createdAt","updatedAt") VALUES ($1,$2,$5,$3,$4,$4) ON CONFLICT DO NOTHING"#,vec![id.clone().into(),user.into(),item.into(),now().into(),kind.into()])).await?;
    txn.execute_raw(sql(
        r#"UPDATE "PaymentEntitlement" SET revision=revision+1,"updatedAt"=$2 WHERE id=$1"#,
        vec![id.into(), now().into()],
    ))
    .await?;
    Ok(())
}

pub(crate) async fn block_entitlement(
    txn: &sea_orm::DatabaseTransaction,
    user: &str,
    app: &str,
) -> Result<(), ApiError> {
    coordinate_entitlement(txn, user, ItemKind::App, app).await?;
    txn.execute_raw(sql(r#"UPDATE "PaymentEntitlement" SET blocked=true,revision=revision+1,"updatedAt"=$3 WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2"#,vec![user.into(),app.into(),now().into()])).await?;
    txn.execute_raw(sql(r#"UPDATE "AccessGrant" SET status='REVOKED',reason='membership_removed',revision=revision+1,"updatedAt"=$3 WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND status='ACTIVE'"#,vec![user.into(),app.into(),now().into()])).await?;
    let orders=txn.query_all_raw(sql(r#"UPDATE "PaymentOrder" SET "cancelRequested"=true,status='CANCEL_PENDING',revision=revision+1,"updatedAt"=$3 WHERE "userId"=$1 AND kind='APP' AND "itemId"=$2 AND "acceptedAttemptId" IS NULL AND "openKey" IS NOT NULL RETURNING id"#,vec![user.into(),app.into(),now().into()])).await?;
    for order in orders {
        let id: String = order.try_get("", "id")?;
        outbox::enqueue(
            txn,
            &format!("mkt:{id}:access-removed"),
            "marketplace_reconcile",
            SOURCE,
            &id,
            json!({}),
        )
        .await?;
    }
    Ok(())
}

async fn reserve_limit<C: ConnectionTrait>(
    txn: &C,
    scope: &StripeScope,
    seller: &str,
    id: &str,
    amount: i64,
    cap: i64,
    expires: i64,
) -> Result<(), ApiError> {
    let day = now() / 86_400_000;
    let key = format!(
        "seller:{}:{}:{seller}:{day}:eur",
        scope.platform_account_id, scope.livemode
    );
    txn.execute_raw(sql(r#"INSERT INTO "PaymentLimitCounter" (key,currency,"windowEnd","createdAt","updatedAt") VALUES ($1,'eur',$2,$3,$3) ON CONFLICT DO NOTHING"#,vec![key.clone().into(),((day+1)*86_400_000).into(),now().into()])).await?;
    let changed=txn.execute_raw(sql(r#"UPDATE "PaymentLimitCounter" SET amount=amount+$2,count=count+1,revision=revision+1,"updatedAt"=$3 WHERE key=$1 AND amount<=$4-$2 AND count<1000"#,vec![key.clone().into(),amount.into(),now().into(),cap.into()])).await?;
    if changed.rows_affected() != 1 {
        return Err(error(
            "PAYMENT_LIMIT_REACHED",
            "The seller daily payment limit has been reached",
        ));
    }
    txn.execute_raw(sql(r#"INSERT INTO "PaymentLimitReservation" (id,"counterKey","sourceType","sourceId",amount,"expiresAt","createdAt","updatedAt") VALUES ($1,$2,'MARKETPLACE',$1,$3,$4,$5,$5)"#,vec![id.into(),key.into(),amount.into(),expires.into(),now().into()])).await?;
    Ok(())
}

/// The product still matches the offer. Returns whether an app listing needs an
/// approved join request before purchase.
async fn validate_listing(
    txn: &sea_orm::DatabaseTransaction,
    offer: &Offer,
) -> Result<bool, ApiError> {
    let changed = || {
        error(
            "LISTING_UNAVAILABLE",
            &format!(
                "This offer changed. Reload the {} before buying",
                offer.kind.noun()
            ),
        )
    };
    match offer.kind {
        ItemKind::App => {
            use crate::entity::sea_orm_active_enums::Visibility;
            let product = app::Entity::find_by_id(&offer.item_id)
                .one(txn)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if product.price != offer.amount
                || !matches!(
                    product.visibility,
                    Visibility::Public | Visibility::PublicRequestAccess
                )
                || payee_for_app(txn, &offer.item_id).await? != offer.payee
            {
                return Err(changed());
            }
            Ok(product.visibility == Visibility::PublicRequestAccess)
        }
        ItemKind::Package => {
            let product = wasm_package::Entity::find_by_id(&offer.item_id)
                .one(txn)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if product.price != offer.amount
                || product.visibility != WasmPackageVisibility::Public
                || product.status != WasmPackageStatus::Active
                || payee_for_package(txn, &offer.item_id).await? != offer.payee
            {
                return Err(changed());
            }
            Ok(false)
        }
    }
}

async fn validate_admission(
    txn: &sea_orm::DatabaseTransaction,
    offer: &Offer,
) -> Result<(), ApiError> {
    let needs_approval = validate_listing(txn, offer).await?;
    let legacy_kind = match offer.kind {
        ItemKind::App => "app_purchase",
        ItemKind::Package => "wasm_purchase",
    };
    if txn.query_one_raw(sql(r#"SELECT id FROM "LegacyCheckout" WHERE kind=$3 AND "userId"=$1 AND "itemId"=$2 AND "openKey" IS NOT NULL"#,vec![offer.buyer.clone().into(),offer.item_id.clone().into(),legacy_kind.into()])).await?.is_some(){return Err(error("PAYMENT_OPERATION_PENDING","Your previous checkout must finish before a new order can open"));}
    if accounts::is_platform_admin(txn, &offer.payee).await? != offer.platform_owned {
        return Err(error(
            "LISTING_UNAVAILABLE",
            &format!(
                "The payment recipient changed. Reload the {} before buying",
                offer.kind.noun()
            ),
        ));
    }
    if !offer.platform_owned {
        let account = connected_account::Entity::find_by_id(
            offer
                .account_row_id
                .as_deref()
                .ok_or_else(|| ApiError::internal("Missing seller payment account"))?,
        )
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
        if account.revision != offer.account_revision
            || account.retired_at.is_some()
            || !account.can_sell
        {
            return Err(error(
                "LISTING_UNAVAILABLE",
                "The seller payment account changed",
            ));
        }
        let fenced=txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET revision=revision+1 WHERE id=$1 AND revision=$2 AND "retiredAt" IS NULL"#,vec![account.id.into(),account.revision.into()])).await?;
        if fenced.rows_affected() != 1 {
            return Err(error(
                "LISTING_UNAVAILABLE",
                "The seller payment account changed",
            ));
        }
    }
    match offer.kind {
        ItemKind::App => {
            if txn.query_one_raw(sql(r#"SELECT "userId" FROM "PaymentsBlock" WHERE "userId"=$1 UNION ALL SELECT "ownerUserId" FROM "AppPaymentSettings" WHERE "appId"=$2 AND "adminBlockedAt" IS NOT NULL"#,vec![offer.payee.clone().into(),offer.item_id.clone().into()])).await?.is_some() {return Err(error("LISTING_UNAVAILABLE","Payments are paused"));}
            if txn.query_one_raw(sql(r#"SELECT id FROM "Membership" WHERE "userId"=$1 AND "appId"=$2 UNION ALL SELECT id FROM "AccessGrant" WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND status='ACTIVE' UNION ALL SELECT id FROM "PaymentEntitlement" WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND blocked=true"#,vec![offer.buyer.clone().into(),offer.item_id.clone().into()])).await?.is_some() {return Err(error("ALREADY_OWNED","The app is already available or access is restricted"));}
            if needs_approval && txn.query_one_raw(sql(r#"SELECT id FROM "JoinQueue" WHERE "userId"=$1 AND "appId"=$2 AND "approvedAt" IS NOT NULL"#,vec![offer.buyer.clone().into(),offer.item_id.clone().into()])).await?.is_none() {return Err(error("APPROVAL_REQUIRED","The owner must approve your request before purchase"));}
        }
        ItemKind::Package => {
            if txn
                .query_one_raw(sql(
                    r#"SELECT "userId" FROM "PaymentsBlock" WHERE "userId"=$1"#,
                    vec![offer.payee.clone().into()],
                ))
                .await?
                .is_some()
            {
                return Err(error("LISTING_UNAVAILABLE", "Payments are paused"));
            }
            if txn.query_one_raw(sql(r#"SELECT id FROM "WasmPackageUser" WHERE "userId"=$1 AND "packageId"=$2 UNION ALL SELECT id FROM "AccessGrant" WHERE "userId"=$1 AND "itemKind"='PACKAGE' AND "itemId"=$2 AND status='ACTIVE' UNION ALL SELECT id FROM "PaymentEntitlement" WHERE "userId"=$1 AND "itemKind"='PACKAGE' AND "itemId"=$2 AND blocked=true"#,vec![offer.buyer.clone().into(),offer.item_id.clone().into()])).await?.is_some() {return Err(error("ALREADY_OWNED","You already have this package or access to it is restricted"));}
        }
    }
    Ok(())
}

pub(super) async fn order_view(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let order = load_order(&state.db, id).await?;
    let attempt = payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::SourceId.eq(id))
        .order_by_desc(payment_attempt::Column::Attempt)
        .one(&state.db)
        .await?;
    let offer: Offer = serde_json::from_value(order.snapshot.clone())?;
    Ok(
        json!({"id":order.id,"orderId":order.id,"platformOwned":offer.platform_owned,"appId":order.item_id,"itemId":order.item_id,"itemKind":ItemKind::of(&order).as_str(),"itemName":offer.title,"amountMinor":order.amount,"receiptUrl":attempt.as_ref().and_then(|a|a.snapshot.get("receiptUrl")).and_then(Value::as_str),"refundableRemaining":attempt.as_ref().map(|a|a.captured_amount-a.refunded_amount-a.reserved_refund_amount).unwrap_or(0),"status":order.status,"amount":order.amount,"currency":order.currency,"checkoutUrl":attempt.as_ref().filter(|a|a.status=="OPEN" && !order.cancel_requested).and_then(|a|a.checkout_url.clone()),"refundedAmount":attempt.as_ref().map(|a|a.refunded_amount).unwrap_or(0),"pendingRefundAmount":attempt.as_ref().map(|a|a.reserved_refund_amount).unwrap_or(0),"refundReviewRequired":attempt.as_ref().and_then(|a|a.snapshot.get("refundReviewRequired")).and_then(Value::as_bool).unwrap_or(false),"withdrawable":order.accepted_attempt_id.is_some() && order.withdrawn_at.is_none() && !offer.withdrawal_waiver && now()<=offer.withdrawal_deadline,"withdrawDeadline":offer.withdrawal_deadline}),
    )
}

#[utoipa::path(get,path="/user/purchases/{id}",tag="payments",responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn get_purchase(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    let order = load_order(&state.db, &id).await?;
    if order.user_id != sub {
        return Err(ApiError::NOT_FOUND);
    }
    Ok(Json(order_view(&state, &id).await?))
}

#[utoipa::path(get,path="/user/purchases",tag="payments",responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn list_purchases(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<PurchaseQuery>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    let mut orders = payment_order::Entity::find().filter(payment_order::Column::UserId.eq(&sub));
    if let Some(before) = query.before {
        orders = orders.filter(payment_order::Column::CreatedAt.lt(before));
    }
    let rows = orders
        .order_by_desc(payment_order::Column::CreatedAt)
        .limit(50)
        .all(&state.db)
        .await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        views.push(order_view(&state, &row.id).await?);
    }
    let legacy = super::legacy_checkout::purchase_history(&state, &sub, None).await?;
    Ok(Json(json!({"orders":views,"legacyPurchases":legacy})))
}

#[utoipa::path(get,path="/user/sales",tag="payments",responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn list_sales(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<PurchaseQuery>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    let platform_admin = accounts::is_platform_admin(&state.db, &sub).await?;
    let mut recipients = sea_orm::Condition::any().add(
        sea_orm::Condition::all()
            .add(payment_order::Column::PayeeUserId.eq(&sub))
            .add(payment_order::Column::ChargeType.ne("PLATFORM")),
    );
    if platform_admin {
        recipients = recipients.add(payment_order::Column::ChargeType.eq("PLATFORM"));
    }
    let mut orders = payment_order::Entity::find()
        .filter(recipients)
        .filter(payment_order::Column::Livemode.eq(state.platform_config.payments.livemode));
    if let Some(before) = query.before {
        orders = orders.filter(payment_order::Column::CreatedAt.lt(before));
    }
    let rows = orders
        .order_by_desc(payment_order::Column::CreatedAt)
        .limit(50)
        .all(&state.db)
        .await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        views.push(order_view(&state, &row.id).await?);
    }
    Ok(Json(json!({"orders":views})))
}

pub(super) async fn cancel_order(state: &AppState, id: &str) -> Result<(), ApiError> {
    let id = id.to_owned();
    state.transaction(|txn|{let id=id.clone();Box::pin(async move{
        txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET "cancelRequested"=true,status='CANCEL_PENDING',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "acceptedAttemptId" IS NULL AND status NOT IN ('CANCELED','EXPIRED','FAILED')"#,vec![id.clone().into(),now().into()])).await?;
        outbox::enqueue(txn,&format!("mkt:{id}:cancel"),"marketplace_reconcile",SOURCE,&id,json!({})).await
    })}).await?;
    reconcile_order(state, &id).await
}

#[utoipa::path(post,path="/user/purchases/{id}/cancel",tag="payments",responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn cancel(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let order = load_order(&state.db, &id).await?;
    if order.user_id != payer(&user)? {
        return Err(ApiError::NOT_FOUND);
    }
    cancel_order(&state, &id).await?;
    Ok(Json(order_view(&state, &id).await?))
}

#[utoipa::path(post,path="/user/sales/{id}/refund",tag="payments",request_body=RefundInput,responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn refund_sale(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(body): Json<RefundInput>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    let order = load_order(&state.db, &id).await?;
    let offer: Offer = serde_json::from_value(order.snapshot.clone())?;
    let permitted = if offer.platform_owned {
        accounts::is_platform_admin(&state.db, &sub).await?
    } else {
        order.payee_user_id == sub
    };
    if !permitted {
        return Err(ApiError::NOT_FOUND);
    }
    let attempt = order
        .accepted_attempt_id
        .ok_or_else(|| error("PAYMENT_NOT_PAID", "This order has no accepted payment"))?;
    let refund = settlement::request_seller_refund(
        &state,
        &attempt,
        &body.command_id,
        body.amount,
        body.reason.as_deref().unwrap_or("requested_by_customer"),
        &sub,
    )
    .await?;
    Ok(Json(json!({"refundId":refund,"status":"PENDING"})))
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawalInput {
    pub confirm: bool,
    pub consumer_name: String,
    pub confirmation_email: String,
}

impl WithdrawalInput {
    fn validated(mut self) -> Result<Self, ApiError> {
        if !self.confirm {
            return Err(ApiError::bad_request("Confirm withdrawal"));
        }
        if self.consumer_name.chars().any(char::is_control) {
            return Err(ApiError::bad_request(
                "Enter your name without control characters",
            ));
        }
        self.consumer_name = self.consumer_name.trim().to_owned();
        if self.consumer_name.is_empty() || self.consumer_name.chars().count() > 200 {
            return Err(ApiError::bad_request(
                "Enter your name using 1 to 200 characters",
            ));
        }
        // Reject header controls before trimming; they must never reach a mail provider.
        super::mail_confirmations::validate_confirmation_email(&self.confirmation_email)?;
        self.confirmation_email = self.confirmation_email.trim().to_owned();
        Ok(self)
    }

    fn evidence(&self, order_id: &str, locale: &str, received_at: i64) -> Value {
        let declaration = if locale.split(['-', '_']).next() == Some("de") {
            format!("Ich widerrufe den Kauf mit der Auftragsnummer {order_id}.")
        } else {
            format!("I withdraw from the purchase identified by order {order_id}.")
        };
        json!({"consumer_name":self.consumer_name,"confirmation_email":self.confirmation_email,"declaration":declaration,"received_at":received_at})
    }
}

#[utoipa::path(post,path="/user/purchases/{id}/withdraw",tag="payments",request_body=WithdrawalInput,responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn withdraw(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(body): Json<WithdrawalInput>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    let body = body.validated()?;
    let received_at = now();
    let order = load_order(&state.db, &id).await?;
    if order.user_id != sub {
        return Err(ApiError::NOT_FOUND);
    }
    state.transaction(|txn| {
        let (order, body) = (order.clone(), body.clone());
        Box::pin(async move {
            coordinate_entitlement(txn, &order.user_id, ItemKind::of(&order), &order.item_id)
                .await?;
            let current = load_order(txn, &order.id).await?;
            // A retry must keep the original declaration, recipient and receipt time.
            if current.withdrawn_at.is_some() {
                return Ok::<_, ApiError>(());
            }
            let offer: Offer = serde_json::from_value(current.snapshot.clone())?;
            if offer.withdrawal_waiver || received_at > offer.withdrawal_deadline {
                return Err(error("WITHDRAWAL_UNAVAILABLE", "This order is outside its withdrawal period"));
            }
            if current.accepted_attempt_id.is_none() {
                return Err(error("PAYMENT_NOT_PAID", "This order has no accepted payment"));
            }
            let mut snapshot = current.snapshot.clone();
            snapshot["withdrawal_confirmation"] = body.evidence(&current.id, &offer.locale, received_at);
            txn.execute_raw(sql(
                r#"UPDATE "PaymentOrder" SET "withdrawnAt"=$2,snapshot=$3,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "withdrawnAt" IS NULL"#,
                vec![current.id.clone().into(), received_at.into(), snapshot.into()],
            )).await?;
            settlement::revoke_grant(txn, &current, "withdrawal").await?;
            outbox::enqueue(txn, &format!("mkt:{}:email:withdrawn", current.id), "PAYMENT_EMAIL", SOURCE, &current.id, json!({"kind":"withdrawn"})).await?;
            outbox::enqueue(txn, &format!("mkt:{}:withdrawal", current.id), "marketplace_withdrawal", SOURCE, &current.id, json!({})).await
        })
    }).await?;
    Ok(Json(order_view(&state, &id).await?))
}

#[cfg(test)]
mod withdrawal_input_tests {
    use super::*;

    #[test]
    fn declaration_requires_confirmation_and_a_safe_name_and_email() {
        let input = || WithdrawalInput {
            confirm: true,
            consumer_name: "  Jörg Buyer  ".into(),
            confirmation_email: "buyer@example.com".into(),
        };
        let valid = input().validated().unwrap();
        assert_eq!(valid.consumer_name, "Jörg Buyer");
        let evidence = valid.evidence("order-1", "de-DE", 123);
        assert_eq!(
            evidence["declaration"],
            "Ich widerrufe den Kauf mit der Auftragsnummer order-1."
        );
        assert_eq!(evidence["received_at"], 123);
        let mut unconfirmed = input();
        unconfirmed.confirm = false;
        assert!(unconfirmed.validated().is_err());
        for name in ["", " \t ", "Buyer\nOther", &"x".repeat(201)] {
            let mut invalid = input();
            invalid.consumer_name = name.into();
            assert!(invalid.validated().is_err());
        }
        for email in [
            "",
            "a@example.com\r\nBcc: other@example.com",
            "a@b",
            "a@b@c.com",
        ] {
            let mut invalid = input();
            invalid.confirmation_email = email.into();
            assert!(invalid.validated().is_err());
        }
    }
}

#[utoipa::path(post,path="/apps/{app_id}/marketplace/terms",tag="payments",request_body=CheckoutInput,responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn accept_seller_terms(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(input): Json<CheckoutInput>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    ensure_app_owner(&state.db, &app_id, &sub).await?;
    record_seller_terms(&state, &sub, "APP", &app_id, input).await
}

#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/marketplace/terms",
    tag = "payments",
    description = "Accept the seller terms so the package can be sold. Only the package owner can accept them.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = CheckoutInput,
    responses(
        (status = 200, description = "Seller terms accepted"),
        (status = 403, description = "Only the package owner can accept the seller terms")
    ),
    security(("bearer_auth" = []))
)]
pub async fn accept_package_seller_terms(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(input): Json<CheckoutInput>,
) -> Result<Json<Value>, ApiError> {
    let sub = payer(&user)?;
    if payee_for_package(&state.db, &package_id).await? != sub {
        return Err(ApiError::forbidden(
            "Only the package owner can accept the seller terms",
        ));
    }
    record_seller_terms(&state, &sub, "WASM_PACKAGE", &package_id, input).await
}

async fn record_seller_terms(
    state: &AppState,
    sub: &str,
    subject_type: &str,
    subject_id: &str,
    input: CheckoutInput,
) -> Result<Json<Value>, ApiError> {
    if !input.terms_accepted
        || state
            .platform_config
            .payments
            .seller_terms_version
            .as_deref()
            != Some(&input.terms_version)
    {
        return Err(error(
            "PAYMENT_TERMS_REQUIRED",
            "Accept the current seller terms",
        ));
    }
    let locale = input.locale.unwrap_or_else(|| "en".into());
    let text = legal_text(state, "SELLER_TERMS", &input.terms_version, &locale)?;
    let id = flow_like_types::create_id();
    state.db.execute_raw(sql(r#"INSERT INTO "LegalConsent" (id,"userId",kind,"subjectType","subjectId","textVersion","textHash",locale,accepted,evidence,"createdAt") VALUES ($1,$2,'SELLER_TERMS',$9,$3,$4,$5,$6,true,$7,$8)"#,vec![id.clone().into(),sub.into(),subject_id.into(),input.terms_version.into(),blake3::hash(text.as_bytes()).to_hex().to_string().into(),locale.into(),json!({"text":text}).into(),now().into(),subject_type.into()])).await?;
    Ok(Json(json!({"consentId":id})))
}

#[utoipa::path(post,path="/apps/{app_id}/marketplace/requests/{user_id}/approve",tag="payments",request_body=Confirmation,responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn approve(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, user_id)): Path<(String, String)>,
    Json(body): Json<Confirmation>,
) -> Result<Json<Value>, ApiError> {
    let owner = payer(&user)?;
    if !body.confirm {
        return Err(ApiError::bad_request("Confirm approval"));
    }
    state.transaction(|txn|{let (app_id,user_id,owner)=(app_id.clone(),user_id.clone(),owner.clone());Box::pin(async move{crate::db::coordination::coordinate(txn,"payments-app",&[&app_id]).await?;ensure_app_owner(txn,&app_id,&owner).await?;let changed=txn.execute_raw(sql(r#"UPDATE "JoinQueue" SET "approvedAt"=$3,"approvedBy"=$4 WHERE "appId"=$1 AND "userId"=$2"#,vec![app_id.into(),user_id.into(),now().into(),owner.into()])).await?;if changed.rows_affected()!=1{return Err(ApiError::NOT_FOUND);}Ok::<_,ApiError>(())})}).await?;
    Ok(Json(json!({"approved":true})))
}

#[utoipa::path(post,path="/apps/{app_id}/marketplace/comp/{user_id}",tag="payments",request_body=Confirmation,responses((status=200,description="Persisted payment result")),security(("bearer_auth"=[])))]
pub async fn comp(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, user_id)): Path<(String, String)>,
    Json(body): Json<Confirmation>,
) -> Result<Json<Value>, ApiError> {
    let owner = payer(&user)?;
    if !body.confirm {
        return Err(ApiError::bad_request("Confirm free access"));
    }
    let grant_id = flow_like_types::create_id();
    state.transaction(|txn|{let (app_id,user_id,owner,grant_id)=(app_id.clone(),user_id.clone(),owner.clone(),grant_id.clone());Box::pin(async move{crate::db::coordination::coordinate(txn,"payments-app",&[&app_id]).await?;ensure_app_owner(txn,&app_id,&owner).await?;coordinate_entitlement(txn,&user_id,ItemKind::App,&app_id).await?;if txn.query_one_raw(sql(r#"SELECT id FROM "PaymentEntitlement" WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND blocked=true"#,vec![user_id.clone().into(),app_id.clone().into()])).await?.is_some(){return Err(error("ACCESS_RESTRICTED","Access is blocked for this app"));}
        if crate::entity::user::Entity::find_by_id(&user_id).one(txn).await?.is_none(){return Err(ApiError::NOT_FOUND);}
        let product=app::Entity::find_by_id(&app_id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;let role=product.default_role_id.ok_or_else(||error("LISTING_UNAVAILABLE","The app has no buyer role"))?;
        txn.execute_raw(sql(r#"INSERT INTO "AccessGrant" (id,"userId","itemKind","itemId","sourceType","sourceId","grantedBy","createdAt","updatedAt") VALUES ($1,$2,'APP',$3,'COMP',$1,$4,$5,$5)"#,vec![grant_id.clone().into(),user_id.clone().into(),app_id.clone().into(),owner.into(),now().into()])).await?;
        txn.execute_raw(sql(r#"INSERT INTO "Membership" (id,"userId","appId","roleId","joinedVia","createdAt","updatedAt") VALUES ($1,$2,$3,$4,'payment_comp',$5,$5) ON CONFLICT ("userId","appId") DO NOTHING"#,vec![flow_like_types::create_id().into(),user_id.clone().into(),app_id.clone().into(),role.into(),Utc::now().fixed_offset().into()])).await?;
        txn.execute_raw(sql(r#"DELETE FROM "JoinQueue" WHERE "userId"=$1 AND "appId"=$2"#,vec![user_id.into(),app_id.into()])).await?;
        outbox::enqueue(txn,&format!("comp:{grant_id}"),"marketplace_comp",SOURCE,&grant_id,json!({})).await})}).await?;
    Ok(Json(json!({"grantId":grant_id})))
}
