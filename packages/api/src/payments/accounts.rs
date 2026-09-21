use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    connect_scope, ensure_app_owner, error, operations, outbox, payee_for_app, sql, stripe_error,
};
use crate::{
    auth::AppUser,
    entity::{connected_account, payment_account_binding},
    error::ApiError,
    permission::global_permission::GlobalPermission,
    state::AppState,
    stripe_connect::{StripeGateway, StripeRequest, types::Account},
};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectView {
    pub platform_owned: bool,
    pub state: String,
    pub can_accept_payments: bool,
    pub can_sell: bool,
    pub payouts_enabled: bool,
    pub country: Option<String>,
    pub default_currency: Option<String>,
    pub business: Option<Value>,
    pub requirements: Option<Value>,
    pub dashboard_url: String,
}

impl From<Option<connected_account::Model>> for ConnectView {
    fn from(account: Option<connected_account::Model>) -> Self {
        match account {
            None => Self {
                platform_owned: false,
                state: "not_started".into(),
                can_accept_payments: false,
                can_sell: false,
                payouts_enabled: false,
                country: None,
                default_currency: None,
                business: None,
                requirements: None,
                dashboard_url: "https://dashboard.stripe.com".into(),
            },
            Some(account) => Self {
                platform_owned: false,
                state: account.state,
                can_accept_payments: account.can_accept_payments && account.retired_at.is_none(),
                can_sell: account.can_sell && account.retired_at.is_none(),
                payouts_enabled: account.payouts_enabled,
                country: Some(account.country),
                default_currency: account.default_currency,
                business: account.business,
                requirements: account.requirements,
                dashboard_url: "https://dashboard.stripe.com".into(),
            },
        }
    }
}

/// The recipient is resolved from the owner's current global role for each new payment.
#[derive(Clone, Debug)]
pub enum PaymentRecipient {
    Platform,
    Connected(connected_account::Model),
}

impl PaymentRecipient {
    pub fn platform_owned(&self) -> bool {
        matches!(self, Self::Platform)
    }

    pub fn connected_account(&self) -> Option<&connected_account::Model> {
        match self {
            Self::Platform => None,
            Self::Connected(account) => Some(account),
        }
    }
}

pub async fn is_platform_admin<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
) -> Result<bool, ApiError> {
    let bits = crate::entity::user::Entity::find_by_id(user_id)
        .select_only()
        .column(crate::entity::user::Column::Permission)
        .into_tuple::<i64>()
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let permission = GlobalPermission::from_bits(bits).ok_or(ApiError::FORBIDDEN)?;
    Ok(permission.contains(GlobalPermission::Admin))
}

async fn require_connected_owner<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
) -> Result<(), ApiError> {
    if is_platform_admin(db, user_id).await? {
        return Err(error(
            "PLATFORM_PAYMENTS_MANAGED",
            "Admin-owned apps collect payments in Flow-Like's Stripe account; connected account setup is unavailable",
        ));
    }
    Ok(())
}

async fn platform_view(state: &AppState, user_id: &str) -> Result<ConnectView, ApiError> {
    let config = &state.platform_config.payments;
    let allowed = config.validate().is_ok()
        && require_seller_allowed(&state.db, config, user_id)
            .await
            .is_ok();
    Ok(ConnectView {
        platform_owned: true,
        state: "platform".into(),
        can_accept_payments: allowed && config.node_payments_enabled,
        can_sell: allowed && config.marketplace_enabled,
        payouts_enabled: false,
        country: None,
        default_currency: Some("eur".into()),
        business: None,
        requirements: None,
        dashboard_url: "https://dashboard.stripe.com".into(),
    })
}

pub fn session_user(user: &AppUser) -> Result<&str, ApiError> {
    match user {
        AppUser::OpenID(user) => Ok(&user.sub),
        _ => Err(ApiError::coded(
            axum::http::StatusCode::FORBIDDEN,
            "CONNECT_REQUIRES_SESSION",
            "Sign in with your account to manage payments",
        )),
    }
}

pub async fn recent_session(state: &AppState, user: &AppUser) -> Result<String, ApiError> {
    let AppUser::OpenID(user) = user else {
        return session_user(user).map(str::to_owned);
    };
    let claims = state
        .validate_token(&user.access_token)
        .await
        .map_err(|_| ApiError::UNAUTHORIZED)?;
    let now = Utc::now().timestamp();
    if !claims
        .claims
        .get("auth_time")
        .and_then(Value::as_i64)
        .is_some_and(|time| time <= now + 30 && now - time <= 600)
    {
        return Err(ApiError::coded(
            axum::http::StatusCode::UNAUTHORIZED,
            "REAUTH_REQUIRED",
            "Sign in again with recent authentication to change your payment account",
        ));
    }
    Ok(user.sub.clone())
}

pub async fn account_for_user(
    state: &AppState,
    user_id: &str,
) -> Result<Option<connected_account::Model>, ApiError> {
    let binding = payment_account_binding::Entity::find()
        .filter(payment_account_binding::Column::UserId.eq(user_id))
        .filter(
            payment_account_binding::Column::Livemode.eq(state.platform_config.payments.livemode),
        )
        .filter(payment_account_binding::Column::Purpose.eq("shared"))
        .one(&state.db)
        .await?;
    let Some(id) = binding.and_then(|row| row.active_account_id.or(row.candidate_account_id))
    else {
        return Ok(None);
    };
    Ok(connected_account::Entity::find_by_id(id)
        .one(&state.db)
        .await?)
}

