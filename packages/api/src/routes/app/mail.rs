use std::{collections::HashSet, time::Duration};

use crate::{
    cache::Reservation,
    error::ApiError,
    execution::state::{ExecutionRunRecord, RunVariant},
    mail::{AutoSubmitted, AutomationEmailMessage, DynMailClient},
    mail_ingress::{
        ReplySource, digest, failed_verdict, is_automated,
        limits::{self, Counter, Window},
        now, statement,
    },
    middleware::jwt::{AppUser, ExecutorUser},
    permission::role_permission::RolePermissions,
    routes::execution::progress::get_state_store,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderValue, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use flow_like_catalog_core::{MailMessageRef, MailSession};
use mail_parser::{Message, MessageParser};
use sea_orm::{ConnectionTrait, FromQueryResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_RECIPIENTS: usize = 20;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_DISPLAY_NAME: usize = 64;
/// Longer than any send route deadline, so a live claim is never taken over.
const SEND_LEASE_MS: i64 = 600_000;
const IDEMPOTENCY_WINDOW_MS: i64 = 86_400_000;

// OpenAPI mirrors the shared catalog locator types without adding an HTTP dependency to them.
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct MailSessionSchema {
    pub app_id: String,
    pub event_id: String,
}
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct MailMessageRefSchema {
    pub session: MailSessionSchema,
    pub delivery_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMailRequest {
    #[schema(value_type = MailSessionSchema)]
    pub session: MailSession,
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
    /// Retries with the same ID within 24 hours return the first response instead of
    /// sending again. 1-128 characters of letters, digits, `:`, `.`, `_` and `-`.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyMailRequest {
    #[schema(value_type = MailSessionSchema)]
    pub session: MailSession,
    #[schema(value_type = MailMessageRefSchema)]
    pub message: MailMessageRef,
    pub text: Option<String>,
    pub html: Option<String>,
    /// Retries with the same ID within 24 hours return the first response instead of
    /// replying again. 1-128 characters of letters, digits, `:`, `.`, `_` and `-`.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SendMailResponse {
    pub accepted: bool,
    pub request_id: String,
    #[schema(value_type = MailSessionSchema)]
    pub session: MailSession,
}

/// An API error that may tell the caller when a retry can succeed.
pub struct MailError {
    error: ApiError,
    retry_after: Option<u64>,
}

impl From<ApiError> for MailError {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            retry_after: None,
        }
    }
}

impl IntoResponse for MailError {
    fn into_response(self) -> Response {
        let mut response = self.error.into_response();
        if let Some(seconds) = self.retry_after {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

fn too_many(message: impl Into<String>, retry_after_secs: u64) -> MailError {
    MailError {
        error: ApiError::too_many_requests(message),
        retry_after: Some(retry_after_secs.max(1)),
    }
}

fn valid_address(address: &str) -> bool {
    if address.len() > 254 || !address.is_ascii() {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn validate_body(text: Option<&str>, html: Option<&str>) -> Result<(), ApiError> {
    let bytes = text.map_or(0, str::len) + html.map_or(0, str::len);
    if bytes == 0 || bytes > MAX_BODY_BYTES {
        return Err(ApiError::bad_request(
            "Provide a text or HTML body totaling at most 1 MiB",
        ));
    }
    Ok(())
}

fn validate_request_id(request_id: Option<&str>) -> Result<(), ApiError> {
    if request_id.is_some_and(|id| {
        !(1..=128).contains(&id.len())
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b))
    }) {
        return Err(ApiError::bad_request(
            "request_id must be 1-128 letters, digits, ':', '.', '_' or '-'",
        ));
    }
    Ok(())
}

fn validate(request: &SendMailRequest) -> Result<(), ApiError> {
    if request.to.is_empty()
        || request.to.len() + request.cc.len() + request.bcc.len() > MAX_RECIPIENTS
    {
        return Err(ApiError::bad_request(
            "Provide a To recipient and at most 20 total recipients",
        ));
    }
    if request
        .to
        .iter()
        .chain(&request.cc)
        .chain(&request.bcc)
        .any(|address| !valid_address(address))
    {
        return Err(ApiError::bad_request(
            "Recipients must be plain email addresses",
        ));
    }
    if request.subject.trim().is_empty()
        || request.subject.len() > 998
        || request.subject.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(
            "Subject must contain 1-998 bytes without control characters",
        ));
    }
    validate_request_id(request.request_id.as_deref())?;
    validate_body(request.text.as_deref(), request.html.as_deref())
}

/// Each address receives one copy; To wins over Cc, and Cc over Bcc.
fn dedupe_recipients(request: &mut SendMailRequest) {
    let mut seen = HashSet::new();
    for list in [&mut request.to, &mut request.cc, &mut request.bcc] {
        list.retain(|address| seen.insert(address.to_ascii_lowercase()));
    }
}

/// Event addresses only receive mail from outside, so flows cannot trigger each other.
fn ensure_external<'a>(
    domain: Option<&str>,
    recipients: impl IntoIterator<Item = &'a String>,
) -> Result<(), ApiError> {
    let Some(domain) = domain else {
        return Ok(());
    };
    let subdomain = format!(".{domain}");
    let internal = recipients.into_iter().find(|address| {
        address.rsplit_once('@').is_some_and(|(_, host)| {
            let host = host.to_ascii_lowercase();
            host == domain || host.ends_with(&subdomain)
        })
    });
    if let Some(address) = internal {
        return Err(ApiError::unprocessable(format!(
            "{address} is an event address on {domain}; flows cannot email email events"
        )));
    }
    Ok(())
}

/// The event name as display name, so recipients see who writes instead of the platform.
fn display_name(event_name: &str, from: &str) -> String {
    let cleaned = event_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.&+'".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();
    let name = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_DISPLAY_NAME)
        .collect::<String>();
    let name = name.trim();
    if name.is_empty() {
        from.split('@').next().unwrap_or(from).to_owned()
    } else {
        name.to_owned()
    }
}

