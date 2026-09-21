use super::{
    accounts, connect_scope, domain, error, operations, outbox, payee_for_app, sql, stripe_error,
};
use crate::{
    entity::{app_payment_settings, connected_account, payment_attempt, payment_request},
    error::ApiError,
    execution::ExecutionClaims,
    middleware::jwt::{AppUser, viewer_authorization},
    state::AppState,
    stripe_connect::{ChargeModel, PaymentMethodPolicy, StripeRequest, StripeScope},
};
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
#[path = "node_tests.rs"]
mod integration_tests;
#[path = "node_settlement.rs"]
mod settlement;
pub use settlement::{handle_effect, handle_event, reconcile, reconcile_request};
pub const SOURCE: &str = "REQUEST";

#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePayment {
    pub node_id: String,
    pub nonce: String,
    pub amount_minor: i64,
    pub currency: String,
    pub product_name: String,
    pub product_tax_code: String,
    #[serde(default)]
    pub shipping_countries: Vec<String>,
    pub description: String,
    pub reference: Option<String>,
    pub ttl_seconds: i64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayerScope {
    pub app_id: String,
    pub run_id: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportInput {
    pub reason: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaymentView {
    pub id: String,
    pub app_id: String,
    pub run_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub expires_at: i64,
    pub amount_minor: i64,
    pub currency: String,
    pub product_name: String,
    pub description: String,
    pub payee_user_id: String,
    pub platform_owned: bool,
    pub payee_display_name: Option<String>,
    pub app_name: Option<String>,
    pub receipt_url: Option<String>,
    pub checkout_url: Option<String>,
    pub refunded_amount: i64,
    pub pending_refund_amount: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/execution/payments", post(create))
        .route("/execution/payments/{id}", get(executor_get))
        .route("/execution/payments/{id}/cancel", post(executor_cancel))
        .route("/payments/{id}", get(payer_get))
        .route("/payments/{id}/checkout", post(checkout))
        .route("/payments/{id}/decline", post(decline))
        .route("/payments/{id}/report", post(report))
}
pub(super) fn now() -> i64 {
    Utc::now().timestamp_millis()
}
fn claims(headers: &HeaderMap) -> Result<ExecutionClaims, ApiError> {
    let token = viewer_authorization(headers)
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ApiError::UNAUTHORIZED)?;
    let claims =
        crate::execution::verify_execution_jwt(token).map_err(|_| ApiError::UNAUTHORIZED)?;
    if claims.payer_sub.as_deref() != Some(claims.sub.as_str())
        || claims.technical_user_id.is_some()
        || claims.shadow.unwrap_or(false)
        || claims.app_chain.as_ref().is_some_and(|c| !c.is_empty())
        || claims.runtime_limit_ms.is_none()
    {
        return Err(error(
            "PAYMENT_REQUIRES_ATTENDED_RUN",
            "Payments require an attended remote run",
        ));
    }
    Ok(claims)
}
fn validate(input: &CreatePayment) -> Result<(), ApiError> {
    if !valid_tax_code(&input.product_tax_code) {
        return Err(error(
            "PAYMENT_TAX_CODE_REQUIRED",
            "Choose the Stripe tax code for the product or service being sold",
        ));
    }
    if !valid_shipping_countries(&input.shipping_countries) {
        return Err(ApiError::bad_request(
            "Shipping countries must be unique two-letter uppercase country codes",
        ));
    }
    if input.node_id.is_empty()
        || input.node_id.len() > 128
        || input.nonce.is_empty()
        || input.nonce.len() > 128
        || !input
            .nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || input.product_name.trim().is_empty()
        || input.product_name.chars().count() > 120
        || input.description.chars().count() > 1000
        || input
            .reference
            .as_ref()
            .is_some_and(|r| r.chars().count() > 200)
        || !(30..=3600).contains(&input.ttl_seconds)
    {
        return Err(ApiError::bad_request("Invalid payment request fields"));
    }
    Ok(())
}
fn valid_tax_code(code: &str) -> bool {
    flow_like::hub::valid_product_tax_code(code)
}
fn valid_shipping_countries(countries: &[String]) -> bool {
    countries.len() <= 100
        && countries
            .iter()
            .all(|country| country.len() == 2 && country.bytes().all(|c| c.is_ascii_uppercase()))
        && countries
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == countries.len()
}
pub(super) async fn run_record<C: ConnectionTrait>(
    db: &C,
    run_id: &str,
    app_id: &str,
    payer: &str,
) -> Result<Option<(i64, i64)>, ApiError> {
    let row=db.query_one_raw(sql(r#"SELECT q.deadline,q.generation FROM "QuotaOperation" q JOIN "ExecutionRun" r ON r.id=q.id WHERE q.id=$1 AND q.kind='workflow' AND q.status='running' AND q."cancelRequested"=false AND q.deadline>$4 AND r."appId"=$2 AND r."userId"=$3 AND r.status IN ('PENDING','RUNNING') AND r.mode!='LOCAL' AND r."runVariant"='PRIMARY' AND r."technicalUserId" IS NULL AND r."callerAppChain" IS NULL AND r."completedAt" IS NULL"#,vec![run_id.into(),app_id.into(),payer.into(),now().into()])).await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some((
        row.try_get("", "deadline")?,
        row.try_get("", "generation")?,
    )))
}
pub(super) async fn live_run<C: ConnectionTrait>(
    db: &C,
    run_id: &str,
    app_id: &str,
    payer: &str,
) -> Result<(i64, i64), ApiError> {
    run_record(db, run_id, app_id, payer).await?.ok_or_else(|| {
        error(
            "PAYMENT_RUN_ENDED",
            "The attended execution is no longer running",
        )
    })
}
#[utoipa::path(post,path="/execution/payments",tag="payments", request_body=CreatePayment,security(("executor_jwt"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreatePayment>,
) -> Result<Json<Value>, ApiError> {
    let signed = claims(&headers)?;
    validate(&input)?;
    let config = &state.platform_config.payments;
    if !config.node_payments_enabled {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Interactive payments are disabled",
        ));
    }
    config
        .validate()
        .map_err(|_| error("PAYMENTS_DISABLED", "Payment configuration is incomplete"))?;
    domain::validate_amount(
        input.amount_minor,
        &input.currency,
        50,
        config.max_payment_amount,
    )?;
    let digest = blake3::hash(&serde_json::to_vec(&input)?)
        .to_hex()
        .to_string();
    let id = blake3::hash(&serde_json::to_vec(&json!([
        signed.run_id,
        input.node_id,
        input.nonce
    ]))?)
    .to_hex()
    .to_string();
    if let Some(existing) = payment_request::Entity::find_by_id(&id)
        .one(&state.db)
        .await?
    {
        executor_scope(&signed, &existing)?;
        if existing.request_digest != digest {
            return Err(error(
                "PAYMENT_REPLAY_MISMATCH",
                "This invocation already has different payment parameters",
            ));
        }
        return Ok(Json(view(&state, &existing, false).await?));
    }
    let payee = payee_for_app(&state.db, &signed.app_id).await?;
    if payee == signed.sub {
        return Err(error(
            "PAYMENT_SELF_PURCHASE",
            "The app owner cannot pay their own app",
        ));
    }
    let recipient = accounts::require_can_accept(&state, &payee).await?;
    let scope = connect_scope(&state).await?;
    let account_id = match recipient.connected_account() {
        Some(account) => Some(account.stripe_account_id.clone().ok_or_else(|| {
            error(
                "CONNECT_ONBOARDING_REQUIRED",
                "The app owner payment account is incomplete",
            )
        })?),
        None => None,
    };
    state.transaction(|txn| {
        let (state, signed, input, recipient, account_id, scope, payee, id, digest) =
            (state.clone(),signed.clone(),input.clone(),recipient.clone(),account_id.clone(),scope.clone(),payee.clone(),id.clone(),digest.clone());
        Box::pin(async move {
        let config=&state.platform_config.payments;
    crate::db::coordination::coordinate(txn, "payments-owner", &[&payee]).await?;
    crate::db::coordination::coordinate(txn, "payments-app", &[&signed.app_id]).await?;
    crate::db::coordination::coordinate(txn, "payment-request", &[&id]).await?;
    if let Some(existing) = payment_request::Entity::find_by_id(&id).one(txn).await? {
        if existing.request_digest != digest {
            return Err(error(
                "PAYMENT_REPLAY_MISMATCH",
                "This invocation already has different payment parameters",
            ));
        }
        return Ok::<_,ApiError>(());
    }
    let (deadline, generation) =
        live_run(txn, &signed.run_id, &signed.app_id, &signed.sub).await?;
    if txn.query_one_raw(sql(r#"SELECT id FROM "ExecutionRun" WHERE id=$1 AND "boardId"=$2 AND "eventId" IS NOT DISTINCT FROM $3"#,vec![signed.run_id.clone().into(),signed.board_id.clone().into(),signed.event_id.clone().into()])).await?.is_none(){return Err(ApiError::NOT_FOUND);}
    let expires = domain::request_deadline(now(), input.ttl_seconds, deadline, signed.exp)?;
    let settings = admission(
        txn,
        &state,
        &signed.app_id,
        &payee,
        &recipient,
        input.amount_minor,
    )
    .await?;
    let platform_owned = recipient.platform_owned();
    let fee_bps = if platform_owned { 0 } else { config.node_fee_bps };
    let fee = if platform_owned { 0 } else { domain::fee(input.amount_minor, fee_bps)? };
    let policy = PaymentMethodPolicy {
        version: "interactive-v1".into(),
        charge_model: if platform_owned { ChargeModel::Platform } else { ChargeModel::Direct },
        allowed_methods: config.node_payment_methods.clone(),
        configuration_id: config.node_method_configuration.clone(),
        allow_delayed: false,
    };
    policy.validate().map_err(stripe_error)?;
    let payee_user=crate::entity::user::Entity::find_by_id(&payee).one(txn).await?;
    let account = recipient.connected_account();
    let payee_name=if platform_owned { Some("Flow-Like".to_owned()) } else {
        account.and_then(|account|account.business.as_ref()).and_then(|business|business["name"].as_str()).map(str::to_owned)
        .or_else(||payee_user.and_then(|user|user.username.or(user.name)))
    };
    let app_name=txn.query_one_raw(sql(r#"SELECT name FROM "Meta" WHERE "appId"=$1 ORDER BY CASE WHEN lang='en' THEN 0 ELSE 1 END LIMIT 1"#,vec![signed.app_id.clone().into()])).await?.map(|row|row.try_get::<String>("","name")).transpose()?;
    let snapshot = json!({"platformOwned":platform_owned,"accountRowId":account.map(|account|&account.id),"accountRevision":account.map(|account|account.revision),"methodPolicy":policy,"ownerTermsVersion":if platform_owned {None} else {config.owner_terms_version.as_deref()},"taxMode":if platform_owned {"platform_supplier"} else {"seller_supplier"},"automaticTax":true,"productTaxCode":input.product_tax_code,"shippingCountries":input.shipping_countries,"payeeDisplayName":payee_name,"appName":app_name});
    reserve_limit(
        txn,
        &payee,
        &scope,
        &id,
        input.amount_minor,
        config.new_seller_daily_cap,
        expires,
    )
    .await?;
    txn.execute_raw(sql(r#"INSERT INTO "PaymentRequest" (id,"runId","nodeId",nonce,"requestDigest","appId","boardId","eventId","payerUserId","payeeUserId","connectedAccountId","platformAccountId",livemode,amount,currency,"applicationFeeAmount","feeBps","productName",description,reference,snapshot,status,"runRevision","settingsRevision","expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,'CREATED',$22,$23,$24,$25,$25,$25)"#,vec![id.clone().into(),signed.run_id.into(),input.node_id.into(),input.nonce.into(),digest.into(),signed.app_id.into(),signed.board_id.into(),signed.event_id.into(),signed.sub.into(),payee.into(),account_id.into(),scope.platform_account_id.into(),scope.livemode.into(),input.amount_minor.into(),input.currency.into(),fee.into(),i32::from(fee_bps).into(),input.product_name.into(),input.description.into(),input.reference.into(),snapshot.into(),generation.into(),settings.revision.into(),expires.into(),now().into()])).await?;
    Ok::<_,ApiError>(())
        })
    }).await?;
    Ok(Json(view(&state, &load(&state, &id).await?, false).await?))
}
pub(super) async fn admission<C: ConnectionTrait>(
    db: &C,
    state: &AppState,
    app: &str,
    payee: &str,
    recipient: &accounts::PaymentRecipient,
    amount: i64,
) -> Result<app_payment_settings::Model, ApiError> {
    if !state.platform_config.payments.node_payments_enabled
        || payee_for_app(db, app).await? != payee
    {
        return Err(error("PAYMENTS_DISABLED", "The app payment owner changed"));
    }
    let config = &state.platform_config.payments;
    config
        .validate()
        .map_err(|_| error("PAYMENTS_DISABLED", "Payment configuration is incomplete"))?;
    accounts::require_seller_allowed(db, &state.platform_config.payments, payee).await?;
    if accounts::is_platform_admin(db, payee).await? != recipient.platform_owned() {
        return Err(error(
            "PAYMENT_RECIPIENT_CHANGED",
            "The app payment recipient changed",
        ));
    }
    if let Some(account) = recipient.connected_account() {
        require_owner_terms(db, config, &account.id, payee).await?;
    }

    let settings = app_payment_settings::Entity::find_by_id(app)
        .one(db)
        .await?
        .ok_or_else(|| {
            error(
                "PAYMENTS_DISABLED",
                "The owner has not enabled payments for this app",
            )
        })?;
    if settings.owner_user_id != payee
        || !settings.payments_enabled
        || settings.admin_blocked_at.is_some()
        || settings
            .max_payment_amount
            .as_ref()
            .and_then(|v| v["eur"].as_i64())
            .is_some_and(|max| amount > max)
    {
        return Err(error(
            "PAYMENTS_DISABLED",
            "App payment settings do not permit this amount",
        ));
    }
    if let Some(account) = recipient.connected_account() {
        let fenced=db.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET revision=revision+1 WHERE id=$1 AND revision=$2 AND "stripeAccountId"=$3 AND "retiredAt" IS NULL AND "canAcceptPayments"=true AND EXISTS (SELECT 1 FROM "PaymentAccountBinding" b WHERE b."activeAccountId"=$1 AND b."userId"=$4)"#,vec![account.id.clone().into(),account.revision.into(),account.stripe_account_id.clone().into(),payee.into()])).await?;
        if fenced.rows_affected() != 1 {
            return Err(error(
                "PAYMENT_ACCOUNT_CHANGED",
                "The payment account changed. Retry the request",
            ));
        }
    }
    Ok(settings)
}
pub(crate) async fn require_owner_terms<C: ConnectionTrait>(
    db: &C,
    config: &flow_like::hub::PaymentsConfig,
    account_row: &str,
    payee: &str,
) -> Result<(), ApiError> {
    let consent=db.query_one_raw(sql(r#"SELECT l."textHash",l.locale FROM "ConnectedAccount" c JOIN "LegalConsent" l ON l.id=c."consentId" WHERE c.id=$1 AND l."userId"=$2 AND l.kind='PAYMENTS_OWNER_TERMS' AND l."subjectType"='CONNECTED_ACCOUNT' AND l."subjectId"=$1 AND l."textVersion"=$3 AND l.accepted=true"#,vec![account_row.into(),payee.into(),config.owner_terms_version.clone().into()])).await?.ok_or_else(||error("PAYMENT_TERMS_REQUIRED","The owner must accept the current payment agreement"))?;
    let consent_hash: String = consent.try_get("", "textHash")?;
    let consent_locale: String = consent.try_get("", "locale")?;
    let text = config
        .legal_texts
        .iter()
        .find(|text| {
            text.kind == "PAYMENTS_OWNER_TERMS"
                && Some(text.version.as_str()) == config.owner_terms_version.as_deref()
                && text.locale == consent_locale
        })
        .ok_or_else(|| error("PAYMENT_TERMS_REQUIRED", "The owner agreement changed"))?;
    let content = text
        .content()
        .map_err(|message| error("PAYMENT_TERMS_REQUIRED", &message))?;
    if blake3::hash(content.as_bytes()).to_hex().as_str() != consent_hash {
        return Err(error(
            "PAYMENT_TERMS_REQUIRED",
            "The owner agreement changed",
        ));
    }
    Ok(())
}

async fn reserve_limit<C: ConnectionTrait>(
    db: &C,
    payee: &str,
    scope: &StripeScope,
    id: &str,
    amount: i64,
    cap: i64,
    expires: i64,
) -> Result<(), ApiError> {
    let day = now() / 86_400_000;
    let key = format!(
        "seller:{}:{}:{payee}:{day}:eur",
        scope.platform_account_id, scope.livemode
    );
    let end = (day + 1) * 86_400_000;
    db.execute_raw(sql(r#"INSERT INTO "PaymentLimitCounter" (key,currency,"windowEnd","createdAt","updatedAt") VALUES ($1,'eur',$2,$3,$3) ON CONFLICT DO NOTHING"#,vec![key.clone().into(),end.into(),now().into()])).await?;
    let changed=db.execute_raw(sql(r#"UPDATE "PaymentLimitCounter" SET amount=amount+$2,count=count+1,revision=revision+1,"updatedAt"=$3 WHERE key=$1 AND amount<=$4-$2 AND count<1000"#,vec![key.clone().into(),amount.into(),now().into(),cap.into()])).await?;
    if changed.rows_affected() != 1 {
        return Err(error(
            "PAYMENT_LIMIT_REACHED",
            "The seller daily payment limit has been reached",
        ));
    }
    db.execute_raw(sql(r#"INSERT INTO "PaymentLimitReservation" (id,"counterKey","sourceType","sourceId",amount,"expiresAt","createdAt","updatedAt") VALUES ($1,$2,'REQUEST',$1,$3,$4,$5,$5)"#,vec![id.into(),key.into(),amount.into(),expires.into(),now().into()])).await?;
    Ok(())
}
pub(super) async fn load(state: &AppState, id: &str) -> Result<payment_request::Model, ApiError> {
    payment_request::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)
}
pub(super) async fn attempt(
    state: &AppState,
    id: &str,
) -> Result<Option<payment_attempt::Model>, ApiError> {
    Ok(payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::SourceId.eq(id))
        .order_by_desc(payment_attempt::Column::Attempt)
        .one(&state.db)
        .await?)
}
pub(super) async fn view(
    state: &AppState,
    row: &payment_request::Model,
    include_url: bool,
) -> Result<Value, ApiError> {
    let attempt = attempt(state, &row.id).await?;
    Ok(
        json!({"id":row.id,"appId":row.app_id,"runId":row.run_id,"status":row.status,"reason":row.reason,"expiresAt":row.expires_at,"amountMinor":row.amount,"currency":row.currency,"productName":row.product_name,"description":row.description,"payeeUserId":row.payee_user_id,"platformOwned":platform_owned(row),"payeeDisplayName":row.snapshot["payeeDisplayName"],"appName":row.snapshot["appName"],"receiptUrl":attempt.as_ref().and_then(|a|a.snapshot["receiptUrl"].as_str()),"checkoutUrl":attempt.as_ref().filter(|_|include_url && !row.cancel_requested && row.expires_at>now()).and_then(|a|a.checkout_url.as_ref()),"refundedAmount":attempt.as_ref().map(|a|a.refunded_amount).unwrap_or(0),"pendingRefundAmount":attempt.as_ref().map(|a|a.reserved_refund_amount).unwrap_or(0)}),
    )
}
fn executor_scope(claims: &ExecutionClaims, row: &payment_request::Model) -> Result<(), ApiError> {
    if claims.run_id != row.run_id
        || claims.app_id != row.app_id
        || claims.payer_sub.as_deref() != Some(&row.payer_user_id)
        || row.board_id.as_deref() != Some(claims.board_id.as_str())
        || claims.event_id != row.event_id
    {
        Err(ApiError::NOT_FOUND)
    } else {
        Ok(())
    }
}
fn payer_scope(
    user: &AppUser,
    scope: &PayerScope,
    row: &payment_request::Model,
) -> Result<(), ApiError> {
    if accounts::session_user(user)? != row.payer_user_id
        || scope.app_id != row.app_id
        || scope.run_id != row.run_id
    {
        Err(ApiError::NOT_FOUND)
    } else {
        Ok(())
    }
}
#[utoipa::path(get,path="/execution/payments/{id}",tag="payments", params(("id"=String,Path,description="Payment request id")),security(("executor_jwt"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn executor_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let c = claims(&headers)?;
    let row = load(&state, &id).await?;
    executor_scope(&c, &row)?;
    if row.next_check_at <= now() {
        reconcile_request(&state, &id).await?;
    }
    Ok(Json(view(&state, &load(&state, &id).await?, false).await?))
}
#[utoipa::path(post,path="/execution/payments/{id}/cancel",tag="payments", params(("id"=String,Path,description="Payment request id")),security(("executor_jwt"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn executor_cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let c = claims(&headers)?;
    let row = load(&state, &id).await?;
    executor_scope(&c, &row)?;
    cancel(&state, &id, "RUN_CANCELED").await?;
    Ok(Json(view(&state, &load(&state, &id).await?, false).await?))
}
#[utoipa::path(get,path="/payments/{id}",tag="payments", params(("id"=String,Path,description="Payment request id"),("appId"=String,Query,description="Recorded app id"),("runId"=String,Query,description="Recorded run id")),security(("bearer_auth"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn payer_get(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(scope): Query<PayerScope>,
) -> Result<Json<Value>, ApiError> {
    let row = load(&state, &id).await?;
    payer_scope(&user, &scope, &row)?;
    if row.next_check_at <= now() {
        reconcile_request(&state, &id).await?;
    }
    Ok(Json(view(&state, &load(&state, &id).await?, true).await?))
}
#[utoipa::path(post,path="/payments/{id}/decline",tag="payments", params(("id"=String,Path,description="Payment request id"),("appId"=String,Query,description="Recorded app id"),("runId"=String,Query,description="Recorded run id")),security(("bearer_auth"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn decline(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(scope): Query<PayerScope>,
) -> Result<Json<Value>, ApiError> {
    let row = load(&state, &id).await?;
    payer_scope(&user, &scope, &row)?;
    cancel(&state, &id, "PAYER_DECLINED").await?;
    Ok(Json(view(&state, &load(&state, &id).await?, false).await?))
}
#[utoipa::path(post,path="/payments/{id}/report",tag="payments", request_body=ReportInput, params(("id"=String,Path,description="Payment request id"),("appId"=String,Query,description="Recorded app id"),("runId"=String,Query,description="Recorded run id")),security(("bearer_auth"=[])),responses((status=200,description="Report recorded"),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn report(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(scope): Query<PayerScope>,
    Json(input): Json<ReportInput>,
) -> Result<Json<Value>, ApiError> {
    let row = load(&state, &id).await?;
    payer_scope(&user, &scope, &row)?;
    if input.reason.trim().is_empty() || input.reason.chars().count() > 1000 {
        return Err(ApiError::bad_request(
            "Report reason must contain 1 to 1000 characters",
        ));
    }
    outbox::enqueue(
        &state.db,
        &format!("payment-report:{id}"),
        "PAYMENT_REPORT",
        SOURCE,
        &id,
        json!({"payerUserId":row.payer_user_id,"reason":input.reason}),
    )
    .await?;
    cancel(&state, &id, "PAYER_REPORTED").await?;
    Ok(Json(json!({"reported":true})))
}
pub async fn cancel(state: &AppState, id: &str, reason: &str) -> Result<(), ApiError> {
    state
        .transaction(|txn| {
            let (id, reason) = (id.to_owned(), reason.to_owned());
            Box::pin(async move { cancel_in_transaction(txn, &id, &reason).await })
        })
        .await
}
async fn cancel_in_transaction(
    db: &sea_orm::DatabaseTransaction,
    id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(db, "payment-request", &[id]).await?;
    let updated=db.execute_raw(sql(r#"UPDATE "PaymentRequest" SET "cancelRequested"=true,status='CANCEL_PENDING',reason=COALESCE(reason,$2),"updatedAt"=$3,revision=revision+1 WHERE id=$1 AND status IN ('CREATED','OPENING','OPEN','PROCESSING','CANCEL_PENDING')"#,vec![id.into(),reason.into(),now().into()])).await?;
    if updated.rows_affected() > 0 {
        outbox::enqueue(
            db,
            &format!("request:{id}:cancel"),
            "node_reconcile",
            SOURCE,
            id,
            json!({}),
        )
        .await?;
    }
    Ok(())
}
pub async fn cancel_run(state: &AppState, run_id: &str, reason: &str) -> Result<(), ApiError> {
    cancel_run_database(&state.db, state.db_dialect, run_id, reason).await
}
pub(crate) async fn cancel_run_database(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    run_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    crate::db::retry_transaction(
        db,
        dialect,
        None,
        &crate::db::RetryPolicy::default(),
        |txn| {
            let (run_id, reason) = (run_id.to_owned(), reason.to_owned());
            Box::pin(async move {
                let rows = payment_request::Entity::find()
                    .filter(payment_request::Column::RunId.eq(&run_id))
                    .filter(payment_request::Column::Status.is_in([
                        "CREATED",
                        "OPENING",
                        "OPEN",
                        "PROCESSING",
                        "CANCEL_PENDING",
                    ]))
                    .all(txn)
                    .await?;
                for row in rows {
                    cancel_in_transaction(txn, &row.id, &reason).await?;
                }
                Ok::<_, ApiError>(())
            })
        },
    )
    .await
}
#[utoipa::path(post,path="/payments/{id}/checkout",tag="payments", params(("id"=String,Path,description="Payment request id"),("appId"=String,Query,description="Recorded app id"),("runId"=String,Query,description="Recorded run id")),security(("bearer_auth"=[])),responses((status=200,description="Authoritative payment state",body=PaymentView),(status=404,description="Unknown request or payer scope"),(status=409,description="Payment policy rejected the request")))]
pub async fn checkout(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(scope): Query<PayerScope>,
) -> Result<Json<Value>, ApiError> {
    let row = load(&state, &id).await?;
    payer_scope(&user, &scope, &row)?;
    if row.status == "PAID" {
        return Ok(Json(view(&state, &row, false).await?));
    }
    if row.cancel_requested || row.expires_at <= now() {
        return Err(error("PAYMENT_EXPIRED", "This payment request has ended"));
    }
    let attempt_id = flow_like_types::create_id();
    let expires = now() + 1_860_000;
    let policy: PaymentMethodPolicy = serde_json::from_value(row.snapshot["methodPolicy"].clone())?;
    let mut return_url = flow_like_types::reqwest::Url::parse(
        state
            .platform_config
            .payments
            .frontend_url
            .as_deref()
            .ok_or_else(|| error("PAYMENTS_DISABLED", "Payment return URL is not configured"))?,
    )
    .map_err(|_| ApiError::internal("Invalid payment return URL"))?;
    return_url.set_path("/payments/return");
    return_url.set_query(None);
    return_url.set_fragment(None);
    return_url
        .query_pairs_mut()
        .append_pair("id", &id)
        .append_pair("appId", &row.app_id)
        .append_pair("runId", &row.run_id);
    let params = checkout_parameters(&row, &attempt_id, expires, return_url.as_str(), &policy)?;
    let stripe_scope = request_stripe_scope(&row)?;
    if row.status == "CREATED" && automatic_tax(&row) {
        super::tax::require_ready(&state, &stripe_scope, product_tax_code(&row)?).await?;
    }
    let request = StripeRequest::post(
        stripe_scope,
        "/v1/checkout/sessions",
        params,
        format!("request:{id}:session:1"),
    );
    state.transaction(|txn| {
        let (state,row,id,attempt_id,request)=(state.clone(),row.clone(),id.clone(),attempt_id.clone(),request.clone());
        Box::pin(async move {
    crate::db::coordination::coordinate(txn, "payments-owner", &[&row.payee_user_id]).await?;
    crate::db::coordination::coordinate(txn, "payments-app", &[&row.app_id]).await?;
    crate::db::coordination::coordinate(txn, "payment-request", &[&id]).await?;
    let current = payment_request::Entity::find_by_id(&id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    live_run(txn, &row.run_id, &row.app_id, &row.payer_user_id).await?;
    if current.cancel_requested || current.expires_at <= now() {
        return Err(error("PAYMENT_EXPIRED", "This payment request has ended"));
    }
    let recipient = match row.connected_account_id.as_deref() {
        Some(account_id) => {
            let acct_row = row.snapshot["accountRowId"].as_str()
                .ok_or_else(|| ApiError::internal("Missing account snapshot"))?;
            let account = connected_account::Entity::find_by_id(acct_row).one(txn).await?
                .ok_or(ApiError::NOT_FOUND)?;
            if account.stripe_account_id.as_deref() != Some(account_id) {
                return Err(error("PAYMENT_ACCOUNT_CHANGED", "The payment account changed"));
            }
            accounts::PaymentRecipient::Connected(account)
        },
        None => accounts::PaymentRecipient::Platform,
    };
    admission(txn, &state, &row.app_id, &row.payee_user_id, &recipient, row.amount).await?;
    if current.status == "CREATED" {
        let operation = operations::prepare(txn, SOURCE, &id, "checkout_create", &request).await?;
        txn.execute_raw(sql(r#"INSERT INTO "PaymentAttempt" (id,"sourceType","sourceId",attempt,"platformAccountId","scopeKey","connectedAccountId",livemode,"operationId","payerUserId","payeeUserId","appId",amount,currency,"applicationFeeAmount","feeBps",snapshot,"expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,'REQUEST',$2,1,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17,$17)"#,vec![attempt_id.into(),id.clone().into(),row.platform_account_id.into(),row.connected_account_id.clone().unwrap_or_else(|| "platform".into()).into(),row.connected_account_id.into(),row.livemode.into(),operation.into(),row.payer_user_id.into(),row.payee_user_id.into(),row.app_id.into(),row.amount.into(),row.currency.into(),row.application_fee_amount.into(),row.fee_bps.into(),row.snapshot.into(),expires.into(),now().into()])).await?;
        txn.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status='OPENING',"nextCheckAt"=$2,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND status='CREATED' AND "cancelRequested"=false"#,vec![id.clone().into(),now().into()])).await?;
        outbox::enqueue(
            txn,
            &format!("request:{id}:open"),
            "node_reconcile",
            SOURCE,
            &id,
            json!({}),
        )
        .await?;
    }
    Ok::<_,ApiError>(())
        })
    }).await?;
    reconcile_request(&state, &id).await?;
    Ok(Json(view(&state, &load(&state, &id).await?, true).await?))
}

pub(super) fn platform_owned(row: &payment_request::Model) -> bool {
    row.snapshot["platformOwned"].as_bool().unwrap_or(false)
}

pub(super) fn automatic_tax(row: &payment_request::Model) -> bool {
    row.snapshot["automaticTax"].as_bool().unwrap_or(false)
}

fn product_tax_code(row: &payment_request::Model) -> Result<&str, ApiError> {
    row.snapshot["productTaxCode"]
        .as_str()
        .filter(|code| valid_tax_code(code))
        .ok_or_else(|| {
            error(
                "PAYMENT_TAX_CODE_REQUIRED",
                "The payment has no valid product tax code",
            )
        })
}

pub(super) fn request_stripe_scope(row: &payment_request::Model) -> Result<StripeScope, ApiError> {
    let scope = StripeScope::platform(row.platform_account_id.clone(), row.livemode);
    match (platform_owned(row), row.connected_account_id.as_deref()) {
        (true, None) if row.application_fee_amount == 0 && row.fee_bps == 0 => Ok(scope),
        (false, Some(account)) => Ok(scope.connected(account)),
        _ => Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "The recorded payment recipient is inconsistent",
        )),
    }
}

fn checkout_parameters(
    row: &payment_request::Model,
    attempt_id: &str,
    expires: i64,
    return_url: &str,
    policy: &PaymentMethodPolicy,
) -> Result<Value, ApiError> {
    request_stripe_scope(row)?;
    if policy.charge_model
        != if platform_owned(row) {
            ChargeModel::Platform
        } else {
            ChargeModel::Direct
        }
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "The recorded payment method policy changed",
        ));
    }
    let mut params = json!({"mode":"payment","ui_mode":"hosted_page","line_items":[{"quantity":1,"price_data":{"currency":row.currency,"unit_amount":row.amount,"product_data":{"name":row.product_name,"description":if row.description.is_empty(){None}else{Some(row.description.clone())}}}}],"payment_intent_data":{"metadata":{"flowlike_attempt":attempt_id,"flowlike_kind":"request"}},"metadata":{"flowlike_attempt":attempt_id,"flowlike_kind":"request"},"client_reference_id":row.id,"success_url":return_url,"cancel_url":return_url,"expires_at":expires/1000,"locale":"auto"});
    if automatic_tax(row) {
        params["line_items"][0]["price_data"]["tax_behavior"] = json!("inclusive");
        params["line_items"][0]["price_data"]["product_data"]["tax_code"] =
            json!(product_tax_code(row)?);
        // Self means the account in this request's immutable Stripe scope.
        params["automatic_tax"] = json!({"enabled":true,"liability":{"type":"self"}});
        params["billing_address_collection"] = json!("required");
        params["customer_creation"] = json!("always");
        params["invoice_creation"] =
            json!({"enabled":true,"invoice_data":{"issuer":{"type":"self"}}});
        let countries: Vec<String> = match row.snapshot.get("shippingCountries") {
            None => Vec::new(),
            Some(countries) => serde_json::from_value(countries.clone())
                .map_err(|_| ApiError::bad_request("The payment shipping countries are invalid"))?,
        };
        if !valid_shipping_countries(&countries) {
            return Err(ApiError::bad_request(
                "The payment shipping countries are invalid",
            ));
        }
        if !countries.is_empty() {
            params["shipping_address_collection"] = json!({"allowed_countries":countries});
        }
    }
    if !platform_owned(row) {
        params["payment_intent_data"]["application_fee_amount"] = json!(row.application_fee_amount);
    }
    for (key, value) in policy
        .checkout_parameters()
        .map_err(stripe_error)?
        .as_object()
        .ok_or_else(|| ApiError::internal("Invalid method policy"))?
    {
        params[key] = value.clone();
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_fields_bound_nonce_text_and_lifetime() {
        let mut input = CreatePayment {
            node_id: "node".into(),
            nonce: "invocation".into(),
            amount_minor: 100,
            currency: "eur".into(),
            product_name: "Access".into(),
            product_tax_code: "txcd_10000000".into(),
            shipping_countries: Vec::new(),
            description: String::new(),
            reference: None,
            ttl_seconds: 300,
        };
        assert!(validate(&input).is_ok());
        input.nonce = "../replay".into();
        assert!(validate(&input).is_err());
        input.nonce = "valid".into();
        input.ttl_seconds = 0;
        assert!(validate(&input).is_err());
    }
    pub(super) fn request_fixture() -> payment_request::Model {
        serde_json::from_value(json!({"id":"payment","run_id":"run","node_id":"node","nonce":"nonce","request_digest":"digest","app_id":"app","board_id":"board","event_id":"event","payer_user_id":"payer","payee_user_id":"seller","connected_account_id":"acct_seller","platform_account_id":"acct_platform","livemode":false,"amount":100,"currency":"eur","application_fee_amount":1,"fee_bps":100,"product_name":"Product","description":"","snapshot":{},"status":"OPEN","cancel_requested":false,"expires_at":1000,"next_check_at":0,"revision":0,"created_at":0,"updated_at":0})).unwrap()
    }
    fn executor_fixture() -> ExecutionClaims {
        serde_json::from_value(json!({"sub":"payer","payer_sub":"payer","run_id":"run","app_id":"app","board_id":"board","event_id":"event","callback_url":"https://api.example","typ":"executor","iss":"flow-like","aud":"flow-like-executor","iat":1,"nbf":1,"exp":100,"jti":"token"})).unwrap()
    }
    #[test]
    fn executor_reads_require_exact_payer_run_app_board_and_event() {
        let row = request_fixture();
        let claims = executor_fixture();
        assert!(executor_scope(&claims, &row).is_ok());
        for field in ["run_id", "app_id", "board_id", "event_id", "payer_sub"] {
            let mut value = serde_json::to_value(&claims).unwrap();
            value[field] = json!("other");
            let changed: ExecutionClaims = serde_json::from_value(value).unwrap();
            assert_eq!(
                executor_scope(&changed, &row).unwrap_err().status(),
                axum::http::StatusCode::NOT_FOUND
            );
        }
        let mut legacy = claims;
        legacy.payer_sub = None;
        assert!(executor_scope(&legacy, &row).is_err());
    }
    #[test]
    fn payer_read_hides_other_users_and_rejects_pat_even_for_same_subject() {
        let row = request_fixture();
        let scope = PayerScope {
            app_id: "app".into(),
            run_id: "run".into(),
        };
        let user = |sub: &str| {
            AppUser::OpenID(crate::middleware::jwt::OpenIDUser {
                sub: sub.into(),
                access_token: "unused".into(),
            })
        };
        assert!(payer_scope(&user("payer"), &scope, &row).is_ok());
        assert_eq!(
            payer_scope(&user("other"), &scope, &row)
                .unwrap_err()
                .status(),
            axum::http::StatusCode::NOT_FOUND
        );
        let pat = AppUser::PAT(crate::middleware::jwt::PATUser {
            sub: "payer".into(),
            pat: "unused".into(),
        });
        assert!(payer_scope(&pat, &scope, &row).is_err());
    }
    #[test]
    fn amount_is_integer_and_callers_cannot_supply_payment_identity() {
        let base = json!({"nodeId":"node","nonce":"nonce","amountMinor":100,"currency":"eur","productName":"Product","productTaxCode":"txcd_10000000","description":"","ttlSeconds":300});
        assert!(serde_json::from_value::<CreatePayment>(base.clone()).is_ok());
        let mut fractional = base.clone();
        fractional["amountMinor"] = json!(100.5);
        assert!(serde_json::from_value::<CreatePayment>(fractional).is_err());
        let mut identity = base;
        identity["payerUserId"] = json!("victim");
        assert!(serde_json::from_value::<CreatePayment>(identity).is_err());
    }

    #[test]
    fn real_requests_require_explicit_classification_and_valid_shipping_countries() {
        let base = json!({"nodeId":"node","nonce":"nonce","amountMinor":100,"currency":"eur","productName":"Product","productTaxCode":"txcd_10000000","description":"","ttlSeconds":300});
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove("productTaxCode");
        assert!(serde_json::from_value::<CreatePayment>(missing).is_err());
        for code in [
            "",
            "10000000",
            "txcd_1000000",
            "txcd_100000000",
            "txcd_abcdefgh",
        ] {
            let mut input: CreatePayment = serde_json::from_value(base.clone()).unwrap();
            input.product_tax_code = code.into();
            assert!(
                validate(&input).is_err(),
                "accepted invalid tax code: {code}"
            );
        }
        let mut input: CreatePayment = serde_json::from_value(base).unwrap();
        input.shipping_countries = vec!["DE".into(), "FR".into()];
        assert!(validate(&input).is_ok());
        for countries in [
            vec!["de".into()],
            vec!["DE".into(), "DE".into()],
            vec!["DEU".into()],
        ] {
            input.shipping_countries = countries;
            assert!(validate(&input).is_err());
        }
    }
}