pub async fn require_seller_allowed<C: ConnectionTrait>(
    db: &C,
    config: &flow_like::hub::PaymentsConfig,
    user_id: &str,
) -> Result<(), ApiError> {
    if !config.allows_seller_id(user_id) {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Payments are not enabled for this seller",
        ));
    }
    if db
        .query_one_raw(sql(
            r#"SELECT "userId" FROM "PaymentsBlock" WHERE "userId"=$1"#,
            vec![user_id.into()],
        ))
        .await?
        .is_some()
    {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Payments are paused for this seller",
        ));
    }
    Ok(())
}

pub async fn require_can_sell(
    state: &AppState,
    user_id: &str,
) -> Result<PaymentRecipient, ApiError> {
    let account = require_can_accept(state, user_id).await?;
    if account
        .connected_account()
        .is_some_and(|account| !account.can_sell)
    {
        return Err(error(
            "CONNECT_ONBOARDING_REQUIRED",
            "Complete your payment account setup before selling",
        ));
    }
    Ok(account)
}

pub async fn require_can_accept(
    state: &AppState,
    user_id: &str,
) -> Result<PaymentRecipient, ApiError> {
    let config = &state.platform_config.payments;
    config
        .validate()
        .map_err(|_| error("PAYMENTS_DISABLED", "Payment configuration is incomplete"))?;
    require_seller_allowed(&state.db, config, user_id).await?;
    if is_platform_admin(&state.db, user_id).await? {
        return Ok(PaymentRecipient::Platform);
    }
    let mut account = account_for_user(state, user_id).await?.ok_or_else(|| {
        error(
            "CONNECT_ONBOARDING_REQUIRED",
            "Set up your payment account first",
        )
    })?;
    if account.retired_at.is_some() {
        return Err(error(
            "CONNECT_ONBOARDING_REQUIRED",
            "Reconnect your payment account first",
        ));
    }
    if account
        .synced_at
        .is_none_or(|time| Utc::now().timestamp_millis() - time > 300_000)
    {
        sync_account(state, &account.id).await?;
        account = connected_account::Entity::find_by_id(&account.id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
    }
    if !account.can_accept_payments || account.retired_at.is_some() {
        return Err(error(
            "CONNECT_ONBOARDING_REQUIRED",
            "Complete your payment account setup before accepting payments",
        ));
    }
    Ok(PaymentRecipient::Connected(account))
}

pub fn derive_state(account: &Account) -> (&'static str, bool, bool) {
    let reason = account
        .requirements
        .disabled_reason
        .as_deref()
        .unwrap_or("");
    let ready = account.charges_enabled
        && account
            .capabilities
            .get("card_payments")
            .is_some_and(|value| value == "active");
    let transfers = account
        .capabilities
        .get("transfers")
        .is_some_and(|value| value == "active");
    let due = !account.requirements.currently_due.is_empty()
        || !account.requirements.past_due.is_empty()
        || !account.requirements.errors.is_empty();
    let known_reason = matches!(
        reason,
        "" | "requirements.past_due"
            | "requirements.pending_verification"
            | "under_review"
            | "listed"
            | "other"
    );
    let state = if reason.starts_with("rejected.") {
        "rejected"
    } else if !known_reason {
        "limited"
    } else if !account.details_submitted {
        "onboarding_incomplete"
    } else if ready {
        if due {
            "enabled_action_needed"
        } else {
            "enabled"
        }
    } else if due {
        "action_required"
    } else if !account.requirements.pending_verification.is_empty()
        || reason == "requirements.pending_verification"
    {
        "pending_verification"
    } else if matches!(reason, "under_review" | "listed" | "other") {
        "under_review"
    } else {
        "limited"
    };
    let accepting = ready && matches!(state, "enabled" | "enabled_action_needed");
    (state, accepting, accepting && transfers)
}

pub async fn sync_account(state: &AppState, id: &str) -> Result<(), ApiError> {
    let local = connected_account::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if local.retired_at.is_some() {
        return Ok(());
    }
    let Some(stripe_id) = local.stripe_account_id else {
        return Ok(());
    };
    let now = Utc::now().timestamp_millis();
    let claimed=state.db.query_one_raw(sql(r#"UPDATE "ConnectedAccount" SET "syncRevision"="syncRevision"+1,"syncLeaseUntil"=$3,"syncAttempts"="syncAttempts"+1 WHERE id=$1 AND "retiredAt" IS NULL AND "syncNextAt"<=$2 AND ("syncLeaseUntil" IS NULL OR "syncLeaseUntil"<$2) RETURNING "syncRevision","syncAttempts""#,vec![id.into(),now.into(),(now+60_000).into()])).await?.ok_or_else(||error("PAYMENT_OPERATION_PENDING","Account status is already refreshing; try again shortly"))?;
    let rev: i64 = claimed.try_get("", "syncRevision")?;
    let attempts: i32 = claimed.try_get("", "syncAttempts")?;
    let fetch = async {
        let scope = connect_scope(state).await?;
        if scope.platform_account_id != local.platform_account_id
            || scope.livemode != local.livemode
        {
            return Err(error(
                "PAYMENT_ACCOUNT_MISMATCH",
                "The configured platform does not own this account",
            ));
        }
        let request = StripeRequest::get(scope, format!("/v1/accounts/{stripe_id}"), json!({}));
        let gateway = super::gateway_for_version(state, &request.api_version).await?;
        gateway
            .execute(&request)
            .await
            .map_err(stripe_error)?
            .decode::<Account>()
            .map_err(stripe_error)
    }
    .await;
    let remote = match fetch {
        Ok(remote) => remote,
        Err(failure) => {
            state.db.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "syncLeaseUntil"=NULL,"syncNextAt"=$3 WHERE id=$1 AND "syncRevision"=$2"#,vec![id.into(),rev.into(),(now+operations::backoff_ms(attempts)).into()])).await?;
            return Err(failure);
        }
    };
    if remote.id != stripe_id {
        return Err(ApiError::internal(
            "Stripe returned a different account identity",
        ));
    }
    let (derived, mut can_accept, mut can_sell) = derive_state(&remote);
    let country_allowed = remote
        .country
        .as_ref()
        .is_some_and(|country| state.platform_config.payments.countries.contains(country));
    can_accept &= country_allowed;
    can_sell &= country_allowed;
    let binding_id = binding_id(&local.user_id, local.livemode);
    let id = id.to_owned();
    let business = remote.business_profile.clone();
    let requirements = serde_json::to_value(&remote.requirements)?;
    let capabilities = serde_json::to_value(&remote.capabilities)?;
    state.transaction(|txn| {
        let id=id.clone();let binding_id=binding_id.clone();let business=business.clone();let requirements=requirements.clone();let capabilities=capabilities.clone();let currency=remote.default_currency.clone();let reason=remote.requirements.disabled_reason.clone();
        Box::pin(async move {
            let won=txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET state=$2,"canAcceptPayments"=$3,"canSell"=$4,"chargesEnabled"=$5,"payoutsEnabled"=$6,"defaultCurrency"=$7,business=$8,requirements=$9,capabilities=$10,"disabledReason"=$11,"syncedAt"=$12,"syncLeaseUntil"=NULL,"syncNextAt"=$12+10000,"syncAttempts"=0,revision=revision+1,"updatedAt"=$12 WHERE id=$1 AND "syncRevision"=$13 AND "retiredAt" IS NULL"#,
                vec![id.clone().into(),derived.into(),can_accept.into(),can_sell.into(),remote.charges_enabled.into(),remote.payouts_enabled.into(),currency.into(),business.into(),requirements.into(),capabilities.into(),reason.into(),now.into(),rev.into()])).await?.rows_affected()==1;
            if won && can_accept {
                txn.execute_raw(sql(r#"UPDATE "PaymentAccountBinding" SET "activeAccountId"=$2,"candidateAccountId"=NULL,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND "activeAccountId" IS NULL AND "candidateAccountId"=$2"#,vec![binding_id.into(),id.into(),now.into()])).await?;
            }
            Ok::<_,ApiError>(())
        })
    }).await
}

fn binding_id(user_id: &str, live: bool) -> String {
    blake3::hash(format!("{user_id}:{live}:shared").as_bytes())
        .to_hex()
        .to_string()
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingInput {
    pub country: Option<String>,
    pub terms_version: String,
    pub locale: Option<String>,
}
#[derive(Deserialize)]
pub struct RefreshQuery {
    pub refresh: Option<bool>,
}

#[utoipa::path(get,path="/user/payments/connect",tag="payments",security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=ConnectView),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn get_account(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<RefreshQuery>,
) -> Result<Json<ConnectView>, ApiError> {
    let user_id = session_user(&user)?;
    if is_platform_admin(&state.db, user_id).await? {
        return platform_view(&state, user_id).await.map(Json);
    }
    let account = account_for_user(&state, user_id).await?;
    if query.refresh.unwrap_or(false)
        && let Some(account) = &account
        && account
            .synced_at
            .is_none_or(|time| Utc::now().timestamp_millis() - time > 10_000)
    {
        sync_account(&state, &account.id).await?;
    }
    Ok(Json(account_for_user(&state, user_id).await?.into()))
}

#[utoipa::path(get,path="/user/payments/connect/countries",tag="payments",security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn countries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Value>, ApiError> {
    session_user(&user)?;
    Ok(Json(
        json!({"countries":state.platform_config.payments.countries}),
    ))
}

#[utoipa::path(post,path="/user/payments/connect/onboarding",tag="payments",request_body=OnboardingInput,security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn onboarding(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<OnboardingInput>,
) -> Result<Json<Value>, ApiError> {
    let user_id = recent_session(&state, &user).await?;
    require_connected_owner(&state.db, &user_id).await?;
    let config = &state.platform_config.payments;
    if !config.onboarding_enabled {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Payment onboarding is not enabled",
        ));
    }
    config
        .validate()
        .map_err(|message| error("PAYMENTS_DISABLED", &message))?;
    require_seller_allowed(&state.db, config, &user_id).await?;
    let locale = body.locale.as_deref().unwrap_or("en");
    let text = config
        .legal_texts
        .iter()
        .find(|text| {
            text.kind == "PAYMENTS_OWNER_TERMS"
                && text.version == body.terms_version
                && text.locale == locale
        })
        .ok_or_else(|| {
            error(
                "PAYMENT_TERMS_REQUIRED",
                "The current owner agreement is unavailable",
            )
        })?;
    let content = text
        .content()
        .map_err(|message| error("PAYMENT_TERMS_REQUIRED", &message))?;
    if config.owner_terms_version.as_deref() != Some(&body.terms_version) {
        return Err(error(
            "PAYMENT_TERMS_REQUIRED",
            "Accept the current owner agreement",
        ));
    }
    let scope = connect_scope(&state).await?;
    let country = body
        .country
        .clone()
        .unwrap_or_else(|| config.countries[0].clone());
    if !config.countries.contains(&country) {
        return Err(error(
            "PAYMENT_COUNTRY_UNSUPPORTED",
            "This country is not enabled for payments",
        ));
    }
    let binding_id = binding_id(&user_id, scope.livemode);
    let candidate = flow_like_types::create_id();
    let consent = flow_like_types::create_id();
    let now = Utc::now().timestamp_millis();
    let text_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    let account_id=state.transaction(|txn| {
        let binding_id=binding_id.clone();let user_id=user_id.clone();let candidate=candidate.clone();let consent=consent.clone();let country=country.clone();let scope=scope.clone();let version=body.terms_version.clone();let locale=locale.to_owned();let text_hash=text_hash.clone();
        Box::pin(async move {
            crate::db::coordination::coordinate(txn,"payments-owner",&[&user_id]).await?;
            require_connected_owner(txn, &user_id).await?;
            txn.execute_raw(sql(r#"INSERT INTO "PaymentAccountBinding" (id,"userId",livemode,purpose,"createdAt","updatedAt") VALUES ($1,$2,$3,'shared',$4,$4) ON CONFLICT DO NOTHING"#,vec![binding_id.clone().into(),user_id.clone().into(),scope.livemode.into(),now.into()])).await?;
            let binding=payment_account_binding::Entity::find_by_id(&binding_id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
            if let Some(id)=binding.active_account_id.or(binding.candidate_account_id) {
                let claimed=txn.execute_raw(sql(r#"UPDATE "PaymentAccountBinding" SET revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND revision=$3"#,vec![binding_id.into(),now.into(),binding.revision.into()])).await?;
                if claimed.rows_affected()!=1 {return Err(error("PAYMENT_OPERATION_PENDING","The payment account changed; refresh its status"));}
                let updated=txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "consentId"=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND "userId"=$4 AND "platformAccountId"=$5 AND livemode=$6 AND "retiredAt" IS NULL"#,vec![id.clone().into(),consent.clone().into(),now.into(),user_id.clone().into(),scope.platform_account_id.into(),scope.livemode.into()])).await?;
                if updated.rows_affected()!=1 {return Err(error("CONNECT_ONBOARDING_REQUIRED","Reconnect your payment account before accepting updated terms"));}
                txn.execute_raw(sql(r#"INSERT INTO "LegalConsent" (id,"userId",kind,"subjectType","subjectId","textVersion","textHash",locale,accepted,"createdAt") VALUES ($1,$2,'PAYMENTS_OWNER_TERMS','CONNECTED_ACCOUNT',$3,$4,$5,$6,TRUE,$7)"#,vec![consent.clone().into(),user_id.clone().into(),id.clone().into(),version.clone().into(),text_hash.into(),locale.into(),now.into()])).await?;
                outbox::enqueue(txn,&format!("consent:{consent}"),"PAYMENT_AUDIT","CONNECTED_ACCOUNT",&id,json!({"action":"payments.terms.accepted","actor":user_id,"version":version})).await?;
                return Ok(id);
            }
            let claimed=txn.execute_raw(sql(r#"UPDATE "PaymentAccountBinding" SET "candidateAccountId"=$2,generation=generation+1,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND revision=$4 AND "activeAccountId" IS NULL AND "candidateAccountId" IS NULL"#,vec![binding_id.into(),candidate.clone().into(),now.into(),binding.revision.into()])).await?;
            if claimed.rows_affected()!=1 {return Err(error("PAYMENT_OPERATION_PENDING","Another onboarding request is in progress"));}
            txn.execute_raw(sql(r#"INSERT INTO "LegalConsent" (id,"userId",kind,"subjectType","subjectId","textVersion","textHash",locale,accepted,"createdAt") VALUES ($1,$2,'PAYMENTS_OWNER_TERMS','CONNECTED_ACCOUNT',$3,$4,$5,$6,TRUE,$7)"#,vec![consent.clone().into(),user_id.clone().into(),candidate.clone().into(),version.into(),text_hash.into(),locale.into(),now.into()])).await?;
            let params=json!({"country":country,"controller":{"fees":{"payer":"account"},"losses":{"payments":"stripe"},"requirement_collection":"stripe","stripe_dashboard":{"type":"full"}},"capabilities":{"card_payments":{"requested":true},"transfers":{"requested":true}},"metadata":{"flowlike_row":candidate}});
            txn.execute_raw(sql(r#"INSERT INTO "ConnectedAccount" (id,"userId","platformAccountId",livemode,purpose,generation,"creationParams",country,"consentId","createdAt","updatedAt") VALUES ($1,$2,$3,$4,'shared',$5,$6,$7,$8,$9,$9)"#,vec![candidate.clone().into(),user_id.clone().into(),scope.platform_account_id.clone().into(),scope.livemode.into(),(binding.generation+1).into(),params.clone().into(),country.into(),consent.into(),now.into()])).await?;
            let request=StripeRequest::post(scope,"/v1/accounts",params,format!("ca:{candidate}:create"));
            operations::prepare(txn,"CONNECTED_ACCOUNT",&candidate,"CREATE_ACCOUNT",&request).await?;
            outbox::enqueue(txn,&format!("account:{candidate}:created"),"PAYMENT_AUDIT","CONNECTED_ACCOUNT",&candidate,json!({"action":"payments.account.onboarding","actor":user_id})).await?;
            Ok::<_,ApiError>(candidate)
        })
    }).await?;
    finish_account_creation(&state, &account_id).await?;
    mint_link(&state, &account_id).await.map(Json)
}

pub async fn finish_account_creation(state: &AppState, id: &str) -> Result<(), ApiError> {
    let account = connected_account::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if account.stripe_account_id.is_some() {
        return Ok(());
    }
    let operations=state.db.query_all_raw(sql(r#"SELECT id FROM "StripeOperation" WHERE "sourceType"='CONNECTED_ACCOUNT' AND "sourceId"=$1 AND operation='CREATE_ACCOUNT' LIMIT 2"#,vec![id.into()])).await?;
    if operations.len() != 1 {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "The original account creation command needs review",
        ));
    }
    let operation_id: String = operations[0].try_get("", "id")?;
    // The operation guard blocks new setup for admins or deleted users while allowing
    // an already completed provider response to restore its retained account identity.
    let response = operations::execute_prepared(state, &operation_id).await?;
    let remote: Account = response.decode().map_err(stripe_error)?;
    state.db.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "stripeAccountId"=$2,state=CASE WHEN "retiredAt" IS NULL THEN 'onboarding_incomplete' ELSE state END,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND "stripeAccountId" IS NULL"#,vec![id.into(),remote.id.into(),Utc::now().timestamp_millis().into()])).await?;
    sync_account(state, id).await
}

async fn mint_link(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let account = connected_account::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    require_connected_owner(&state.db, &account.user_id).await?;
    if account.retired_at.is_some() {
        return Err(error(
            "CONNECT_ONBOARDING_REQUIRED",
            "Reconnect this account before continuing setup",
        ));
    }
    let stripe_id = account.stripe_account_id.ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_PENDING",
            "Account creation is still in progress",
        )
    })?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let nonce_hash = blake3::hash(nonce.as_bytes()).to_hex().to_string();
    let now = Utc::now().timestamp_millis();
    state.db.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "resumeNonceHash"=$2,"resumeExpiresAt"=$3,"resumeUsedAt"=NULL,"updatedAt"=$4 WHERE id=$1 AND "retiredAt" IS NULL"#,vec![id.into(),nonce_hash.into(),(now+1_800_000).into(),now.into()])).await?;
    let frontend = state
        .platform_config
        .payments
        .frontend_url
        .as_deref()
        .ok_or_else(|| {
            error(
                "PAYMENTS_DISABLED",
                "The payment frontend is not configured",
            )
        })?
        .trim_end_matches('/');
    let params = json!({"account":stripe_id,"type":"account_onboarding","collection_options":{"fields":"eventually_due"},"return_url":format!("{frontend}/payments/return?connect=return"),"refresh_url":format!("{frontend}/payments/return?connect=refresh&r={nonce}")});
    let request = StripeRequest::post(
        connect_scope(state).await?,
        "/v1/account_links",
        params,
        format!("ca:{id}:link:{}", flow_like_types::create_id()),
    );
    let result =
        operations::execute(state, "CONNECTED_ACCOUNT", id, "ACCOUNT_LINK", request).await?;
    let link: crate::stripe_connect::types::AccountLink = result.decode().map_err(stripe_error)?;
    require_connected_owner(&state.db, &account.user_id).await?;
    Ok(json!({"url":link.url,"expiresAt":link.expires_at*1000}))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResumeInput {
    pub resume_token: String,
}

#[utoipa::path(post,path="/user/payments/connect/resume",tag="payments",request_body=ResumeInput,responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn resume(
    State(state): State<AppState>,
    Json(body): Json<ResumeInput>,
) -> Result<Json<Value>, ApiError> {
    if !state.platform_config.payments.onboarding_enabled || body.resume_token.len() != 32 {
        return Err(ApiError::NOT_FOUND);
    }
    let hash = blake3::hash(body.resume_token.as_bytes())
        .to_hex()
        .to_string();
    let now = Utc::now().timestamp_millis();
    let id=state.transaction(|txn| {let hash=hash.clone();Box::pin(async move {
        let row=txn.query_one_raw(sql(r#"UPDATE "ConnectedAccount" SET "resumeUsedAt"=$2 WHERE "resumeNonceHash"=$1 AND "resumeUsedAt" IS NULL AND "resumeExpiresAt">$2 AND "retiredAt" IS NULL AND state IN ('onboarding_incomplete','action_required','enabled_action_needed') RETURNING id,"userId""#,vec![hash.into(),now.into()])).await?.ok_or(ApiError::NOT_FOUND)?;
        let id:String=row.try_get("","id")?;
        let owner:String=row.try_get("","userId")?;
        crate::db::coordination::coordinate(txn,"payments-owner",&[&owner]).await?;
        require_connected_owner(txn,&owner).await?;
        let window=now/3_600_000;
        let key=format!("onboarding-resume:{id}:{window}");
        txn.execute_raw(sql(r#"INSERT INTO "PaymentLimitCounter" (key,"windowEnd","createdAt","updatedAt") VALUES ($1,$2,$3,$3) ON CONFLICT DO NOTHING"#,vec![key.clone().into(),((window+1)*3_600_000).into(),now.into()])).await?;
        let reserved=txn.execute_raw(sql(r#"UPDATE "PaymentLimitCounter" SET count=count+1,revision=revision+1,"updatedAt"=$2 WHERE key=$1 AND count<10"#,vec![key.into(),now.into()])).await?;
        if reserved.rows_affected()!=1 {return Err(ApiError::coded(axum::http::StatusCode::TOO_MANY_REQUESTS,"PAYMENT_RATE_LIMITED","Too many setup links requested; sign in or try again later"));}
        Ok::<_,ApiError>(id)
    })}).await?;
    mint_link(&state, &id).await.map(Json)
}

#[utoipa::path(post,path="/user/payments/connect/disconnect",tag="payments",security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn disconnect(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Value>, ApiError> {
    let user_id = recent_session(&state, &user).await?;
    let account = account_for_user(&state, &user_id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let now = Utc::now().timestamp_millis();
    state.transaction(|txn| {let account=account.clone();Box::pin(async move {
        txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "retiredAt"=$2,"retiredReason"='USER_DISCONNECT',state='disconnected',"canAcceptPayments"=FALSE,"canSell"=FALSE,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "retiredAt" IS NULL"#,vec![account.id.clone().into(),now.into()])).await?;
        txn.execute_raw(sql(r#"UPDATE "PaymentAccountBinding" SET revision=revision+1,"updatedAt"=$2 WHERE "activeAccountId"=$1 OR "candidateAccountId"=$1"#,vec![account.id.clone().into(),now.into()])).await?;
        outbox::enqueue(txn,&format!("account:{}:retire:{}",account.id,account.revision),"CANCEL_ACCOUNT_PAYMENTS","CONNECTED_ACCOUNT",&account.id,json!({"accountId":account.stripe_account_id})).await?;
        outbox::enqueue(txn,&format!("account:{}:disconnect:{}",account.id,account.revision),"PAYMENT_AUDIT","CONNECTED_ACCOUNT",&account.id,json!({"action":"payments.account.disconnected","actor":account.user_id})).await?;
        Ok::<_,ApiError>(())
    })}).await?;
    Ok(Json(
        json!({"disconnected":true,"stripeAccountRemainsOpen":true}),
    ))
}

#[utoipa::path(get,path="/apps/{app_id}/payments/readiness",tag="payments",params(("app_id"=String,Path)),security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn readiness(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    user.app_permission(&app_id, &state).await?;
    let owner = payee_for_app(&state.db, &app_id).await?;
    let viewer = user.sub()?;
    let account = account_for_user(&state, &owner).await?;
    let settings=state.db.query_one_raw(sql(r#"SELECT "paymentsEnabled","ownerUserId","adminBlockedAt" FROM "AppPaymentSettings" WHERE "appId"=$1"#,vec![app_id.into()])).await?;
    let enabled = settings.as_ref().is_some_and(|row| {
        row.try_get::<bool>("", "paymentsEnabled").unwrap_or(false)
            && row
                .try_get::<String>("", "ownerUserId")
                .is_ok_and(|id| id == owner)
            && row
                .try_get::<Option<i64>>("", "adminBlockedAt")
                .ok()
                .flatten()
                .is_none()
    });
    let view: ConnectView = if is_platform_admin(&state.db, &owner).await? {
        platform_view(&state, &owner).await?
    } else {
        account.into()
    };
    Ok(Json(
        json!({"platformOwned":view.platform_owned,"payee":{"isYou":owner==viewer,"platformOwned":view.platform_owned},"paymentsEnabled":enabled,"canAcceptPayments":view.can_accept_payments,"canSell":view.can_sell,"state":view.state}),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInput {
    pub payments_enabled: bool,
    pub refunds_from_flows: bool,
    pub max_payment_amount: i64,
    pub refunds_daily_cap: i64,
}

#[utoipa::path(get,path="/apps/{app_id}/payments/settings",tag="payments",params(("app_id"=String,Path)),security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn get_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let user_id = session_user(&user)?;
    ensure_app_owner(&state.db, &app_id, user_id).await?;
    let row = crate::entity::app_payment_settings::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?;
    let current = row.as_ref().filter(|row| row.owner_user_id == user_id);
    Ok(Json(json!({
        "paymentsEnabled":current.is_some_and(|row| row.payments_enabled),
        "refundsFromFlows":current.is_some_and(|row| row.refunds_from_flows),
        "maxPaymentAmount":current.and_then(|row|row.max_payment_amount.as_ref()).and_then(|value|value.get("eur")).and_then(Value::as_i64).unwrap_or(0),
        "refundsDailyCap":current.and_then(|row|row.refunds_daily_cap.as_ref()).and_then(|value|value.get("eur")).and_then(Value::as_i64).unwrap_or(0),
        "currency":"eur",
        "platformMaxPaymentAmount":state.platform_config.payments.max_payment_amount,
        "adminBlocked":row.is_some_and(|row|row.admin_blocked_at.is_some()),
    })))
}

#[utoipa::path(patch,path="/apps/{app_id}/payments/settings",tag="payments",request_body=SettingsInput,params(("app_id"=String,Path)),security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn update_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<SettingsInput>,
) -> Result<Json<Value>, ApiError> {
    let user_id = recent_session(&state, &user).await?;
    ensure_app_owner(&state.db, &app_id, &user_id).await?;
    let config = &state.platform_config.payments;
    if body.max_payment_amount < 0
        || body.max_payment_amount > config.max_payment_amount
        || body.refunds_daily_cap < 0
        || body.refunds_daily_cap > config.new_seller_daily_cap
        || body.payments_enabled && body.max_payment_amount < 50
        || body.refunds_from_flows && body.refunds_daily_cap < 50
    {
        return Err(error(
            "PAYMENT_AMOUNT_INVALID",
            "Choose payment and refund limits within the platform limits",
        ));
    }
    if body.payments_enabled {
        if !config.node_payments_enabled {
            return Err(error("PAYMENTS_DISABLED", "Flow payments are not enabled"));
        }
        require_can_accept(&state, &user_id).await?;
    }
    let now = Utc::now().timestamp_millis();
    state.transaction(|txn| {let app_id=app_id.clone();let user_id=user_id.clone();Box::pin(async move {
        crate::db::coordination::coordinate(txn,"payments-app",&[&app_id]).await?;
        ensure_app_owner(txn,&app_id,&user_id).await?;
        let blocked=txn.query_one_raw(sql(r#"SELECT "adminBlockedAt" FROM "AppPaymentSettings" WHERE "appId"=$1"#,vec![app_id.clone().into()])).await?;
        if body.payments_enabled && blocked.is_some_and(|row|row.try_get::<Option<i64>>("","adminBlockedAt").ok().flatten().is_some()) {return Err(error("PAYMENTS_DISABLED","Payments are paused for this app"));}
        txn.execute_raw(sql(r#"INSERT INTO "AppPaymentSettings" ("appId","ownerUserId","paymentsEnabled","refundsFromFlows","maxPaymentAmount","refundsDailyCap","updatedBy","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$2,$7,$7) ON CONFLICT ("appId") DO UPDATE SET "ownerUserId"=EXCLUDED."ownerUserId","paymentsEnabled"=EXCLUDED."paymentsEnabled","refundsFromFlows"=EXCLUDED."refundsFromFlows","maxPaymentAmount"=EXCLUDED."maxPaymentAmount","refundsDailyCap"=EXCLUDED."refundsDailyCap","updatedBy"=EXCLUDED."updatedBy",revision="AppPaymentSettings".revision+1,"updatedAt"=EXCLUDED."updatedAt"#,
            vec![app_id.clone().into(),user_id.clone().into(),body.payments_enabled.into(),body.refunds_from_flows.into(),json!({"eur":body.max_payment_amount}).into(),json!({"eur":body.refunds_daily_cap}).into(),now.into()])).await?;
        if !body.payments_enabled {outbox::enqueue(txn,&format!("app:{app_id}:disable:{now}"),"CANCEL_APP_PAYMENTS","APP",&app_id,json!({"actor":user_id})).await?;}
        outbox::enqueue(txn,&format!("app:{app_id}:settings:{now}"),"PAYMENT_AUDIT","APP",&app_id,json!({"action":"payments.settings.updated","actor":user_id,"enabled":body.payments_enabled,"maxAmount":body.max_payment_amount,"refunds":body.refunds_from_flows,"refundCap":body.refunds_daily_cap})).await?;
        Ok::<_,ApiError>(())
    })}).await?;
    get_settings(State(state), Extension(user), Path(app_id)).await
}

pub async fn owner_transferred<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    new_owner: &str,
    actor: &str,
) -> Result<(), ApiError> {
    let now = Utc::now().timestamp_millis();
    db.execute_raw(sql(r#"UPDATE "AppPaymentSettings" SET "ownerUserId"=$2,"paymentsEnabled"=FALSE,"refundsFromFlows"=FALSE,revision=revision+1,"updatedBy"=$3,"updatedAt"=$4 WHERE "appId"=$1"#,vec![app_id.into(),new_owner.into(),actor.into(),now.into()])).await?;
    outbox::enqueue(
        db,
        &format!("app:{app_id}:owner:{new_owner}:{now}"),
        "CANCEL_APP_PAYMENTS",
        "APP",
        app_id,
        json!({"reason":"OWNER_CHANGED"}),
    )
    .await
}

#[utoipa::path(post,path="/user/payments/connect/reconnect",tag="payments",security(("bearer_auth"=[])),responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn reconnect(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Value>, ApiError> {
    let user_id = recent_session(&state, &user).await?;
    require_connected_owner(&state.db, &user_id).await?;
    let config = &state.platform_config.payments;
    if !config.onboarding_enabled {
        return Err(error(
            "PAYMENTS_DISABLED",
            "Payment onboarding is not enabled",
        ));
    }
    require_seller_allowed(&state.db, config, &user_id).await?;
    let account = account_for_user(&state, &user_id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if account.retired_reason.as_deref() != Some("USER_DISCONNECT") {
        return Err(error(
            "CONNECT_ONBOARDING_REQUIRED",
            "This account requires a new onboarding review",
        ));
    }
    let stripe_id = account.stripe_account_id.as_ref().ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_PENDING",
            "Account creation is unresolved",
        )
    })?;
    let request = StripeRequest::get(
        connect_scope(&state).await?,
        format!("/v1/accounts/{stripe_id}"),
        json!({}),
    );
    let gateway = super::gateway_for_version(&state, &request.api_version).await?;
    let remote: Account = gateway
        .execute(&request)
        .await
        .map_err(stripe_error)?
        .decode()
        .map_err(stripe_error)?;
    if remote.id != *stripe_id {
        return Err(ApiError::internal(
            "Stripe returned a different account identity",
        ));
    }
    let now = Utc::now().timestamp_millis();
    state.transaction(|txn| {let account=account.clone();let user_id=user_id.clone();Box::pin(async move {
        crate::db::coordination::coordinate(txn,"payments-owner",&[&user_id]).await?;
        require_connected_owner(txn,&user_id).await?;
        let updated=txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "retiredAt"=NULL,"retiredReason"=NULL,state='onboarding_incomplete',"syncedAt"=NULL,"syncNextAt"=0,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND revision=$3 AND "retiredReason"='USER_DISCONNECT' AND EXISTS (SELECT 1 FROM "PaymentAccountBinding" b WHERE b."activeAccountId"=$1 OR b."candidateAccountId"=$1)"#,vec![account.id.clone().into(),now.into(),account.revision.into()])).await?;
        if updated.rows_affected()!=1 {return Err(error("PAYMENT_OPERATION_PENDING","The account changed; refresh its status"));}
        txn.execute_raw(sql(r#"UPDATE "PaymentAccountBinding" SET revision=revision+1,"updatedAt"=$2 WHERE "activeAccountId"=$1 OR "candidateAccountId"=$1"#,vec![account.id.clone().into(),now.into()])).await?;
        outbox::enqueue(txn,&format!("account:{}:reconnect:{}",account.id,account.revision),"PAYMENT_AUDIT","CONNECTED_ACCOUNT",&account.id,json!({"action":"payments.account.reconnected","actor":user_id})).await?;
        Ok::<_,ApiError>(())
    })}).await?;
    sync_account(&state, &account.id).await?;
    Ok(Json(json!({"reconnected":true})))
}

#[derive(Deserialize)]
pub struct TermsQuery {
    pub kind: String,
    pub locale: Option<String>,
    pub version: Option<String>,
}

#[utoipa::path(get,path="/payments/terms",tag="payments",responses((status=200,description="Payment account response",body=Object),(status=409,description="Payment is unavailable or awaiting reconciliation")))]
pub async fn terms(
    State(state): State<AppState>,
    Query(query): Query<TermsQuery>,
) -> Result<Json<Value>, ApiError> {
    let config = &state.platform_config.payments;
    let version = query
        .version
        .as_deref()
        .or(match query.kind.as_str() {
            "PAYMENTS_OWNER_TERMS" => config.owner_terms_version.as_deref(),
            "SELLER_TERMS" => config.seller_terms_version.as_deref(),
            "PURCHASE_TERMS" => config.purchase_terms_version.as_deref(),
            "PURCHASE_WAIVER" => config.purchase_waiver_version.as_deref(),
            _ => None,
        })
        .ok_or(ApiError::NOT_FOUND)?;
    let locale = query.locale.as_deref().unwrap_or("en");
    let text =
        select_legal_text(config, &query.kind, version, locale).ok_or(ApiError::NOT_FOUND)?;
    let content = text
        .content()
        .map_err(|message| error("PAYMENT_TERMS_REQUIRED", &message))?;
    Ok(Json(
        json!({"kind":text.kind,"version":text.version,"locale":text.locale,"text":content,"hash":blake3::hash(content.as_bytes()).to_hex().to_string(),"url":text.url}),
    ))
}

fn select_legal_text<'a>(
    config: &'a flow_like::hub::PaymentsConfig,
    kind: &str,
    version: &str,
    locale: &str,
) -> Option<&'a flow_like::hub::PaymentLegalText> {
    let language = locale.split(['-', '_']).next().unwrap_or(locale);
    // Return the actual locale so acceptance always records the text shown.
    [locale, language, "en"].into_iter().find_map(|locale| {
        config
            .legal_texts
            .iter()
            .find(|text| text.kind == kind && text.version == version && text.locale == locale)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account() -> Account {
        serde_json::from_value(json!({"id":"acct_seller","details_submitted":true,"charges_enabled":true,"payouts_enabled":false,"capabilities":{"card_payments":"active","transfers":"active"}})).unwrap()
    }
    #[test]
    fn legal_text_fallback_returns_the_actual_accepted_language_and_version() {
        use flow_like::hub::{PaymentLegalText, PaymentsConfig};
        let config = PaymentsConfig {
            legal_texts: vec![
                PaymentLegalText {
                    kind: "PURCHASE_TERMS".into(),
                    version: "v1".into(),
                    locale: "en".into(),
                    text: "English".into(),
                    ..Default::default()
                },
                PaymentLegalText {
                    kind: "PURCHASE_TERMS".into(),
                    version: "v1".into(),
                    locale: "de".into(),
                    text: "Deutsch".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            select_legal_text(&config, "PURCHASE_TERMS", "v1", "de-DE")
                .unwrap()
                .locale,
            "de"
        );
        assert_eq!(
            select_legal_text(&config, "PURCHASE_TERMS", "v1", "fr")
                .unwrap()
                .locale,
            "en"
        );
        assert!(select_legal_text(&config, "PURCHASE_TERMS", "v2", "fr").is_none());
        assert!(select_legal_text(&config, "SELLER_TERMS", "v1", "en").is_none());
    }

    #[test]
    fn website_terms_resolve_locale_fallback_without_hiding_invalid_references() {
        let source: Value =
            serde_json::from_str(include_str!("../../../../flow-like.config.json")).unwrap();
        let mut config: flow_like::hub::PaymentsConfig =
            serde_json::from_value(source["payments"].clone()).unwrap();
        let version = config.purchase_terms_version.clone().unwrap();
        for (requested, expected) in [("de-DE", "de"), ("fr", "en")] {
            let text = select_legal_text(&config, "PURCHASE_TERMS", &version, requested).unwrap();
            assert_eq!(text.locale, expected);
            assert!(text.text.is_empty());
            let content = text.content().unwrap();
            assert_eq!(
                text.hash.as_deref(),
                Some(blake3::hash(content.as_bytes()).to_hex().as_str())
            );
        }
        config
            .legal_texts
            .iter_mut()
            .find(|text| text.kind == "PURCHASE_TERMS" && text.locale == "de")
            .unwrap()
            .hash = Some("0".repeat(64));
        let selected = select_legal_text(&config, "PURCHASE_TERMS", &version, "de-DE").unwrap();
        assert_eq!(selected.locale, "de");
        assert!(selected.content().unwrap_err().contains("hash mismatch"));
    }

    #[test]
    fn readiness_does_not_confuse_payment_and_payout_availability() {
        let mut account = account();
        assert_eq!(derive_state(&account), ("enabled", true, true));
        account
            .capabilities
            .insert("transfers".into(), "pending".into());
        assert_eq!(derive_state(&account), ("enabled", true, false));
        account
            .requirements
            .currently_due
            .push("business_profile.url".into());
        assert_eq!(
            derive_state(&account),
            ("enabled_action_needed", true, false)
        );
    }
    #[test]
    fn readiness_fails_closed_for_unknown_and_rejected_restrictions() {
        let mut account = account();
        account.requirements.disabled_reason = Some("future.restriction".into());
        assert_eq!(derive_state(&account), ("limited", false, false));
        account.requirements.disabled_reason = Some("rejected.fraud".into());
        assert_eq!(derive_state(&account), ("rejected", false, false));
    }
}
