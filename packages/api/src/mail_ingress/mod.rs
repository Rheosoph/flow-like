//! Durable inbound mail acceptance and asynchronous event dispatch.
pub mod address;
pub(crate) mod limits;
mod payload;

pub(crate) use payload::is_automated;

use crate::{
    cache::Reservation,
    entity::{app as app_entity, event_sink, sea_orm_active_enums::Status, sink_token},
    error::ApiError,
    execution::state::RunStatus,
    routes::sink::trigger::{
        SinkTriggerClaims, TriggerEventInput, trigger_event_with_run_id, validate_sink_trigger_jwt,
    },
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, Path, State},
    http::{HeaderMap, StatusCode, request::Parts},
    routing::{get, post},
};
use base64::Engine;
use flow_like::flow::event::{Event, EventExecutionMode};
use flow_like_catalog_core::{InboundEmailAuthentication, InboundEmailVerdict};
use flow_like_storage::object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, QueryFilter,
    Statement, Value,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    time::Duration,
};
use utoipa::ToSchema;

pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub(super) fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}
pub(super) fn max_bytes(state: &AppState) -> usize {
    state.mail_automation.max_bytes
}
fn ttl_ms(state: &AppState) -> i64 {
    state.mail_automation.ttl_seconds as i64 * 1000
}
/// A dispatched run must be able to read its files for a while; short TTLs halve the margin.
fn dispatch_margin_ms(ttl_ms: i64) -> i64 {
    (ttl_ms / 2).min(MIN_DISPATCH_LIFETIME_MS)
}
fn min_dispatch_lifetime_ms(state: &AppState) -> i64 {
    dispatch_margin_ms(ttl_ms(state))
}

pub(super) fn digest(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for part in parts {
        h.update(&(part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

pub fn routes(max_bytes: usize) -> Router<AppState> {
    Router::new()
        .route("/recipients/{address}", get(recipient))
        .route("/ingest", post(ingest))
        .route("/dispatch", post(dispatch))
        .layer(DefaultBodyLimit::max(max_bytes * 4 / 3 + 64 * 1024))
}

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<SinkTriggerClaims, ApiError> {
    let value = crate::middleware::jwt::viewer_authorization(headers)
        .ok_or_else(|| ApiError::unauthorized("Missing bearer token"))?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid bearer token"))?;
    let secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::forbidden("Mail ingress is disabled"))?;
    let claims = validate_sink_trigger_jwt(token, secret)?;
    if !claims.sink_types.iter().any(|t| t == "inbound_email") {
        return Err(ApiError::forbidden("Token cannot receive email"));
    }
    // New ingress integrations require registered tokens so revocation is immediate.
    let jti = claims
        .jti
        .as_deref()
        .ok_or_else(|| ApiError::unauthorized("A registered sink token is required"))?;
    let registered = sink_token::Entity::find_by_id(jti).one(&state.db).await?;
    if registered.is_none_or(|t| t.revoked) {
        return Err(ApiError::unauthorized("Sink token is revoked or unknown"));
    }
    Ok(claims)
}

/// Ingress credentials, checked from the headers before any request body is read.
pub struct IngressAuth(SinkTriggerClaims);

impl FromRequestParts<AppState> for IngressAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        authorize(state, &parts.headers).await.map(Self)
    }
}

fn allowed(claims: &SinkTriggerClaims, app: &str) -> bool {
    claims
        .app_ids
        .as_ref()
        .is_none_or(|ids| ids.iter().any(|id| id == app))
}

pub(crate) async fn live_target(
    state: &AppState,
    app: &str,
    id: &str,
) -> Result<Option<(event_sink::Model, Event)>, ApiError> {
    if !state.mail_automation.enabled {
        return Ok(None);
    }
    if app_entity::Entity::find_by_id(app)
        .filter(app_entity::Column::Status.eq(Status::Active))
        .one(&state.db)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(id))
        .filter(event_sink::Column::AppId.eq(app))
        .filter(event_sink::Column::Active.eq(true))
        .filter(event_sink::Column::SinkType.eq("inbound_email"))
        .one(&state.db)
        .await?;
    let Some(sink) = sink else {
        return Ok(None);
    };
    let event = crate::routes::app::events::db::get_event_from_db_opt(&state.db, id, app).await?;
    Ok(event
        .filter(|event| {
            event.active
                && event.event_type == "inbound_email"
                && event.execution_mode == EventExecutionMode::Remote
        })
        .map(|e| (sink, e)))
}

/// The user whose stored token runs this sink, when the sink has one.
pub(crate) async fn sink_owner(
    state: &AppState,
    sink: &event_sink::Model,
) -> Result<Option<String>, ApiError> {
    let token = sink
        .pat_encrypted
        .as_ref()
        .and_then(|s| crate::routes::app::events::db::decrypt_token(s, &state.encryption_key));
    crate::routes::sink::trigger::resolve_sink_pat_user_id(state, sink, token.as_deref()).await
}

fn failed(verdict: Option<&InboundEmailVerdict>) -> bool {
    verdict.is_some_and(|verdict| verdict.status.eq_ignore_ascii_case("FAIL"))
}

/// The provider verdict that makes a message unsafe to answer.
pub(crate) fn failed_verdict(
    authentication: Option<&InboundEmailAuthentication>,
) -> Option<&'static str> {
    let authentication = authentication?;
    [
        ("DMARC", authentication.dmarc.as_ref()),
        ("spam", authentication.spam.as_ref()),
        ("virus", authentication.virus.as_ref()),
    ]
    .into_iter()
    .find_map(|(name, verdict)| failed(verdict).then_some(name))
}

async fn resolve(
    state: &AppState,
    claims: &SinkTriggerClaims,
    raw: &str,
) -> Result<Option<(address::Address, Event)>, ApiError> {
    let Some(domain) = address::domain(state) else {
        return Ok(None);
    };
    let normalized = raw.trim().to_ascii_lowercase();
    let Some((local, actual_domain)) = normalized.split_once('@') else {
        return Ok(None);
    };
    if actual_domain != domain || local.is_empty() || normalized.len() > 320 {
        return Ok(None);
    }
    let address = address::Address::find_by_statement(statement(
        r#"SELECT * FROM "InboundMailAddress" WHERE "address"=$1 AND "active"=TRUE"#,
        vec![normalized.into()],
    ))
    .one(&state.db)
    .await?;
    let Some(address) = address else {
        return Ok(None);
    };
    if !allowed(claims, &address.app_id) {
        return Ok(None);
    }
    let Some((_, event)) = live_target(state, &address.app_id, &address.event_id).await? else {
        return Ok(None);
    };
    Ok(Some((address, event)))
}