fn run_can_send(run: &ExecutionRunRecord, session: &MailSession) -> bool {
    run.app_id == session.app_id
        && run.event_id.as_deref() == Some(session.event_id.as_str())
        && !run.status.is_terminal()
        && !matches!(run.run_variant, RunVariant::Shadow | RunVariant::Regression)
        && run.shadow_of_run_id.is_none()
        && run.regression_run_id.is_none()
        && run
            .expires_at
            .is_none_or(|expiry| expiry > chrono::Utc::now().timestamp_millis())
}

fn executor_can_send(
    executor: &ExecutorUser,
    run: &ExecutionRunRecord,
    session: &MailSession,
    sink_id: &str,
) -> bool {
    if executor.app_id != session.app_id || executor.run_id != run.id || !run_can_send(run, session)
    {
        return false;
    }
    if let Some(subject_sink_id) = executor.sub.strip_prefix("sink:") {
        return subject_sink_id == sink_id
            && executor.app_chain.is_none()
            && run.user_id.is_none()
            && run.technical_user_id.is_none()
            && executor.technical_user_id.is_none();
    }
    // User, API-key and connected-app dispatches retain their effective user and
    // optional technical user in both the run record and the signed token.
    run.user_id.as_deref() == Some(executor.sub.as_str())
        && executor.technical_user_id == run.technical_user_id
}

/// Who is sending: the event supplies the display name, the principal carries the daily cap.
struct Sender {
    event_name: String,
    principal: Option<String>,
}

/// A live run of the session's event; the run's user or the sink owner is the principal.
async fn authorize_executor(
    state: &AppState,
    executor: &ExecutorUser,
    app_id: &str,
    session: &MailSession,
) -> Result<Sender, ApiError> {
    if executor.app_id != app_id {
        return Err(ApiError::FORBIDDEN);
    }
    let run = get_state_store(state)
        .await?
        .get_run(&executor.run_id)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .filter(|run| run_can_send(run, session))
        .ok_or(ApiError::FORBIDDEN)?;
    let (sink, event) = crate::mail_ingress::live_target(state, app_id, &session.event_id)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if !executor_can_send(executor, &run, session, &sink.id) {
        return Err(ApiError::FORBIDDEN);
    }
    let principal = match run.user_id {
        Some(user_id) => Some(user_id),
        None => crate::mail_ingress::sink_owner(state, &sink).await?,
    };
    Ok(Sender {
        event_name: event.name,
        principal,
    })
}

async fn authorize(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    session: &MailSession,
) -> Result<Sender, ApiError> {
    if session.app_id != app_id
        || session.app_id.is_empty()
        || session.event_id.is_empty()
        || session.app_id.len() > 256
        || session.event_id.len() > 256
    {
        return Err(ApiError::FORBIDDEN);
    }
    if let AppUser::Executor(executor) = user {
        return authorize_executor(state, executor, app_id, session).await;
    }
    // Direct callers must currently be allowed to edit and execute this app's events.
    let permission = user.app_permission_fresh(app_id, state).await?;
    if !permission.has_permission(RolePermissions::ExecuteEvents)
        || !permission.has_permission(RolePermissions::WriteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }
    let (_, event) = crate::mail_ingress::live_target(state, app_id, &session.event_id)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    Ok(Sender {
        event_name: event.name,
        principal: Some(
            permission
                .effective_user_id
                .clone()
                .unwrap_or_else(|| permission.identifier()),
        ),
    })
}

fn ensure_enabled(state: &AppState) -> Result<(), ApiError> {
    if !state.mail_automation.can_send() {
        return Err(ApiError::service_unavailable(
            "Mail automation is disabled on this deployment",
        ));
    }
    Ok(())
}

fn require_executor(user: &AppUser) -> Result<(), ApiError> {
    if matches!(user, AppUser::Executor(_)) {
        Ok(())
    } else {
        Err(ApiError::FORBIDDEN)
    }
}

#[derive(FromQueryResult)]
struct SendRecord {
    status: String,
    response: Option<String>,
}

struct OwnedClaim {
    id: String,
    lease: String,
}

enum Claim {
    Owned(OwnedClaim),
    Replay(SendMailResponse),
}