#[derive(Serialize, ToSchema)]
pub struct RecipientResponse {
    /// Whether mail for this address reaches an active event.
    accepted: bool,
}

#[utoipa::path(
    get,
    path = "/sink/mail/recipients/{address}",
    tag = "sink",
    description = "Mail ingress lookup: whether the server accepts mail for an address, so unknown recipients can be rejected during the SMTP session.",
    params(("address" = String, Path, description = "Recipient email address")),
    responses(
        (status = 200, description = "Whether the address is deliverable", body = RecipientResponse),
        (status = 401, description = "Missing, invalid or revoked ingress token"),
        (status = 403, description = "Token cannot receive email")
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn recipient(
    State(state): State<AppState>,
    IngressAuth(claims): IngressAuth,
    Path(address): Path<String>,
) -> Result<Json<RecipientResponse>, ApiError> {
    Ok(Json(RecipientResponse {
        accepted: resolve(&state, &claims, &address).await?.is_some(),
    }))
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct S3Source {
    bucket: String,
    key: String,
    #[serde(default)]
    version_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestRequest {
    /// `ses` for an S3 receipt object or `postfix` for an inline message.
    source: String,
    delivery_id: String,
    /// SMTP envelope sender; empty for bounces.
    envelope_from: String,
    recipients: Vec<String>,
    #[serde(default)]
    raw_mime_base64: Option<String>,
    #[serde(default)]
    s3: Option<S3Source>,
    /// Provider verdicts such as SES spam, virus, SPF, DKIM and DMARC.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    authentication: Option<InboundEmailAuthentication>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Envelope {
    source: String,
    delivery_id: String,
    envelope_from: String,
    recipient: String,
    s3: Option<S3Source>,
    raw_object: Option<String>,
    authentication: Option<InboundEmailAuthentication>,
    #[serde(default)]
    event_snapshot: Option<Event>,
}

#[derive(Serialize, ToSchema)]
pub struct IngestResponse {
    message_id: String,
    /// `accepted`, `ignored` (no active recipient) or `rejected` (virus verdict).
    status: String,
}

fn ingest_response(message_id: String, status: &str) -> (StatusCode, Json<IngestResponse>) {
    (
        StatusCode::ACCEPTED,
        Json(IngestResponse {
            message_id,
            status: status.to_owned(),
        }),
    )
}

fn validate_source(
    config: &crate::runtime_config::mail::MailAutomationConfig,
    input: &IngestRequest,
) -> Result<(), ApiError> {
    if input.delivery_id.is_empty()
        || input.delivery_id.len() > 256
        || input.envelope_from.len() > 320
        || input.recipients.is_empty()
        || input.recipients.len() > 100
        || input.recipients.iter().any(|r| r.len() > 320)
        || serde_json::to_vec(&input.authentication)?.len() > 16384
    {
        return Err(ApiError::bad_request("Invalid mail envelope"));
    }
    match (input.source.as_str(), &input.s3, &input.raw_mime_base64) {
        ("ses", Some(s3), None) => {
            let bucket = config.bucket.as_deref().unwrap_or_default();
            let prefix = &config.prefix;
            let id = input
                .delivery_id
                .strip_prefix("ses:")
                .ok_or_else(|| ApiError::bad_request("Invalid SES delivery ID"))?;
            if bucket.is_empty()
                || id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || s3.bucket != bucket
                || s3.key != format!("{prefix}{id}")
                || s3.version_id.as_ref().is_some_and(|v| v.len() > 1024)
            {
                return Err(ApiError::bad_request(
                    "Mail object is outside the configured receipt location",
                ));
            }
            #[cfg(not(feature = "ses"))]
            return Err(ApiError::bad_request(
                "This server was built without SES support",
            ));
            #[cfg(feature = "ses")]
            Ok(())
        }
        ("postfix", None, Some(raw)) if raw.len() <= config.max_bytes * 4 / 3 + 4 => Ok(()),
        _ => Err(ApiError::bad_request(
            "Supply an SES object reference or a Postfix MIME message",
        )),
    }
}

const LEASE_MS: i64 = 300_000;
const WORK_TIMEOUT: Duration = Duration::from_secs(120);
const MIN_DISPATCH_LIFETIME_MS: i64 = 600_000;
/// Files of a still-running execution outlive the delivery expiry by at most this long.
const RUN_RETENTION_CAP_MS: i64 = 86_400_000;
const RUN_RECHECK_MS: i64 = 600_000;
/// Deduplication rows outlive every provider and Postfix retry window.
const DEDUPE_RETENTION_MS: i64 = 7 * 86_400_000;
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(600);
const PURGE_BATCH: u32 = 500;
/// Rows that still hold encrypted metadata or files.
const HOLDING: &str =
    "'receiving','pending','sending','dispatched','disabled','rejected','too_large'";
/// Final statuses after cleanup; `rejected` and `too_large` keep their reason.
const SETTLED: &str = "'expired','completed','rejected','too_large'";

fn raw_receipt_path(delivery_id: &str, bytes: &[u8]) -> String {
    format!(
        "tmp/mail-ingress/{delivery_id}/{}.enc",
        blake3::hash(bytes).to_hex()
    )
}

async fn put_immutable_raw(
    store: &dyn ObjectStore,
    path: &str,
    encrypted: &[u8],
) -> Result<(), ApiError> {
    match store
        .put_opts(
            &path.into(),
            encrypted.to_vec().into(),
            PutOptions::from(PutMode::Create),
        )
        .await
    {
        Ok(_) | Err(flow_like_storage::object_store::Error::AlreadyExists { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn same_receipt(stored: &Envelope, incoming: &Envelope) -> bool {
    stored.source == incoming.source
        && stored.delivery_id == incoming.delivery_id
        && stored.envelope_from == incoming.envelope_from
        && stored.raw_object == incoming.raw_object
        && stored.s3 == incoming.s3
}

fn seal(state: &AppState, envelope: &Envelope) -> Result<String, ApiError> {
    Ok(crate::utils::crypto::encrypt_secret(
        &serde_json::to_string(envelope)?,
        &state.encryption_key,
    ))
}

async fn delivery(state: &AppState, id: &str) -> Result<Delivery, ApiError> {
    Delivery::find_by_statement(statement(
        r#"SELECT * FROM "InboundMailDelivery" WHERE "id"=$1"#,
        vec![id.into()],
    ))
    .one(&state.db)
    .await?
    .ok_or_else(|| ApiError::internal(format!("Mail delivery {id} disappeared")))
}

struct NewDelivery<'a> {
    id: &'a str,
    message_id: &'a str,
    app: &'a str,
    event: &'a str,
    status: &'a str,
    envelope: String,
    next_attempt: i64,
    received_at: i64,
    expires_at: i64,
    lease: Option<String>,
}

async fn insert_delivery(state: &AppState, row: NewDelivery<'_>) -> Result<bool, ApiError> {
    let inserted = state.db.execute_raw(statement(
        r#"INSERT INTO "InboundMailDelivery" ("id","messageId","appId","eventId","status","envelope","attempts","nextAttempt","receivedAt","expiresAt","runId","lease") VALUES ($1,$2,$3,$4,$5,$6,0,$7,$8,$9,$10,$11) ON CONFLICT ("id") DO NOTHING"#,
        vec![
            row.id.into(),
            row.message_id.into(),
            row.app.into(),
            row.event.into(),
            row.status.into(),
            row.envelope.into(),
            row.next_attempt.into(),
            row.received_at.into(),
            row.expires_at.into(),
            format!("mail_{}", row.id).into(),
            row.lease.into(),
        ],
    )).await?;
    Ok(inserted.rows_affected() == 1)
}

/// SES accepts the whole domain, so role mailboxes arrive here and are only recorded.
/// Postfix rejects or forwards them before ingest because the recipient lookup refuses them.
fn log_reserved(state: &AppState, input: &IngestRequest) {
    if input.source != "ses" {
        return;
    }
    let Some(domain) = address::domain(state) else {
        return;
    };
    let reserved = input
        .recipients
        .iter()
        .filter(|recipient| address::reserved_recipient(&domain, recipient))
        .collect::<Vec<_>>();
    if !reserved.is_empty() {
        tracing::warn!(
            recipients = ?reserved,
            envelope_from = %input.envelope_from,
            delivery_id = %input.delivery_id,
            "Inbound mail for a reserved role address is not delivered to any event"
        );
    }
}

type Targets = std::collections::BTreeMap<(String, String), (address::Address, Event)>;

/// A virus verdict never reaches storage or a flow; the row keeps the SES object for cleanup.
async fn reject(
    state: &AppState,
    input: &IngestRequest,
    message_id: &str,
    targets: Targets,
) -> Result<(), ApiError> {
    let timestamp = now();
    for ((app, event), (address, _)) in targets {
        let id = digest(&[&input.source, &input.delivery_id, &app, &event]);
        let envelope = Envelope {
            source: input.source.clone(),
            delivery_id: input.delivery_id.clone(),
            envelope_from: input.envelope_from.clone(),
            recipient: address.address,
            s3: input.s3.clone(),
            raw_object: None,
            authentication: input.authentication.clone(),
            event_snapshot: None,
        };
        insert_delivery(
            state,
            NewDelivery {
                id: &id,
                message_id,
                app: &app,
                event: &event,
                status: "rejected",
                envelope: seal(state, &envelope)?,
                next_attempt: timestamp,
                received_at: timestamp,
                expires_at: timestamp,
                lease: None,
            },
        )
        .await?;
        tracing::warn!(delivery_id = %id, app_id = %app, event_id = %event, "Inbound email with a failed virus verdict was rejected");
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/sink/mail/ingest",
    tag = "sink",
    description = "Mail ingress hand-off: durably accept one received message for every active event address among its recipients. Unknown recipients are ignored and messages with a failed virus verdict are rejected; both still answer 202 so the ingress does not retry.",
    request_body = IngestRequest,
    responses(
        (status = 202, description = "The message was accepted, ignored or rejected", body = IngestResponse),
        (status = 400, description = "Invalid envelope, message or receipt location"),
        (status = 401, description = "Missing, invalid or revoked ingress token"),
        (status = 403, description = "Token cannot receive email"),
        (status = 409, description = "The delivery ID was reused for different content, or an upload lease expired"),
        (status = 503, description = "Inbound email is disabled")
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn ingest(
    State(state): State<AppState>,
    IngressAuth(claims): IngressAuth,
    Json(input): Json<IngestRequest>,
) -> Result<(StatusCode, Json<IngestResponse>), ApiError> {
    if !state.mail_automation.enabled {
        return Err(ApiError::service_unavailable(
            "Inbound email is disabled on this deployment",
        ));
    }
    validate_source(&state.mail_automation, &input)?;
    let message_id = digest(&[&input.source, &input.delivery_id]);
    log_reserved(&state, &input);
    let mut targets = Targets::new();
    for recipient in &input.recipients {
        if let Some((address, event)) = resolve(&state, &claims, recipient).await? {
            targets
                .entry((address.app_id.clone(), address.event_id.clone()))
                .or_insert((address, event));
        }
    }
    // SES accepts the domain as a whole. Unknown or disabled recipients are discarded here.
    if targets.is_empty() {
        return Ok(ingest_response(message_id, "ignored"));
    }
    if failed(
        input
            .authentication
            .as_ref()
            .and_then(|authentication| authentication.virus.as_ref()),
    ) {
        reject(&state, &input, &message_id, targets).await?;
        return Ok(ingest_response(message_id, "rejected"));
    }
    let raw_bytes = input
        .raw_mime_base64
        .as_ref()
        .map(|raw| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(raw)
                .map_err(|_| ApiError::bad_request("Invalid MIME encoding"))?;
            if bytes.is_empty() || bytes.len() > max_bytes(&state) {
                return Err(ApiError::bad_request("Message exceeds the mail size limit"));
            }
            Ok(bytes)
        })
        .transpose()?;
    let encrypted_raw = input
        .raw_mime_base64
        .as_ref()
        .map(|raw| crate::utils::crypto::encrypt_secret(raw, &state.encryption_key));
    for ((app, event), (address, event_snapshot)) in targets {
        let timestamp = now();
        let id = digest(&[&input.source, &input.delivery_id, &app, &event]);
        // Each delivery owns its raw bytes; expiry of another recipient cannot remove them.
        let raw_object = raw_bytes.as_ref().map(|bytes| raw_receipt_path(&id, bytes));
        let incoming = Envelope {
            source: input.source.clone(),
            delivery_id: input.delivery_id.clone(),
            envelope_from: input.envelope_from.clone(),
            recipient: address.address,
            s3: input.s3.clone(),
            raw_object: raw_object.clone(),
            authentication: input.authentication.clone(),
            event_snapshot: Some(event_snapshot),
        };
        let lease = flow_like_types::create_id();
        let receiving = raw_object.is_some();
        // Persist the cleanup reference before writing the blob. Receiving rows cannot dispatch.
        let inserted = insert_delivery(
            &state,
            NewDelivery {
                id: &id,
                message_id: &message_id,
                app: &app,
                event: &event,
                status: if receiving { "receiving" } else { "pending" },
                envelope: seal(&state, &incoming)?,
                next_attempt: if receiving {
                    timestamp + LEASE_MS
                } else {
                    timestamp
                },
                received_at: timestamp,
                expires_at: timestamp + ttl_ms(&state),
                lease: receiving.then(|| lease.clone()),
            },
        )
        .await?;
        if !inserted {
            let stored = delivery(&state, &id).await?;
            // Retained deduplication rows never resurrect expired mail or its files.
            if stored.expires_at <= now() || stored.envelope.is_none() {
                continue;
            }
            if !same_receipt(&envelope(&state, &stored)?, &incoming) {
                return Err(ApiError::conflict(
                    "This delivery ID was already accepted with different mail content",
                ));
            }
            if stored.status != "receiving" {
                continue;
            }
            let timestamp = now();
            let claimed=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "lease"=$1,"nextAttempt"=$2 WHERE "id"=$3 AND "status"='receiving' AND "nextAttempt" <= $4 AND "expiresAt" > $4"#,
                vec![lease.clone().into(),(timestamp+LEASE_MS).into(),id.clone().into(),timestamp.into()])).await?;
            if claimed.rows_affected() != 1 {
                return Err(ApiError::internal(
                    "Mail upload is in progress; retry this delivery",
                ));
            }
        }
        if let (Some(path), Some(encrypted)) = (raw_object, encrypted_raw.as_ref()) {
            let write = async {
                let store = payload::store(state.as_ref()).await?;
                put_immutable_raw(store.as_generic().as_ref(), &path, encrypted.as_bytes()).await
            };
            let result = flow_like_types::tokio::time::timeout(WORK_TIMEOUT, write)
                .await
                .unwrap_or_else(|_| Err(ApiError::internal("Mail upload timed out")));
            // Release a failed upload for provider retry; its durable cleanup reference remains.
            let status = if result.is_ok() {
                "pending"
            } else {
                "receiving"
            };
            let finalized=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "status"=$1,"nextAttempt"=$2,"lease"=NULL WHERE "id"=$3 AND "status"='receiving' AND "lease"=$4"#,
                vec![status.into(),now().into(),id.into(),lease.into()])).await?;
            result?;
            if finalized.rows_affected() != 1 {
                return Err(ApiError::conflict(
                    "Mail upload lease expired; retry this delivery",
                ));
            }
        }
    }
    Ok(ingest_response(message_id, "accepted"))
}

#[derive(Clone, FromQueryResult)]
struct Delivery {
    id: String,
    #[sea_orm(from_alias = "messageId")]
    message_id: String,
    status: String,
    #[sea_orm(from_alias = "appId")]
    app_id: String,
    #[sea_orm(from_alias = "eventId")]
    event_id: String,
    envelope: Option<String>,
    objects: Option<String>,
    attempts: i32,
    #[sea_orm(from_alias = "receivedAt")]
    received_at: i64,
    #[sea_orm(from_alias = "expiresAt")]
    expires_at: i64,
    #[sea_orm(from_alias = "runId")]
    run_id: String,
}
fn envelope(state: &AppState, delivery: &Delivery) -> Result<Envelope, ApiError> {
    let raw = delivery
        .envelope
        .as_ref()
        .and_then(|s| crate::utils::crypto::decrypt_secret(s, &state.encryption_key))
        .ok_or_else(|| {
            ApiError::internal(format!(
                "Cannot decrypt the envelope of mail delivery {}",
                delivery.id
            ))
        })?;
    Ok(serde_json::from_str(&raw)?)
}

async fn deliver(
    state: &AppState,
    row: &Delivery,
    lease: &str,
    expires_at: i64,
) -> Result<bool, ApiError> {
    let Some((sink, event)) = live_target(state, &row.app_id, &row.event_id).await? else {
        return Ok(false);
    };
    let mut envelope = envelope(state, row)?;
    let event = if let Some(snapshot) = &envelope.event_snapshot {
        snapshot.clone()
    } else {
        envelope.event_snapshot = Some(event.clone());
        let frozen = state
            .db
            .execute_raw(statement(
                r#"UPDATE "InboundMailDelivery" SET "envelope"=$1 WHERE "id"=$2 AND "lease"=$3"#,
                vec![
                    seal(state, &envelope)?.into(),
                    row.id.clone().into(),
                    lease.into(),
                ],
            ))
            .await?;
        if frozen.rows_affected() != 1 {
            return Err(ApiError::conflict("Mail dispatch lease expired"));
        }
        event
    };
    let raw = payload::read(state, &envelope).await?;
    let (email, objects) = payload::prepare(state, row, &sink, &envelope, &raw, expires_at).await?;
    // Record the complete cleanup list before writing files or starting the execution.
    let paths = merge_paths(
        row.objects.as_deref(),
        objects.iter().map(|(path, _)| path.as_str()),
    )?;
    let result=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "objects"=$1 WHERE "id"=$2 AND "lease"=$3 AND "expiresAt">$4"#,vec![serde_json::to_string(&paths)?.into(),row.id.clone().into(),lease.into(),now().into()])).await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict("Mail dispatch lease expired"));
    }
    let store = payload::store(state.as_ref()).await?;
    for (path, bytes) in objects {
        store.as_generic().put(&path.into(), bytes.into()).await?;
    }
    let still_owned=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "nextAttempt"=$1 WHERE "id"=$2 AND "lease"=$3 AND "expiresAt">$4"#,vec![(now()+LEASE_MS).into(),row.id.clone().into(),lease.into(),now().into()])).await?;
    if still_owned.rows_affected() != 1 {
        return Err(ApiError::conflict("Mail dispatch lease expired"));
    }
    let result = trigger_event_with_run_id(
        state,
        sink,
        event,
        TriggerEventInput {
            event_id: row.event_id.clone(),
            payload: Some(serde_json::json!({"email":email})),
            idempotency_key: Some(row.id.clone()),
        },
        row.run_id.clone(),
    )
    .await?;
    if !result.triggered {
        return Err(ApiError::internal(format!(
            "Mail execution dispatch failed for delivery {}",
            row.id
        )));
    }
    Ok(true)
}

fn merge_paths<'a>(
    stored: Option<&str>,
    incoming: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, ApiError> {
    let mut paths: BTreeSet<String> = stored
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    paths.extend(incoming.into_iter().map(str::to_owned));
    Ok(paths.into_iter().collect())
}

/// Binds the token's app scope before `tail`, so a LIMIT never counts other apps' rows.
fn scoped(base: &str, mut values: Vec<Value>, claims: &SinkTriggerClaims, tail: &str) -> Statement {
    let mut sql = base.to_owned();
    if let Some(apps) = &claims.app_ids {
        if apps.is_empty() {
            sql.push_str(" AND FALSE");
        } else {
            let placeholders = apps
                .iter()
                .map(|app| {
                    values.push(app.clone().into());
                    format!("${}", values.len())
                })
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(r#" AND "appId" IN ({placeholders})"#));
        }
    }
    sql.push_str(tail);
    statement(&sql, values)
}

fn pending_deliveries(claims: &SinkTriggerClaims, timestamp: i64, min_expiry: i64) -> Statement {
    scoped(
        r#"SELECT * FROM "InboundMailDelivery" WHERE "status" IN ('pending','sending') AND "nextAttempt" <= $1 AND "expiresAt" > $2"#,
        vec![timestamp.into(), min_expiry.into()],
        claims,
        r#" ORDER BY "nextAttempt" LIMIT 20"#,
    )
}

fn expired_deliveries(claims: &SinkTriggerClaims, timestamp: i64) -> Statement {
    scoped(
        &format!(
            r#"SELECT * FROM "InboundMailDelivery" WHERE "status" IN ({HOLDING}) AND "expiresAt" <= $1 AND "envelope" IS NOT NULL AND "nextAttempt" <= $1"#
        ),
        vec![timestamp.into()],
        claims,
        r#" ORDER BY "expiresAt" LIMIT 50"#,
    )
}

fn settled_deliveries(claims: &SinkTriggerClaims, cutoff: i64) -> Statement {
    scoped(
        &format!(
            r#"DELETE FROM "InboundMailDelivery" WHERE "id" IN (SELECT "id" FROM "InboundMailDelivery" WHERE "status" IN ({SETTLED}) AND "envelope" IS NULL AND "expiresAt" <= $1"#
        ),
        vec![cutoff.into()],
        claims,
        &format!(" LIMIT {PURGE_BATCH})"),
    )
}

async fn clean_delivery(state: &AppState, row: &Delivery, lease: &str) -> Result<bool, ApiError> {
    let env = envelope(state, row)?;
    let paths = merge_paths(row.objects.as_deref(), env.raw_object.as_deref())?;
    let store = payload::store(state.as_ref()).await?;
    for path in paths {
        match store.as_generic().delete(&path.into()).await {
            Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let finalized=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "envelope"=NULL,"objects"=NULL,"status"=CASE WHEN "status"='dispatched' THEN 'completed' WHEN "status" IN ('rejected','too_large') THEN "status" ELSE 'expired' END,"lease"=NULL WHERE "id"=$1 AND "lease"=$2"#,
        vec![row.id.clone().into(),lease.into()])).await?;
    if finalized.rows_affected() != 1 {
        return Ok(false);
    }
    if let Some(source) = &env.s3 {
        release_receipt(state, row, source).await;
    }
    Ok(true)
}

/// SES stores one object per message; the last cleaned recipient delivery deletes it.
async fn release_receipt(state: &AppState, row: &Delivery, source: &S3Source) {
    let config = &state.mail_automation;
    if config.bucket.as_deref() != Some(source.bucket.as_str())
        || !source.key.starts_with(&config.prefix)
    {
        tracing::warn!(delivery_id = %row.id, bucket = %source.bucket, "SES receipt object is outside the configured receipt location and was kept");
        return;
    }
    let pending = state
        .db
        .query_one_raw(statement(
            r#"SELECT "id" FROM "InboundMailDelivery" WHERE "messageId"=$1 AND "id"<>$2 AND "envelope" IS NOT NULL LIMIT 1"#,
            vec![row.message_id.clone().into(), row.id.clone().into()],
        ))
        .await;
    let result = match pending {
        Ok(Some(_)) => return,
        Ok(None) => payload::delete_s3(source).await,
        Err(error) => Err(error.into()),
    };
    if let Err(error) = result {
        tracing::warn!(delivery_id = %row.id, key = %source.key, %error, "SES receipt object was not deleted; the bucket lifecycle removes it");
    }
}

#[derive(Serialize, ToSchema)]
pub struct DispatchResponse {
    /// Deliveries that started an execution in this call.
    dispatched: usize,
    /// Expired deliveries whose files and receipt were removed in this call.
    cleaned: usize,
}

#[utoipa::path(
    post,
    path = "/sink/mail/dispatch",
    tag = "sink",
    description = "Mail ingress schedule: start executions for accepted mail, remove expired mail files and release retired data. Safe to call concurrently and repeatedly.",
    responses(
        (status = 200, description = "Work done in this call", body = DispatchResponse),
        (status = 401, description = "Missing, invalid or revoked ingress token"),
        (status = 403, description = "Token cannot receive email"),
        (status = 500, description = "The batch timed out; unfinished deliveries retry on the next call")
    ),
    security(("bearer_auth" = []))
)]
pub(crate) async fn dispatch(
    State(state): State<AppState>,
    IngressAuth(claims): IngressAuth,
) -> Result<Json<DispatchResponse>, ApiError> {
    // Leave unfinished claims leased when the request deadline expires. Every file is
    // tracked before upload, and another invocation can recover these claims later.
    flow_like_types::tokio::time::timeout(Duration::from_secs(60), dispatch_batch(&state, &claims))
        .await
        .map_err(|_| {
            ApiError::internal("Mail dispatch batch timed out; pending deliveries will retry")
        })?
}

async fn dispatch_batch(
    state: &AppState,
    claims: &SinkTriggerClaims,
) -> Result<Json<DispatchResponse>, ApiError> {
    // Slow execution must not consume every request's budget before expiry cleanup starts.
    let (dispatched, cleaned, ()) = flow_like_types::tokio::join!(
        dispatch_pending(state, claims),
        cleanup_expired(state, claims),
        housekeeping(state, claims),
    );
    Ok(Json(DispatchResponse {
        dispatched: dispatched?,
        cleaned: cleaned?,
    }))
}

async fn dispatch_pending(state: &AppState, claims: &SinkTriggerClaims) -> Result<usize, ApiError> {
    if !state.mail_automation.enabled {
        return Ok(0);
    }
    let timestamp = now();
    // Scope before LIMIT so other apps cannot starve this token's work.
    let rows = Delivery::find_by_statement(pending_deliveries(
        claims,
        timestamp,
        timestamp + min_dispatch_lifetime_ms(state),
    ))
    .all(&state.db)
    .await?;
    let mut dispatched = 0;
    for candidate in rows.into_iter().take(5) {
        let lease = flow_like_types::create_id();
        let timestamp = now();
        let claimed=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "status"='sending',"lease"=$1,"nextAttempt"=$2,"attempts"="attempts"+1 WHERE "id"=$3 AND "status" IN ('pending','sending') AND "nextAttempt" <= $4 AND "expiresAt" > $5"#,
            vec![lease.clone().into(),(timestamp+LEASE_MS).into(),candidate.id.clone().into(),timestamp.into(),(timestamp+min_dispatch_lifetime_ms(state)).into()])).await?;
        if claimed.rows_affected() != 1 {
            continue;
        }
        // Another attempt may have recorded files since the selection query.
        let row = delivery(state, &candidate.id).await?;
        // A started run keeps its files for a full TTL from dispatch.
        let extended = row.expires_at.max(timestamp + ttl_ms(state));
        let result = flow_like_types::tokio::time::timeout(
            WORK_TIMEOUT,
            deliver(state, &row, &lease, extended),
        )
        .await
        .unwrap_or_else(|_| Err(ApiError::internal("Mail dispatch timed out")));
        let settled = now();
        let (status, next_attempt, expires_at) = match result {
            Ok(true) => {
                dispatched += 1;
                ("dispatched", settled, extended)
            }
            Ok(false) => ("disabled", settled, row.expires_at),
            Err(error) if payload::is_too_large(&error) => {
                tracing::warn!(delivery_id=%row.id,app_id=%row.app_id,event_id=%row.event_id,max_bytes=max_bytes(state),"Inbound email exceeds the size limit and will not be dispatched");
                ("too_large", settled, row.expires_at.min(settled))
            }
            Err(error) => {
                tracing::warn!(delivery_id=%row.id,app_id=%row.app_id,event_id=%row.event_id,error=%error,"Inbound email dispatch will retry");
                let delay = (30_000_i64 * (1_i64 << row.attempts.saturating_sub(1).clamp(0, 7)))
                    .min(3_600_000);
                ("pending", settled + delay, row.expires_at)
            }
        };
        state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "status"=$1,"nextAttempt"=$2,"expiresAt"=$3,"lease"=NULL WHERE "id"=$4 AND "lease"=$5"#,
            vec![status.into(),next_attempt.into(),expires_at.into(),row.id.into(),lease.into()])).await?;
    }
    Ok(dispatched)
}

/// An attempt that timed out or lost its lease may have started a run without recording it.
fn may_hold_run(row: &Delivery, timestamp: i64) -> bool {
    timestamp < row.expires_at + RUN_RETENTION_CAP_MS
        && (row.status == "dispatched"
            || (row.attempts > 0
                && matches!(row.status.as_str(), "pending" | "sending" | "disabled")))
}

/// Every attempt creates a pending run before handing it to the executor, so an unrecorded
/// dispatch only protects a run that has actually started.
fn run_keeps_files(dispatched: bool, status: &RunStatus, started_at: Option<i64>) -> bool {
    !status.is_terminal() && (dispatched || started_at.is_some() || *status == RunStatus::Running)
}

/// Deliveries whose execution is still running keep their files, up to a hard cap.
async fn running(state: &AppState, rows: &[Delivery], timestamp: i64) -> HashSet<String> {
    let candidates = rows
        .iter()
        .filter(|row| may_hold_run(row, timestamp))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return HashSet::new();
    }
    let store = match crate::routes::execution::progress::get_state_store(state).await {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, "Cannot check inbound email executions; their files are kept");
            return candidates.iter().map(|row| row.id.clone()).collect();
        }
    };
    let lookups = candidates.iter().map(|row| {
        let store = store.clone();
        async move { (row, store.get_run_for_app(&row.run_id, &row.app_id).await) }
    });
    futures::future::join_all(lookups)
        .await
        .into_iter()
        .filter_map(|(row, run)| match run {
            Ok(Some(run))
                if run_keeps_files(row.status == "dispatched", &run.status, run.started_at) =>
            {
                Some(row.id.clone())
            }
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(delivery_id = %row.id, %error, "Cannot check an inbound email execution; its files are kept");
                Some(row.id.clone())
            }
        })
        .collect()
}