/// Reserves `request_id` for this app. A completed send replays; a crashed one is taken over
/// once its lease ends.
async fn claim(state: &AppState, app_id: &str, request_id: &str) -> Result<Claim, ApiError> {
    let id = digest(&[app_id, request_id]);
    let lease = flow_like_types::create_id();
    let timestamp = now();
    let claimed = state.db.execute_raw(statement(
        r#"INSERT INTO "MailAutomationSend" ("id","appId","status","lease","leaseUntil","response","createdAt","expiresAt") VALUES ($1,$2,'sending',$3,$4,NULL,$5,$6) ON CONFLICT ("id") DO UPDATE SET "status"='sending',"lease"=$3,"leaseUntil"=$4,"response"=NULL,"createdAt"=$5,"expiresAt"=$6 WHERE ("MailAutomationSend"."status"='sending' AND "MailAutomationSend"."leaseUntil" <= $5) OR "MailAutomationSend"."expiresAt" <= $5"#,
        vec![
            id.clone().into(),
            app_id.into(),
            lease.clone().into(),
            (timestamp + SEND_LEASE_MS).into(),
            timestamp.into(),
            (timestamp + IDEMPOTENCY_WINDOW_MS).into(),
        ],
    )).await?;
    if claimed.rows_affected() == 1 {
        return Ok(Claim::Owned(OwnedClaim { id, lease }));
    }
    let record = SendRecord::find_by_statement(statement(
        r#"SELECT "status","response" FROM "MailAutomationSend" WHERE "id"=$1"#,
        vec![id.into()],
    ))
    .one(&state.db)
    .await?;
    match record {
        Some(SendRecord {
            status,
            response: Some(response),
        }) if status == "sent" => Ok(Claim::Replay(serde_json::from_str(&response)?)),
        _ => Err(ApiError::conflict(
            "A message with this request_id is still being sent; retry once it completes",
        )),
    }
}

async fn finish(state: &AppState, claim: &OwnedClaim, response: &SendMailResponse) {
    let result = match serde_json::to_string(response) {
        Ok(response) => state
            .db
            .execute_raw(statement(
                r#"UPDATE "MailAutomationSend" SET "status"='sent',"response"=$1,"lease"=NULL WHERE "id"=$2 AND "lease"=$3"#,
                vec![response.into(), claim.id.clone().into(), claim.lease.clone().into()],
            ))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        tracing::warn!(send_id = %claim.id, %error, "Mail was sent but its request_id could not be recorded; a retry may send again");
    }
}

async fn release(state: &AppState, claim: &OwnedClaim) {
    if let Err(error) = state
        .db
        .execute_raw(statement(
            r#"DELETE FROM "MailAutomationSend" WHERE "id"=$1 AND "lease"=$2"#,
            vec![claim.id.clone().into(), claim.lease.clone().into()],
        ))
        .await
    {
        tracing::warn!(send_id = %claim.id, %error, "A failed send kept its request_id until the lease ends");
    }
}

/// One message per interval and app; the reservation's expiry becomes `Retry-After`.
async fn reserve_interval(state: &AppState, app_id: &str) -> Result<(), MailError> {
    let interval = state.mail_automation.min_send_interval_seconds;
    let unavailable = |_| ApiError::service_unavailable("Mail limit storage is unavailable");
    let cache = state.cache.platform().await.map_err(unavailable)?;
    let until = serde_json::Value::from(now() + interval as i64 * 1000);
    match cache
        .try_insert(
            "mail-automation-rate",
            app_id,
            &until,
            Duration::from_secs(interval),
        )
        .await
        .map_err(unavailable)?
    {
        Reservation::Acquired => Ok(()),
        Reservation::Held(held) => {
            let wait = held.as_i64().map_or(interval, |until| {
                ((until - now()).max(1_000) as u64).div_ceil(1_000)
            });
            Err(too_many(
                format!("This app may send one message every {interval} seconds"),
                wait,
            ))
        }
    }
}

struct Outgoing {
    session: MailSession,
    sender: Sender,
    request_id: Option<String>,
    message: AutomationEmailMessage,
}

/// Every To, Cc and Bcc address counts against the app's and the sender's daily limits.
async fn reserve_recipients<'a>(
    state: &AppState,
    session: &'a MailSession,
    sender: &'a Sender,
    recipients: u32,
) -> Result<(Window, Vec<Counter<'a>>), MailError> {
    let config = &state.mail_automation;
    let window = Window::at(now());
    let mut counters = vec![Counter {
        scope: "app",
        subject: &session.app_id,
        limit: config.daily_app_recipient_limit,
    }];
    if let Some(principal) = sender.principal.as_deref() {
        counters.push(Counter {
            scope: "principal",
            subject: principal,
            limit: config.daily_principal_recipient_limit,
        });
    }
    let Some(index) = limits::consume(state, &window, &counters, recipients).await? else {
        return Ok((window, counters));
    };
    let owner = if counters[index].scope == "app" {
        "This app"
    } else {
        "This user"
    };
    Err(too_many(
        format!(
            "{owner} reached its limit of {} mail recipients per day",
            counters[index].limit
        ),
        window.retry_after_secs(now()),
    ))
}

async fn deliver(
    state: &AppState,
    client: &DynMailClient,
    session: &MailSession,
    sender: &Sender,
    message: AutomationEmailMessage,
) -> Result<SendMailResponse, MailError> {
    reserve_interval(state, &session.app_id).await?;
    let recipients = (message.to.len() + message.cc.len() + message.bcc.len()) as u32;
    let (window, counters) = reserve_recipients(state, session, sender, recipients).await?;
    let request_id = flow_like_types::create_id();
    if let Err(error) = client.send_automation(message).await {
        limits::refund(state, &window, &counters, recipients).await;
        tracing::error!(app_id=%session.app_id,event_id=%session.event_id,%request_id,%error,"Automation mail submission failed");
        return Err(ApiError::service_unavailable(
            "The mail provider could not confirm acceptance",
        )
        .into());
    }
    tracing::info!(app_id=%session.app_id,event_id=%session.event_id,%request_id,recipients,"Automation mail accepted");
    Ok(SendMailResponse {
        accepted: true,
        request_id,
        session: session.clone(),
    })
}