async fn cleanup_expired(state: &AppState, claims: &SinkTriggerClaims) -> Result<usize, ApiError> {
    let timestamp = now();
    let expired = Delivery::find_by_statement(expired_deliveries(claims, timestamp))
        .all(&state.db)
        .await?;
    let running = running(state, &expired, timestamp).await;
    let mut cleaned = 0;
    for candidate in expired {
        let timestamp = now();
        if running.contains(&candidate.id) {
            state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "nextAttempt"=$1 WHERE "id"=$2 AND "nextAttempt" <= $3"#,
                vec![(timestamp+RUN_RECHECK_MS).into(),candidate.id.clone().into(),timestamp.into()])).await?;
            continue;
        }
        let lease = flow_like_types::create_id();
        // Cleanup owns a lease before reading its latest file list or deleting any bytes.
        let claimed=state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "lease"=$1,"nextAttempt"=$2 WHERE "id"=$3 AND "expiresAt" <= $4 AND "envelope" IS NOT NULL AND "nextAttempt" <= $4"#,
            vec![lease.clone().into(),(timestamp+LEASE_MS).into(),candidate.id.clone().into(),timestamp.into()])).await?;
        if claimed.rows_affected() != 1 {
            continue;
        }
        let row = delivery(state, &candidate.id).await?;
        let result = flow_like_types::tokio::time::timeout(
            WORK_TIMEOUT,
            clean_delivery(state, &row, &lease),
        )
        .await
        .unwrap_or_else(|_| Err(ApiError::internal("Mail cleanup timed out")));
        match result {
            Ok(true) => cleaned += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(delivery_id=%row.id,error=%error,"Inbound email cleanup will retry");
                state.db.execute_raw(statement(r#"UPDATE "InboundMailDelivery" SET "lease"=NULL,"nextAttempt"=$1 WHERE "id"=$2 AND "lease"=$3"#,
                    vec![now().into(),row.id.into(),lease.into()])).await?;
            }
        }
    }
    Ok(cleaned)
}

async fn housekeeping_due(state: &AppState, claims: &SinkTriggerClaims) -> bool {
    let scope = claims.app_ids.as_ref().map_or_else(
        || "*".to_owned(),
        |ids| digest(&ids.iter().map(String::as_str).collect::<Vec<_>>()),
    );
    let Ok(cache) = state.cache.platform().await else {
        return true;
    };
    !matches!(
        cache
            .try_insert("mail-housekeeping", &scope, &now(), HOUSEKEEPING_INTERVAL)
            .await,
        Ok(Reservation::Held(_))
    )
}

/// Purges settled deduplication rows, expired send records and counters, and releases the
/// addresses of deleted events after their quarantine. Failures only delay the next pass.
async fn housekeeping(state: &AppState, claims: &SinkTriggerClaims) {
    if !housekeeping_due(state, claims).await {
        return;
    }
    let timestamp = now();
    let purges = [
        settled_deliveries(claims, timestamp - DEDUPE_RETENTION_MS),
        statement(
            &format!(
                r#"DELETE FROM "MailAutomationSend" WHERE "id" IN (SELECT "id" FROM "MailAutomationSend" WHERE "expiresAt" <= $1 LIMIT {PURGE_BATCH})"#
            ),
            vec![timestamp.into()],
        ),
        statement(
            &format!(
                r#"DELETE FROM "MailAutomationQuota" WHERE "id" IN (SELECT "id" FROM "MailAutomationQuota" WHERE "expiresAt" <= $1 LIMIT {PURGE_BATCH})"#
            ),
            vec![timestamp.into()],
        ),
    ];
    for purge in purges {
        if let Err(error) = state.db.execute_raw(purge).await {
            tracing::warn!(%error, "Inbound email housekeeping purge failed");
        }
    }
    if let Err(error) = address::quarantine_orphans(state, timestamp).await {
        tracing::warn!(%error, "Inbound email address quarantine failed");
    }
    if let Err(error) = address::release_quarantined(state, timestamp).await {
        tracing::warn!(%error, "Inbound email address release failed");
    }
}