async fn submit(state: &AppState, outgoing: Outgoing) -> Result<Json<SendMailResponse>, MailError> {
    let client = state
        .automation_mail_client
        .clone()
        .ok_or_else(|| ApiError::service_unavailable("No mail provider is configured"))?;
    // A repeated request_id is answered before any rate limit or quota is consumed.
    let claim = match outgoing.request_id.as_deref() {
        Some(request_id) => match claim(state, &outgoing.session.app_id, request_id).await? {
            Claim::Replay(response) => return Ok(Json(response)),
            Claim::Owned(owned) => Some(owned),
        },
        None => None,
    };
    let result = deliver(
        state,
        &client,
        &outgoing.session,
        &outgoing.sender,
        outgoing.message,
    )
    .await;
    if let Some(claim) = &claim {
        match &result {
            Ok(response) => finish(state, claim, response).await,
            Err(_) => release(state, claim).await,
        }
    }
    result.map(Json)
}

async fn send(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    mut request: SendMailRequest,
) -> Result<Json<SendMailResponse>, MailError> {
    ensure_enabled(state)?;
    let sender = authorize(state, user, app_id, &request.session).await?;
    validate(&request)?;
    dedupe_recipients(&mut request);
    ensure_external(
        state.mail_automation.domain(),
        request.to.iter().chain(&request.cc).chain(&request.bcc),
    )?;
    let from = crate::mail_ingress::address::sending_address(state, &request.session, None).await?;
    let message = AutomationEmailMessage {
        from_name: Some(display_name(&sender.event_name, &from)),
        from_email: Some(from.clone()),
        to: request.to,
        cc: request.cc,
        bcc: request.bcc,
        reply_to: Some(from),
        subject: request.subject,
        body_html: request.html,
        body_text: request.text,
        in_reply_to: None,
        references: Vec::new(),
        auto_submitted: Some(AutoSubmitted::Generated),
    };
    submit(
        state,
        Outgoing {
            session: request.session,
            sender,
            request_id: request.request_id,
            message,
        },
    )
    .await
}

#[utoipa::path(
    post, path = "/apps/{app_id}/mail/send", tag = "app",
    description = "Send from an active inbound email event. The session identifies the event; the server resolves its From address and shows the event name as sender. Recipients are deduplicated and count against the app's and the sender's daily recipient limits. Direct callers need current ExecuteEvents and WriteEvents permissions; executor callers need a live run of that event.",
    params(("app_id" = String, Path)), request_body = SendMailRequest,
    responses((status=200,description="Mail provider accepted the message, or the request_id was already sent",body=SendMailResponse),
        (status=400,description="Invalid message"),(status=403,description="Session not authorized"),
        (status=409,description="A message with this request_id is still being sent"),
        (status=422,description="A recipient is an event address on this server"),
        (status=429,description="App send interval or daily recipient limit reached",headers(("Retry-After" = u64, description = "Seconds until a retry can succeed"))),
        (status=503,description="Mail unavailable")),
    security(("bearer_auth"=[]),("pat"=[]),("executor_jwt"=[]))
)]
pub async fn send_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SendMailRequest>,
) -> Result<Json<SendMailResponse>, MailError> {
    send(&state, &user, &app_id, request).await
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/mail/send", tag = "execution",
    description = "Send from the inbound email event of this run. Executor only; same contract as the app route. Retry only 429 responses, after Retry-After.",
    params(("app_id" = String, Path, description = "Application ID")), request_body = SendMailRequest,
    responses((status=200,description="Mail provider accepted the message, or the request_id was already sent",body=SendMailResponse),
        (status=400,description="Invalid message"),(status=403,description="The caller is not a live run of this event"),
        (status=409,description="A message with this request_id is still being sent"),
        (status=422,description="A recipient is an event address on this server"),
        (status=429,description="App send interval or daily recipient limit reached",headers(("Retry-After" = u64, description = "Seconds until a retry can succeed"))),
        (status=503,description="Mail unavailable")),
    security(("executor_jwt"=[]))
)]
pub async fn executor_send_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<SendMailRequest>,
) -> Result<Json<SendMailResponse>, MailError> {
    require_executor(&user)?;
    send(&state, &user, &app_id, request).await
}

struct ReplyHeaders {
    to: String,
    subject: String,
    in_reply_to: Option<String>,
    references: Vec<String>,
}

fn message_id(raw: &str) -> Option<String> {
    let id = raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(raw);
    let (local, domain) = id.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || id.len() > 900
        || !id
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"<>\\\"(),;".contains(&b))
    {
        return None;
    }
    Some(format!("<{id}>"))
}

fn parse_original(raw: &[u8]) -> Result<Message<'_>, ApiError> {
    MessageParser::default()
        .parse(raw)
        .ok_or_else(|| ApiError::unprocessable("Cannot parse the original email"))
}

/// Exactly one recipient: the first Reply-To address, else the first From address.
fn reply_recipient(message: &Message<'_>) -> Result<String, ApiError> {
    let first = |addresses: Option<&mail_parser::Address<'_>>| {
        addresses
            .and_then(|addresses| addresses.first())
            .and_then(|address| address.address())
            .map(str::to_owned)
    };
    first(message.reply_to())
        .or_else(|| first(message.from()))
        .filter(|address| valid_address(address))
        .ok_or_else(|| {
            ApiError::unprocessable("The original email has no valid Reply-To or From address")
        })
}

/// Replies never go to automated mail or mail that failed a provider verdict.
fn refuse_unsafe_reply(source: &ReplySource, message: &Message<'_>) -> Result<(), ApiError> {
    if is_automated(&source.envelope_from, message) {
        return Err(ApiError::unprocessable(
            "The original email was sent automatically (bounce, auto-reply or mailing list); replying could start a mail loop",
        ));
    }
    if let Some(verdict) = failed_verdict(source.authentication.as_ref()) {
        return Err(ApiError::unprocessable(format!(
            "The original email failed its {verdict} check; replies to it are refused"
        )));
    }
    Ok(())
}

fn reply_headers(message: &Message<'_>) -> Result<ReplyHeaders, ApiError> {
    let to = reply_recipient(message)?;
    let original = message.subject().unwrap_or("(no subject)");
    let mut subject = original
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    let mut end = subject.len().min(900);
    while !subject.is_char_boundary(end) {
        end -= 1;
    }
    subject.truncate(end);
    if !subject
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
    {
        subject = format!("Re: {subject}");
    }
    let in_reply_to = message.message_id().and_then(message_id);
    // Keep the newest ancestors and the parent within every provider's header limit.
    let mut references = in_reply_to.iter().cloned().collect::<Vec<_>>();
    let mut reference_bytes = in_reply_to.as_ref().map_or(0, String::len);
    for ancestor in message
        .references()
        .as_text_list()
        .into_iter()
        .flatten()
        .rev()
    {
        let Some(id) = message_id(ancestor) else {
            continue;
        };
        if in_reply_to.as_ref() == Some(&id) {
            continue;
        }
        let bytes = id.len() + usize::from(!references.is_empty());
        if reference_bytes + bytes > 970 {
            break;
        }
        reference_bytes += bytes;
        references.push(id);
    }
    references.reverse();
    Ok(ReplyHeaders {
        to,
        subject,
        in_reply_to,
        references,
    })
}

fn validate_reply(request: &ReplyMailRequest) -> Result<(), ApiError> {
    if request.message.session != request.session {
        return Err(ApiError::FORBIDDEN);
    }
    if request.message.delivery_id.is_empty() || request.message.delivery_id.len() > 128 {
        return Err(ApiError::bad_request("Invalid message reference"));
    }
    validate_request_id(request.request_id.as_deref())?;
    validate_body(request.text.as_deref(), request.html.as_deref())
}

async fn reply(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    request: ReplyMailRequest,
) -> Result<Json<SendMailResponse>, MailError> {
    ensure_enabled(state)?;
    let sender = authorize(state, user, app_id, &request.session).await?;
    validate_reply(&request)?;
    let source = crate::mail_ingress::reply_source(state, &request.message).await?;
    let original = parse_original(&source.raw)?;
    refuse_unsafe_reply(&source, &original)?;
    let headers = reply_headers(&original)?;
    ensure_external(state.mail_automation.domain(), [&headers.to])?;
    let from = crate::mail_ingress::address::sending_address(
        state,
        &request.session,
        Some(&source.recipient),
    )
    .await?;
    let message = AutomationEmailMessage {
        from_name: Some(display_name(&sender.event_name, &from)),
        from_email: Some(from.clone()),
        to: vec![headers.to],
        cc: Vec::new(),
        bcc: Vec::new(),
        reply_to: Some(from),
        subject: headers.subject,
        body_html: request.html,
        body_text: request.text,
        in_reply_to: headers.in_reply_to,
        references: headers.references,
        auto_submitted: Some(AutoSubmitted::Replied),
    };
    submit(
        state,
        Outgoing {
            session: request.session,
            sender,
            request_id: request.request_id,
            message,
        },
    )
    .await
}

#[utoipa::path(
    post, path = "/apps/{app_id}/mail/reply", tag = "app",
    description = "Reply to a retained message belonging to the authorized email event. The server loads the original recipient, subject and threading headers and answers exactly one address: the first Reply-To, else From. Replies to automated mail (bounces, auto-replies, mailing lists) or to mail that failed its DMARC, spam or virus check are refused. Callers supply only locator references and the reply body.",
    params(("app_id"=String,Path)), request_body=ReplyMailRequest,
    responses((status=200,description="Mail provider accepted the reply, or the request_id was already sent",body=SendMailResponse),
        (status=400,description="Invalid reply"),(status=403,description="Session or message not authorized"),
        (status=404,description="Original message not found for this event"),
        (status=409,description="The original is not available for replies, or a reply with this request_id is still being sent"),
        (status=410,description="Original message expired"),
        (status=422,description="The original is automated, failed a provider check, has no valid reply address, or the reply address is an event address"),
        (status=429,description="App send interval or daily recipient limit reached",headers(("Retry-After" = u64, description = "Seconds until a retry can succeed"))),
        (status=503,description="Mail unavailable")),
    security(("bearer_auth"=[]),("pat"=[]),("executor_jwt"=[]))
)]
pub async fn reply_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ReplyMailRequest>,
) -> Result<Json<SendMailResponse>, MailError> {
    reply(&state, &user, &app_id, request).await
}