/// The original message behind a reply, with its transport verdicts.
pub(crate) struct ReplySource {
    pub(crate) recipient: String,
    pub(crate) envelope_from: String,
    pub(crate) raw: Vec<u8>,
    pub(crate) authentication: Option<InboundEmailAuthentication>,
}

/// The caller must authorize the session before loading the message. The lookup still
/// binds every reference component to the persisted delivery so IDs cannot cross events.
pub(crate) async fn reply_source(
    state: &AppState,
    reference: &flow_like_catalog_core::MailMessageRef,
) -> Result<ReplySource, ApiError> {
    let row = Delivery::find_by_statement(statement(
        r#"SELECT * FROM "InboundMailDelivery" WHERE "id"=$1 AND "appId"=$2 AND "eventId"=$3"#,
        vec![
            reference.delivery_id.clone().into(),
            reference.session.app_id.clone().into(),
            reference.session.event_id.clone().into(),
        ],
    ))
    .one(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("The message does not belong to this email event"))?;
    if row.expires_at <= now() || row.envelope.is_none() {
        return Err(ApiError::gone("The original email has expired"));
    }
    if !matches!(row.status.as_str(), "sending" | "dispatched") {
        return Err(ApiError::conflict(
            "The original email is not available for replies",
        ));
    }
    let envelope = envelope(state, &row)?;
    let raw = payload::read(state, &envelope).await;
    // Storage fetches can overlap the expiry boundary.
    if row.expires_at <= now() {
        return Err(ApiError::gone("The original email has expired"));
    }
    Ok(ReplySource {
        recipient: envelope.recipient,
        envelope_from: envelope.envelope_from,
        raw: raw?,
        authentication: envelope.authentication,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(path: String) -> Envelope {
        Envelope {
            source: "postfix".into(),
            delivery_id: "receipt-1".into(),
            envelope_from: "sender@example.com".into(),
            recipient: "event@example.com".into(),
            s3: None,
            raw_object: Some(path),
            authentication: None,
            event_snapshot: None,
        }
    }

    fn claims(app_ids: Option<Vec<String>>) -> SinkTriggerClaims {
        SinkTriggerClaims {
            sub: "sink-trigger".into(),
            iss: "flow-like".into(),
            jti: Some("token".into()),
            sink_types: vec!["inbound_email".into()],
            app_ids,
            iat: 1,
            exp: None,
        }
    }

    #[test]
    fn retries_cannot_replace_content_or_share_another_targets_raw_file() {
        let first = raw_receipt_path("target-a", b"first mail");
        assert_ne!(first, raw_receipt_path("target-b", b"first mail"));
        let stored = receipt(first.clone());
        assert!(same_receipt(&stored, &receipt(first)));
        assert!(!same_receipt(
            &stored,
            &receipt(raw_receipt_path("target-a", b"replacement mail"))
        ));
        let mut forged = stored.clone();
        forged.envelope_from = "other@example.com".into();
        assert!(!same_receipt(&stored, &forged));
    }

    #[flow_like_types::tokio::test]
    async fn repeated_upload_keeps_first_encrypted_object() {
        let store = flow_like_storage::object_store::memory::InMemory::new();
        let path = raw_receipt_path("target-a", b"mail");
        put_immutable_raw(&store, &path, b"first ciphertext")
            .await
            .unwrap();
        put_immutable_raw(&store, &path, b"retry ciphertext")
            .await
            .unwrap();
        assert_eq!(
            store
                .get(&path.into())
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            &b"first ciphertext"[..]
        );
    }

    #[test]
    fn cleanup_keeps_files_written_under_previous_execution_actors() {
        let old = r#"["tmp/user/old/raw.eml","tmp/user/old/attachment"]"#;
        assert_eq!(
            merge_paths(
                Some(old),
                ["tmp/user/new/raw.eml", "tmp/user/old/attachment"]
            )
            .unwrap(),
            vec![
                "tmp/user/new/raw.eml",
                "tmp/user/old/attachment",
                "tmp/user/old/raw.eml"
            ]
        );
        assert!(merge_paths(Some("corrupt"), ["new"]).is_err());
    }

    #[test]
    fn scoped_batches_bind_app_ids_before_applying_limits() {
        let mut claims = claims(Some(vec!["app' OR TRUE --".into()]));
        for (query, placeholder, values) in [
            (pending_deliveries(&claims, 10, 20), "$3", 3),
            (expired_deliveries(&claims, 10), "$2", 2),
            (settled_deliveries(&claims, 10), "$2", 2),
        ] {
            assert!(
                query
                    .sql
                    .contains(&format!(r#"AND "appId" IN ({placeholder})"#)),
                "{}",
                query.sql
            );
            assert!(!query.sql.contains("OR TRUE"));
            assert_eq!(query.values.unwrap().0.len(), values);
        }
        assert!(
            settled_deliveries(&claims, 10)
                .sql
                .ends_with(&format!(r#"AND "appId" IN ($2) LIMIT {PURGE_BATCH})"#))
        );
        claims.app_ids = Some(vec![]);
        assert!(
            pending_deliveries(&claims, 10, 20)
                .sql
                .contains("AND FALSE")
        );
        claims.app_ids = None;
        assert!(
            !pending_deliveries(&claims, 10, 20)
                .sql
                .contains(r#""appId" IN"#)
        );
    }

    #[test]
    fn cleanup_only_selects_rows_that_still_hold_data_and_purges_only_settled_rows() {
        let claims = claims(None);
        let expired = expired_deliveries(&claims, 10).sql;
        assert!(expired.contains(r#""envelope" IS NOT NULL"#));
        assert!(expired.contains(r#""nextAttempt" <= $1"#));
        for status in ["'dispatched'", "'rejected'", "'too_large'", "'receiving'"] {
            assert!(expired.contains(status), "{status}");
        }
        assert!(!expired.contains("'completed'") && !expired.contains("'expired'"));
        let purge = settled_deliveries(&claims, 10).sql;
        assert!(purge.contains(r#""envelope" IS NULL"#));
        for status in ["'dispatched'", "'pending'", "'sending'", "'receiving'"] {
            assert!(!purge.contains(status), "{status}");
        }
    }

    fn expired_row(status: &str, attempts: i32) -> Delivery {
        Delivery {
            id: "delivery-1".into(),
            message_id: "message-1".into(),
            status: status.into(),
            app_id: "app-1".into(),
            event_id: "event-1".into(),
            envelope: Some("sealed".into()),
            objects: None,
            attempts,
            received_at: 0,
            expires_at: 1_000,
            run_id: "mail_delivery-1".into(),
        }
    }

    #[test]
    fn cleanup_checks_runs_of_attempts_that_never_recorded_their_dispatch() {
        let timestamp = 2_000;
        for status in ["dispatched", "sending", "pending", "disabled"] {
            assert!(may_hold_run(&expired_row(status, 1), timestamp), "{status}");
        }
        for status in ["sending", "pending", "disabled", "receiving"] {
            assert!(
                !may_hold_run(&expired_row(status, 0), timestamp),
                "{status}"
            );
        }
        for status in ["rejected", "too_large", "receiving"] {
            assert!(
                !may_hold_run(&expired_row(status, 3), timestamp),
                "{status}"
            );
        }
        assert!(!may_hold_run(
            &expired_row("sending", 1),
            1_000 + RUN_RETENTION_CAP_MS
        ));
    }

    #[test]
    fn unrecorded_dispatches_only_keep_files_for_runs_that_started() {
        assert!(run_keeps_files(false, &RunStatus::Running, None));
        assert!(run_keeps_files(false, &RunStatus::Pending, Some(1)));
        assert!(!run_keeps_files(false, &RunStatus::Pending, None));
        assert!(run_keeps_files(true, &RunStatus::Pending, None));
        for status in [
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Timeout,
        ] {
            assert!(!run_keeps_files(false, &status, Some(1)), "{status:?}");
            assert!(!run_keeps_files(true, &status, Some(1)), "{status:?}");
        }
    }

    #[test]
    fn short_ttls_keep_a_proportional_dispatch_margin() {
        assert_eq!(dispatch_margin_ms(300_000), 150_000);
        assert_eq!(dispatch_margin_ms(3_600_000), MIN_DISPATCH_LIFETIME_MS);
    }

    #[test]
    fn replies_are_refused_for_failed_provider_verdicts() {
        let verdict = |status: &str| {
            Some(InboundEmailVerdict {
                status: status.into(),
            })
        };
        assert_eq!(failed_verdict(None), None);
        let mut authentication = InboundEmailAuthentication {
            spf: verdict("FAIL"),
            dkim: verdict("FAIL"),
            spam: verdict("PASS"),
            virus: verdict("GRAY"),
            ..Default::default()
        };
        assert_eq!(failed_verdict(Some(&authentication)), None);
        authentication.dmarc = verdict("fail");
        assert_eq!(failed_verdict(Some(&authentication)), Some("DMARC"));
        authentication.dmarc = None;
        authentication.virus = verdict("FAIL");
        assert_eq!(failed_verdict(Some(&authentication)), Some("virus"));
        authentication.spam = verdict("FAIL");
        assert_eq!(failed_verdict(Some(&authentication)), Some("spam"));
    }

    #[test]
    fn delivery_identity_includes_source_and_target() {
        assert_ne!(digest(&["ses", "a", "b"]), digest(&["se", "sa", "b"]));
        assert_ne!(digest(&["ses", "a", "b"]), digest(&["ses", "a", "c"]));
    }
    #[test]
    fn rejects_mixed_transport_payloads() {
        let input = IngestRequest {
            source: "postfix".into(),
            delivery_id: "x".into(),
            envelope_from: "sender@example.com".into(),
            recipients: vec!["mail@example.com".into()],
            raw_mime_base64: Some("YWJj".into()),
            s3: Some(S3Source {
                bucket: "evil".into(),
                key: "raw/x".into(),
                version_id: None,
            }),
            authentication: None,
        };
        assert!(
            validate_source(
                &crate::runtime_config::mail::MailAutomationConfig::default(),
                &input
            )
            .is_err()
        );
    }
}