#[utoipa::path(
    post, path = "/execution/apps/{app_id}/mail/reply", tag = "execution",
    description = "Reply to a retained message of this run's inbound email event. Executor only; same contract as the app route. Retry only 429 responses, after Retry-After.",
    params(("app_id" = String, Path, description = "Application ID")), request_body = ReplyMailRequest,
    responses((status=200,description="Mail provider accepted the reply, or the request_id was already sent",body=SendMailResponse),
        (status=400,description="Invalid reply"),(status=403,description="The caller is not a live run of this event"),
        (status=404,description="Original message not found for this event"),
        (status=409,description="The original is not available for replies, or a reply with this request_id is still being sent"),
        (status=410,description="Original message expired"),
        (status=422,description="The original is automated, failed a provider check, has no valid reply address, or the reply address is an event address"),
        (status=429,description="App send interval or daily recipient limit reached",headers(("Retry-After" = u64, description = "Seconds until a retry can succeed"))),
        (status=503,description="Mail unavailable")),
    security(("executor_jwt"=[]))
)]
pub async fn executor_reply_mail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(request): Json<ReplyMailRequest>,
) -> Result<Json<SendMailResponse>, MailError> {
    require_executor(&user)?;
    reply(&state, &user, &app_id, request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::state::{RunMode, RunStatus};

    fn session() -> MailSession {
        MailSession {
            app_id: "app".into(),
            event_id: "event".into(),
        }
    }
    fn request() -> SendMailRequest {
        SendMailRequest {
            session: session(),
            to: vec!["recipient@example.com".into()],
            cc: vec![],
            bcc: vec![],
            subject: "Order received".into(),
            text: Some("Thank you".into()),
            html: None,
            request_id: None,
        }
    }

    fn run() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run".into(),
            board_id: "board".into(),
            version: None,
            event_id: Some("event".into()),
            status: RunStatus::Running,
            mode: RunMode::Http,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 0,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at: None,
            user_id: None,
            technical_user_id: None,
            app_id: "app".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn executor(sub: &str) -> ExecutorUser {
        ExecutorUser {
            sub: sub.into(),
            app_id: "app".into(),
            run_id: "run".into(),
            technical_user_id: None,
            app_chain: None,
            correlation: None,
        }
    }

    fn reply_source(envelope_from: &str, raw: &[u8]) -> ReplySource {
        ReplySource {
            recipient: "event@mail.example.com".into(),
            envelope_from: envelope_from.into(),
            raw: raw.to_vec(),
            authentication: None,
        }
    }

    #[test]
    fn sink_executor_requires_its_exact_anonymous_sink_identity() {
        let executor = executor("sink:mail-sink");
        let run = run();
        assert!(executor_can_send(&executor, &run, &session(), "mail-sink"));
        assert!(!executor_can_send(
            &executor,
            &run,
            &session(),
            "other-sink"
        ));

        let mut conflicting = executor.clone();
        conflicting.app_chain = Some(vec!["calling-app".into()]);
        assert!(!executor_can_send(
            &conflicting,
            &run,
            &session(),
            "mail-sink"
        ));
        let mut conflicting = executor.clone();
        conflicting.technical_user_id = Some("key".into());
        assert!(!executor_can_send(
            &conflicting,
            &run,
            &session(),
            "mail-sink"
        ));
        let mut attributed = run.clone();
        attributed.user_id = Some("sink:mail-sink".into());
        assert!(!executor_can_send(
            &executor,
            &attributed,
            &session(),
            "mail-sink"
        ));
        let mut attributed = run.clone();
        attributed.technical_user_id = Some("key".into());
        assert!(!executor_can_send(
            &executor,
            &attributed,
            &session(),
            "mail-sink"
        ));
    }

    #[test]
    fn user_executor_requires_the_recorded_user_and_technical_identity() {
        let mut executor = executor("user");
        let mut run = run();
        run.user_id = Some("user".into());
        assert!(executor_can_send(&executor, &run, &session(), "mail-sink"));

        executor.sub = "other-user".into();
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.sub = "user".into();
        executor.technical_user_id = Some("key".into());
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.technical_user_id = None;
        run.technical_user_id = Some("key".into());
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        run.technical_user_id = None;
        run.user_id = None;
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
    }

    #[test]
    fn technical_executor_keeps_its_effective_user_and_supports_app_chains() {
        let mut executor = executor("effective-user");
        executor.technical_user_id = Some("key".into());
        let mut run = run();
        run.user_id = Some("effective-user".into());
        run.technical_user_id = Some("key".into());
        assert!(executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.app_chain = Some(vec!["calling-app".into()]);
        assert!(executor_can_send(&executor, &run, &session(), "mail-sink"));

        executor.sub = "other-user".into();
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.sub = "effective-user".into();
        executor.technical_user_id = Some("other-key".into());
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.technical_user_id = None;
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        executor.technical_user_id = Some("key".into());
        run.user_id = None;
        assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
    }

    #[test]
    fn every_executor_actor_is_bound_to_its_run_app_and_event() {
        for (subject, technical_id) in [
            ("sink:mail-sink", None),
            ("user", None),
            ("user", Some("key")),
        ] {
            let mut executor = executor(subject);
            executor.technical_user_id = technical_id.map(str::to_owned);
            let mut run = run();
            if subject == "user" {
                run.user_id = Some(subject.into());
                run.technical_user_id = technical_id.map(str::to_owned);
            }
            assert!(executor_can_send(&executor, &run, &session(), "mail-sink"));
            let mut other_session = session();
            other_session.event_id = "other-event".into();
            assert!(!executor_can_send(
                &executor,
                &run,
                &other_session,
                "mail-sink"
            ));
            other_session = session();
            other_session.app_id = "other-app".into();
            assert!(!executor_can_send(
                &executor,
                &run,
                &other_session,
                "mail-sink"
            ));
            let mut other_executor = executor.clone();
            other_executor.app_id = "other-app".into();
            assert!(!executor_can_send(
                &other_executor,
                &run,
                &session(),
                "mail-sink"
            ));
            other_executor = executor.clone();
            other_executor.run_id = "other-run".into();
            assert!(!executor_can_send(
                &other_executor,
                &run,
                &session(),
                "mail-sink"
            ));
            run.event_id = None;
            assert!(!executor_can_send(&executor, &run, &session(), "mail-sink"));
        }
    }

    #[test]
    fn execution_routes_accept_only_executor_tokens() {
        assert!(require_executor(&AppUser::Executor(executor("user"))).is_ok());
        assert!(require_executor(&AppUser::Unauthorized).is_err());
    }

    #[test]
    fn validates_addresses_subject_and_body_bounds() {
        assert!(validate(&request()).is_ok());
        for address in [
            "a@@example.com",
            "@example.com",
            "a@-example.com",
            "a..b@example.com",
            "a@example.com\r\nBcc: other@example.com",
        ] {
            assert!(!valid_address(address));
        }
        let mut value = request();
        value.bcc = vec!["a@example.com".into(); MAX_RECIPIENTS];
        assert!(validate(&value).is_err());
        let mut value = request();
        value.subject = "Subject\r\nBcc: victim@example.com".into();
        assert!(validate(&value).is_err());
        assert!(validate_body(None, None).is_err());
        assert!(validate_body(Some(&"x".repeat(MAX_BODY_BYTES + 1)), None).is_err());
    }

    #[test]
    fn request_ids_follow_the_node_contract() {
        let longest = "a".repeat(128);
        let too_long = "a".repeat(129);
        for id in ["ctx-1:trace.2_a", longest.as_str()] {
            assert!(validate_request_id(Some(id)).is_ok(), "{id}");
        }
        for id in ["", "has space", "slash/id", too_long.as_str(), "ünïcode"] {
            assert!(validate_request_id(Some(id)).is_err(), "{id}");
        }
        assert!(validate_request_id(None).is_ok());
        let mut value = request();
        value.request_id = Some("bad id".into());
        assert!(validate(&value).is_err());
    }

    #[test]
    fn duplicate_recipients_receive_one_copy_with_to_first() {
        let mut value = request();
        value.to = vec!["A@example.com".into(), "a@example.com".into()];
        value.cc = vec!["a@EXAMPLE.com".into(), "b@example.com".into()];
        value.bcc = vec!["B@example.com".into(), "c@example.com".into()];
        dedupe_recipients(&mut value);
        assert_eq!(value.to, vec!["A@example.com"]);
        assert_eq!(value.cc, vec!["b@example.com"]);
        assert_eq!(value.bcc, vec!["c@example.com"]);
    }

    #[test]
    fn event_addresses_cannot_be_mailed_by_flows() {
        let domain = Some("mail.example.com");
        let external = ["someone@example.com".to_owned()];
        assert!(ensure_external(domain, &external).is_ok());
        for address in [
            "m-abc@mail.example.com",
            "orders@MAIL.example.com",
            "x@eu.mail.example.com",
        ] {
            let error = ensure_external(domain, &[address.to_owned()]).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        }
        assert!(ensure_external(None, &["x@mail.example.com".to_owned()]).is_ok());
        assert!(ensure_external(domain, &["x@notmail.example.com".to_owned()]).is_ok());
    }

    #[test]
    fn senders_are_named_after_their_event_never_the_platform() {
        assert_eq!(
            display_name("Invoice Intake", "m-1@mail.example.com"),
            "Invoice Intake"
        );
        assert_eq!(
            display_name("  <Orders>\r\n\"Desk\" ", "m-1@mail.example.com"),
            "Orders Desk"
        );
        assert_eq!(
            display_name("Bestellungen & Rückfragen", "m-1@mail.example.com"),
            "Bestellungen & Rückfragen"
        );
        assert_eq!(display_name("<>", "orders@mail.example.com"), "orders");
        assert_eq!(
            display_name(&"x".repeat(100), "m-1@mail.example.com").len(),
            MAX_DISPLAY_NAME
        );
    }

    #[test]
    fn rate_limits_tell_the_caller_when_to_retry() {
        let response = too_many("Slow down", 0).into_response();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[RETRY_AFTER], "1");
        let response = MailError::from(ApiError::bad_request("Invalid")).into_response();
        assert!(response.headers().get(RETRY_AFTER).is_none());
    }

    #[test]
    fn rejects_caller_control_of_sender_or_reply_metadata() {
        let base = serde_json::json!({"session":session(),"to":["a@example.com"],"subject":"Mail","text":"Body"});
        for field in ["from", "from_email", "reply_to", "headers"] {
            let mut value = base.clone();
            value[field] = serde_json::json!("spoof@example.com");
            assert!(serde_json::from_value::<SendMailRequest>(value).is_err());
        }
        let mut with_id = base.clone();
        with_id["request_id"] = serde_json::json!("run-1:trace-1");
        assert_eq!(
            serde_json::from_value::<SendMailRequest>(with_id)
                .unwrap()
                .request_id
                .as_deref(),
            Some("run-1:trace-1")
        );
        let base = serde_json::json!({"session":session(),"message":{"session":session(),"delivery_id":"mail-1"},"text":"Reply"});
        for field in [
            "to",
            "from",
            "subject",
            "in_reply_to",
            "references",
            "raw_path",
        ] {
            let mut value = base.clone();
            value[field] = serde_json::json!("forged");
            assert!(serde_json::from_value::<ReplyMailRequest>(value).is_err());
        }
        let mut value: ReplyMailRequest = serde_json::from_value(base).unwrap();
        assert!(validate_reply(&value).is_ok());
        value.message.session.event_id = "other-event".into();
        assert!(validate_reply(&value).is_err());
    }

    #[test]
    fn replies_use_original_reply_to_and_thread() {
        let raw=b"From: Original <author@example.com>\r\nReply-To: help@example.com, other@example.com\r\nTo: unrelated@example.com\r\nSubject: Order\r\nMessage-ID: <parent@example.com>\r\nReferences: <root@example.com> <previous@example.com>\r\n\r\nBody";
        let reply = reply_headers(&parse_original(raw).unwrap()).unwrap();
        assert_eq!(reply.to, "help@example.com");
        assert_eq!(reply.subject, "Re: Order");
        assert_eq!(reply.in_reply_to.as_deref(), Some("<parent@example.com>"));
        assert_eq!(
            reply.references,
            vec![
                "<root@example.com>",
                "<previous@example.com>",
                "<parent@example.com>"
            ]
        );
        let raw = b"From: First <first@example.com>, Second <second@example.com>\r\nSubject: Hi\r\n\r\nBody";
        assert_eq!(
            reply_headers(&parse_original(raw).unwrap()).unwrap().to,
            "first@example.com"
        );
    }

    #[test]
    fn reply_headers_reject_injection_and_bound_long_threads() {
        for id in [
            "parent@example.com\r\nBcc: victim@example.com",
            "<a@example.com> <b@example.com>",
            "a\\b@example.com",
            "no-at-sign",
        ] {
            assert!(message_id(id).is_none());
        }
        let refs = (0..20_000)
            .map(|i| format!("<ancestor-{i}@example.com>"))
            .collect::<Vec<_>>()
            .join(" ");
        let raw = format!(
            "From: author@example.com\r\nSubject: re: Existing\r\nMessage-ID: <parent@example.com>\r\nReferences: {refs}\r\n\r\n"
        );
        let reply = reply_headers(&parse_original(raw.as_bytes()).unwrap()).unwrap();
        assert_eq!(reply.subject, "re: Existing");
        assert!(reply.references.join(" ").len() <= 970);
        assert_eq!(reply.references.last().unwrap(), "<parent@example.com>");
        let error = reply_headers(&parse_original(b"From: invalid\r\n\r\n").unwrap())
            .err()
            .unwrap();
        assert_eq!(error.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn replies_to_automated_or_failed_mail_are_refused() {
        let human: &[u8] = b"From: author@example.com\r\nSubject: Order\r\n\r\nBody";
        let source = reply_source("author@example.com", human);
        assert!(refuse_unsafe_reply(&source, &parse_original(&source.raw).unwrap()).is_ok());

        let bounce = reply_source("", human);
        let error =
            refuse_unsafe_reply(&bounce, &parse_original(&bounce.raw).unwrap()).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);

        let vacation = reply_source(
            "author@example.com",
            b"From: author@example.com\r\nAuto-Submitted: auto-replied\r\n\r\nAway",
        );
        assert!(refuse_unsafe_reply(&vacation, &parse_original(&vacation.raw).unwrap()).is_err());

        let mut spoofed = reply_source("author@example.com", human);
        spoofed.authentication = Some(flow_like_catalog_core::InboundEmailAuthentication {
            dmarc: Some(flow_like_catalog_core::InboundEmailVerdict {
                status: "FAIL".into(),
            }),
            ..Default::default()
        });
        let error =
            refuse_unsafe_reply(&spoofed, &parse_original(&spoofed.raw).unwrap()).unwrap_err();
        assert!(error.public_message().unwrap().contains("DMARC"));
    }

    #[test]
    fn live_run_is_bound_to_this_event_and_app() {
        let mut run = run();
        assert!(run_can_send(&run, &session()));
        let mut other = session();
        other.event_id = "another-event".into();
        assert!(!run_can_send(&run, &other));
        other = session();
        other.app_id = "another-app".into();
        assert!(!run_can_send(&run, &other));
        run.status = RunStatus::Completed;
        assert!(!run_can_send(&run, &session()));
        run.status = RunStatus::Running;
        run.run_variant = RunVariant::Shadow;
        assert!(!run_can_send(&run, &session()));
        run.run_variant = RunVariant::Regression;
        assert!(!run_can_send(&run, &session()));
        run.run_variant = RunVariant::Primary;
        run.shadow_of_run_id = Some("original-run".into());
        assert!(!run_can_send(&run, &session()));
        run.shadow_of_run_id = None;
        run.regression_run_id = Some("regression-run".into());
        assert!(!run_can_send(&run, &session()));
        run.regression_run_id = None;
        run.expires_at = Some(0);
        assert!(!run_can_send(&run, &session()));
    }
}
