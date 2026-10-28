//! Sink trigger utilities and HTTP endpoint
//!
//! Provides:
//! - `trigger_event` - Utility function for programmatic event triggering (Lambda, SQS, etc.)
//! - `http_trigger` - HTTP endpoint for HTTP sinks
//! - `telegram_trigger` - Telegram webhook endpoint with secret token & IP verification
//! - `service_trigger` - Service-to-service trigger for internal services (cron, discord bot, etc.)

use super::authority::{RunAsDenial, RunAsVerdict, run_as_authority};
use super::service::sink_types;
use crate::{
    entity::{
        event, event_sink, execution_run, pat,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        DispatchRequest, DispatchTrigger, ExecutionBackend, ExecutionJwtError, ExecutionJwtParams,
        TokenType, collect_generic_result, collect_generic_result_bytes, executor_response_bytes,
        is_jwt_configured, rejection, resolve_wasm_packages, sign_execution_jwt,
        sign_unattended_execution_jwt, stream_http_response, variant, wants_http_event_stream,
    },
    instances::schedules::{BOT_EVENT_TYPES, CLAIMED_EVENT_TYPES},
    permission::role_permission::RolePermissions,
    routes::app::{
        events::db::{extract_webhook_secret, get_event_from_db},
        prerun_shared::ensure_draft_board_snapshot,
    },
    state::AppState,
};
use axum::{
    Json,
    body::Body,
    extract::{ConnectInfo, FromRequest, Multipart, Path, Query, State},
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::{Path as StorePath, files::store::FlowLikeStore, object_store::PutPayload};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::{
    Bytes, Result as FlResult, anyhow, create_id, tokio, utils::constant_time_eq,
};
use ipnetwork::IpNetwork;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QuerySelect,
    prelude::DateTimeWithTimeZone,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use utoipa::ToSchema;

/// Telegram server IP ranges (CIDR notation)
/// Webhooks are sent from these ranges only
const TELEGRAM_IP_RANGES: &[&str] = &["149.154.160.0/20", "91.108.4.0/22"];

/// Check if an IP address is within Telegram's allowed ranges
fn is_telegram_ip(ip: &std::net::IpAddr) -> bool {
    // Only IPv4 is supported by Telegram webhooks
    let ipv4 = match ip {
        std::net::IpAddr::V4(v4) => v4,
        std::net::IpAddr::V6(_) => return false,
    };

    for range in TELEGRAM_IP_RANGES {
        if let Ok(network) = range.parse::<IpNetwork>()
            && network.contains(std::net::IpAddr::V4(*ipv4))
        {
            return true;
        }
    }
    false
}

/// Merge two JSON payloads: base payload from event config + override payload from request.
/// Request payload values take precedence over event config values.
/// If both are objects, they are deep-merged. Otherwise, request payload wins entirely.
fn merge_payloads(
    base: Option<serde_json::Value>,
    override_payload: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match (base, override_payload) {
        (None, None) => None,
        (Some(base), None) => Some(base),
        (None, Some(over)) => Some(over),
        (
            Some(serde_json::Value::Object(mut base_map)),
            Some(serde_json::Value::Object(over_map)),
        ) => {
            // Deep merge objects: override values take precedence
            for (key, value) in over_map {
                base_map.insert(key, value);
            }
            Some(serde_json::Value::Object(base_map))
        }
        // If either is not an object, override wins entirely
        (_, Some(over)) => Some(over),
    }
}

const HTTP_SINK_BODY_LIMIT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Default)]
struct ParsedHttpRequestPayload {
    payload: Option<serde_json::Value>,
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Option<String> {
    crate::middleware::jwt::viewer_authorization(headers)
        .map(normalize_authorization_token)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_authorization_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

fn http_auth_token_matches(headers: &HeaderMap, expected_token: &str) -> bool {
    let expected_token = normalize_authorization_token(expected_token);
    authorization_token_from_headers(headers).is_some_and(|provided_token| {
        constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes())
    })
}

fn telegram_secret_matches(
    headers: &HeaderMap,
    query_secret: Option<&str>,
    expected_secret: &str,
) -> bool {
    let provided_secret = headers
        .get("X-Telegram-Bot-Api-Secret-Token")
        .and_then(|value| value.to_str().ok())
        .or(query_secret);

    provided_secret.is_some_and(|provided_secret| {
        constant_time_eq(provided_secret.as_bytes(), expected_secret.as_bytes())
    })
}

/// What a bot door verifies: the Discord public key or Telegram secret token stored on the
/// sink, else the one in the event config. `None` fails the door closed.
fn door_secret(sink: &event_sink::Model, config: &[u8]) -> Option<String> {
    sink.webhook_secret
        .as_deref()
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| extract_webhook_secret(config))
}

/// Event types the HTTP door serves.
const HTTP_DOOR_EVENT_TYPES: [&str; 3] = ["api", "http", "webhook"];

/// The active sink the HTTP door serves at `path` for `method`, or with `None` one saved
/// before HTTP methods were stored. Other events keep their UI route in the same columns,
/// and a sink saved before its event changed type may still hold the old method, so only
/// an HTTP-typed event is reachable here.
fn http_route_sink(
    app_id: &str,
    path: &str,
    method: Option<&str>,
) -> sea_orm::Select<event_sink::Entity> {
    let method = match method {
        Some(method) => event_sink::Column::Method.eq(method),
        None => event_sink::Column::Method.is_null(),
    };
    event_sink::Entity::find()
        .inner_join(event::Entity)
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::Path.eq(path))
        .filter(method)
        .filter(event_sink::Column::Active.eq(true))
        .filter(event::Column::EventType.is_in(HTTP_DOOR_EVENT_TYPES))
}

pub(super) fn parse_pat_token(value: &str) -> Option<(&str, &str)> {
    let pat_parts = value.trim().strip_prefix("pat_")?;
    let (pat_id, secret) = pat_parts.split_once('.')?;
    if pat_id.is_empty() || secret.is_empty() || secret.contains('.') {
        return None;
    }
    Some((pat_id, secret))
}

/// A sink fire refused because the user it runs as may no longer run it.
pub(crate) const SINK_RUN_AS_REFUSED: &str = "SINK_RUN_AS_REFUSED";
/// A sink fire whose run-as user could not be checked; a retry may succeed.
pub(crate) const SINK_RUN_AS_UNAVAILABLE: &str = "SINK_RUN_AS_UNAVAILABLE";
pub(super) const UNREADABLE_PAT: &str =
    "The access token this event runs with cannot be read on this server";

pub(crate) fn is_run_as_refusal(error: &ApiError) -> bool {
    error.public_code() == SINK_RUN_AS_REFUSED
}

/// Restores a run-as answer that crossed an `anyhow` boundary; any other error is internal.
pub(crate) fn sink_dispatch_error(error: flow_like_types::Error) -> ApiError {
    match error.downcast_ref::<ApiError>() {
        Some(answer)
            if matches!(
                answer.public_code(),
                SINK_RUN_AS_REFUSED | SINK_RUN_AS_UNAVAILABLE
            ) =>
        {
            answer.clone()
        }
        _ => ApiError::internal_error(error),
    }
}

/// The answer to a fire refused for `denial`, the same at every door.
pub(super) fn run_as_refused(denial: RunAsDenial) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        SINK_RUN_AS_REFUSED,
        denial.public_message(),
    )
}

/// A run-as refusal that paused nothing because the project is not active. Its sinks stay on
/// and run again once the project is active.
pub(super) fn is_inactive_project_refusal(error: &ApiError) -> bool {
    is_run_as_refusal(error)
        && error.public_message() == Some(RunAsDenial::AppInactive.public_message())
}

/// What the run-as check of a sink found. Nothing is written to find it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SinkRunAs {
    /// The user the run executes as; `None` for a sink without one, which runs as
    /// `sink:{id}`.
    Allowed(Option<String>),
    Denied {
        user_id: Option<String>,
        denial: RunAsDenial,
    },
    Transient {
        user_id: Option<String>,
        reason: &'static str,
    },
}

impl SinkRunAs {
    fn judged(verdict: RunAsVerdict, user_id: Option<&str>) -> Self {
        let user_id = user_id.map(str::to_owned);
        match verdict {
            RunAsVerdict::Allowed(_) => Self::Allowed(user_id),
            RunAsVerdict::Denied(denial) => Self::Denied { user_id, denial },
            RunAsVerdict::Transient(reason) => Self::Transient { user_id, reason },
        }
    }
}

/// Why a legacy sink's stored PAT names nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatRefusal {
    /// Malformed, unknown, expired, revoked or below Read & Write scope.
    Invalid,
    /// The ciphertext does not decrypt with this server's key: a configuration fault.
    Unreadable,
}

/// The user a sink runs as and whether that user may trigger events now: the app is
/// ACTIVE and the account is ACTIVE, a member and allowed ExecuteEvents. A sink saved
/// before run-as identities runs as the owner of its stored PAT for as long as that PAT
/// validates; it never falls back to `sink:{id}`. Side-effect free.
pub(crate) async fn sink_run_as_authority(
    state: &AppState,
    sink: &event_sink::Model,
) -> Result<SinkRunAs, ApiError> {
    let execute = RolePermissions::ExecuteEvents;
    let run_as = match (
        sink.run_as_user_id.as_deref(),
        sink.pat_encrypted.as_deref(),
    ) {
        (Some(user_id), _) => Some(user_id.to_owned()),
        (None, Some(ciphertext)) => match legacy_pat_owner(state, ciphertext).await? {
            Ok(owner) => Some(owner),
            Err(refusal) => {
                let app = run_as_authority(&state.db, &sink.app_id, None, execute).await?;
                return Ok(legacy_refusal(app, refusal));
            }
        },
        (None, None) => None,
    };
    let verdict = run_as_authority(&state.db, &sink.app_id, run_as.as_deref(), execute).await?;
    Ok(SinkRunAs::judged(verdict, run_as.as_deref()))
}

/// A stored PAT that names nobody refuses the run, unless the app's own state already
/// does: an inactive project wins over a bad token.
fn legacy_refusal(app: RunAsVerdict, refusal: PatRefusal) -> SinkRunAs {
    match (app, refusal) {
        (RunAsVerdict::Allowed(_), PatRefusal::Invalid) => SinkRunAs::Denied {
            user_id: None,
            denial: RunAsDenial::TokenInvalid,
        },
        (RunAsVerdict::Allowed(_), PatRefusal::Unreadable) => SinkRunAs::Transient {
            user_id: None,
            reason: UNREADABLE_PAT,
        },
        (verdict, _) => SinkRunAs::judged(verdict, None),
    }
}

/// The user a remote sink runs as, checked on every fire by [`sink_run_as_authority`].
/// A refusal is recorded on the event and, unless the project is merely inactive, pauses
/// the sink and notifies the app's admins. It never answers 401: a refusal is 403
/// `SINK_RUN_AS_REFUSED`, a check that failed on the server is 503
/// `SINK_RUN_AS_UNAVAILABLE`.
pub(crate) async fn resolve_sink_run_as(
    state: &AppState,
    sink: &event_sink::Model,
) -> Result<Option<String>, ApiError> {
    resolve_sink_run_as_for_run(state, sink, None).await
}

/// [`resolve_sink_run_as`] for a durable transport that already holds `run_id`: a refusal
/// finalizes that run instead of recording another.
pub(crate) async fn resolve_sink_run_as_for_run(
    state: &AppState,
    sink: &event_sink::Model,
    run_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    match sink_run_as_authority(state, sink).await? {
        SinkRunAs::Allowed(user_id) => Ok(user_id),
        SinkRunAs::Denied { user_id, denial } => {
            Err(refuse_sink_run(state, sink, user_id, denial, run_id).await)
        }
        SinkRunAs::Transient { user_id, reason } => {
            Err(sink_run_as_unavailable(state, sink, user_id, reason).await)
        }
    }
}

/// The refusal lands on the transport's `run_id` only while no run went out under it. A
/// retry of a delivery whose run was already dispatched leaves that run, its payments and
/// its logs alone, and the refusal is recorded on a run of its own.
async fn refuse_sink_run(
    state: &AppState,
    sink: &event_sink::Model,
    user_id: Option<String>,
    denial: RunAsDenial,
    run_id: Option<&str>,
) -> ApiError {
    tracing::warn!(
        sink_id = %sink.id,
        event_id = %sink.event_id,
        app_id = %sink.app_id,
        user_id = ?user_id,
        reason = denial.reason(),
        "Refusing a sink fire: the user it runs as may no longer run it"
    );
    if denial.pauses() {
        let released = match sink.run_as_user_id.as_deref() {
            Some(run_as) => {
                super::service::release_member_sinks(state, Some(&sink.app_id), run_as, denial)
                    .await
                    .map(|released| released > 0)
            }
            None => {
                super::service::release_legacy_sink(state, sink, user_id.as_deref(), denial).await
            }
        };
        if let Err(error) = released {
            tracing::error!(sink_id = %sink.id, event_id = %sink.event_id, %error, "Failed to pause a sink whose run-as user may no longer run it");
        }
    }
    let run_id = rejection::undispatched_run(state, run_id).await;
    record_trigger_rejection(
        state,
        &sink.event_id,
        rejection::RejectionStage::Permission,
        denial.reason(),
        None,
        run_id,
        user_id,
    )
    .await;
    run_as_refused(denial)
}

/// A server-side fault, so nothing is paused, and the transport's run id is left for its
/// retry.
async fn sink_run_as_unavailable(
    state: &AppState,
    sink: &event_sink::Model,
    user_id: Option<String>,
    reason: &str,
) -> ApiError {
    tracing::error!(
        sink_id = %sink.id,
        event_id = %sink.event_id,
        app_id = %sink.app_id,
        user_id = ?user_id,
        reason,
        "Cannot check the user a sink runs as; the fire can be retried"
    );
    record_trigger_rejection(
        state,
        &sink.event_id,
        rejection::RejectionStage::Setup,
        reason,
        None,
        None,
        user_id,
    )
    .await;
    retry_later()
}

fn retry_later() -> ApiError {
    ApiError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        SINK_RUN_AS_UNAVAILABLE,
        "This event cannot start right now. Try again later.",
    )
}

/// Whose run a sink serves, which decides whether a sink that is off stops it and what the run
/// takes from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SinkUse {
    /// A run as the sink's run-as user: a fire nobody triggered, an anonymous hosted visit, a
    /// public call without a login. Any pause stops it.
    RunAs,
    /// A run a Flow-Like person triggered, which acts as that person. A release's pause stops
    /// only the runs that use the released user, so it does not stop this one; a sink a person
    /// switched off does.
    Person,
}

impl SinkUse {
    pub(crate) fn admits(self, sink: &event_sink::Model) -> bool {
        match self {
            Self::RunAs => sink.active,
            Self::Person => sink.active || sink.run_as_released_at.is_some(),
        }
    }

    /// Whether the run takes the OAuth grants saved on `sink`, the event's saved sign-ins. A
    /// person's run takes them only while the sink runs as someone, whose grants they are; the
    /// grants of a legacy sink belong to nobody the hub can resolve them for.
    pub(crate) fn takes_saved_grants(self, sink: &event_sink::Model) -> bool {
        sink.oauth_tokens_encrypted.is_some()
            && (self == Self::RunAs || sink.run_as_user_id.is_some())
    }

    /// Whether the run takes anything from `sink`, so it must first fix what it runs: a run as
    /// its run-as user takes its grants and profile, a person's run only its saved sign-ins.
    pub(crate) fn borrows_from(self, sink: &event_sink::Model) -> bool {
        self == Self::RunAs || self.takes_saved_grants(sink)
    }
}

/// What a fire dispatches with once its target is fixed: the sink as read after that, and the
/// exact draft a Latest target runs.
pub(crate) struct PinnedFire {
    pub(crate) sink: event_sink::Model,
    pub(crate) board_etag: Option<String>,
}

/// [`pin_fire_for`] of a run as the sink's run-as user.
pub(crate) async fn pin_fire(
    state: &AppState,
    sink: event_sink::Model,
    target: &variant::ResolvedTarget,
) -> Result<PinnedFire, ApiError> {
    pin_fire_for(state, sink, target, SinkUse::RunAs).await
}

/// Fixes the draft a Latest target runs, then reads the sink again. A person's board edit makes
/// them the run-as user before it saves, so a draft saved after this fire read `sink` comes with
/// another run-as identity, and the fire is refused as retryable instead of running that draft as
/// the person the edit replaced, or with that person's OAuth grants. A pinned version is never
/// rewritten by an edit.
pub(crate) async fn pin_fire_for(
    state: &AppState,
    sink: event_sink::Model,
    target: &variant::ResolvedTarget,
    sink_use: SinkUse,
) -> Result<PinnedFire, ApiError> {
    if target.board_version.is_some() {
        return Ok(PinnedFire {
            sink,
            board_etag: None,
        });
    }
    let draft = state
        .master_board_shared(&sink.app_id, &target.board_id, state, None)
        .await
        .map_err(ApiError::internal_error)?;
    if let Err(error) =
        ensure_draft_board_snapshot(state, &sink.app_id, &target.board_id, &draft).await
    {
        // A 400 here means the draft was saved again while its snapshot was copied.
        return Err(if error.status() == StatusCode::BAD_REQUEST {
            fire_overtaken(&sink)
        } else {
            error
        });
    }
    Ok(PinnedFire {
        sink: recheck_fire_for(state, &sink, sink_use).await?,
        board_etag: Some(draft.e_tag.trim().to_owned()),
    })
}

/// [`recheck_fire_for`] of a run as the sink's run-as user.
pub(crate) async fn recheck_fire(
    state: &AppState,
    sink: &event_sink::Model,
) -> Result<event_sink::Model, ApiError> {
    recheck_fire_for(state, sink, SinkUse::RunAs).await
}

/// Reads the sink again once what the fire runs is fixed, and refuses the fire as retryable
/// unless the sink still serves the run and runs as the identity the fire took from `sink`.
pub(crate) async fn recheck_fire_for(
    state: &AppState,
    sink: &event_sink::Model,
    sink_use: SinkUse,
) -> Result<event_sink::Model, ApiError> {
    match event_sink::Entity::find_by_id(sink.id.clone())
        .one(&state.db)
        .await?
    {
        Some(current) if runs_as_read(sink, &current, sink_use) => Ok(current),
        _ => Err(fire_overtaken(sink)),
    }
}

/// Whether the sink read again still serves the run and runs as the identity the fire took
/// from `read`.
fn runs_as_read(read: &event_sink::Model, current: &event_sink::Model, sink_use: SinkUse) -> bool {
    sink_use.admits(current)
        && current.run_as_user_id == read.run_as_user_id
        && current.pat_encrypted == read.pat_encrypted
}

fn fire_overtaken(sink: &event_sink::Model) -> ApiError {
    tracing::warn!(
        sink_id = %sink.id,
        event_id = %sink.event_id,
        app_id = %sink.app_id,
        "The sink or its draft changed while it fired; the fire can be retried"
    );
    retry_later()
}

pub(super) type PatRow = (String, Option<DateTimeWithTimeZone>, i64);

/// The owner a legacy sink's stored PAT still names. The owner's authority is checked
/// by the caller.
async fn legacy_pat_owner(
    state: &AppState,
    ciphertext: &str,
) -> Result<Result<String, PatRefusal>, ApiError> {
    let token = crate::routes::app::events::db::decrypt_token(ciphertext, &state.encryption_key);
    let (pat_id, secret) = match stored_pat_parts(token.as_deref()) {
        Ok(parts) => parts,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let row = pat::Entity::find()
        .select_only()
        .column(pat::Column::UserId)
        .column(pat::Column::ValidUntil)
        .column(pat::Column::Permissions)
        .filter(pat::Column::Id.eq(pat_id))
        .filter(pat::Column::Key.eq(blake3::hash(secret.as_bytes()).to_hex().to_string()))
        .into_tuple::<PatRow>()
        .one(&state.db)
        .await?;
    Ok(pat_row_owner(row, chrono::Utc::now().fixed_offset()))
}

pub(super) fn stored_pat_parts(token: Option<&str>) -> Result<(&str, &str), PatRefusal> {
    parse_pat_token(token.ok_or(PatRefusal::Unreadable)?).ok_or(PatRefusal::Invalid)
}

/// The owner of the stored PAT's row, while it is unexpired and allows Read & Write or
/// Admin access.
pub(super) fn pat_row_owner(
    row: Option<PatRow>,
    now: DateTimeWithTimeZone,
) -> Result<String, PatRefusal> {
    let (user_id, valid_until, permissions) = row.ok_or(PatRefusal::Invalid)?;
    let expired = valid_until.is_some_and(|valid_until| valid_until < now);
    if expired || !sink_pat_may_run(permissions) {
        return Err(PatRefusal::Invalid);
    }
    Ok(user_id)
}

fn sink_pat_may_run(bits: i64) -> bool {
    use crate::permission::pat_permission::{PatPermission, has_pat_permission};
    PatPermission::from_bits(bits)
        .is_some_and(|permission| has_pat_permission(&permission, PatPermission::ReadWrite))
}

#[cfg(test)]
mod legacy_pat_tests {
    use super::*;
    use crate::permission::pat_permission::PatPermission;

    #[test]
    fn stored_tokens_require_write_access() {
        for permission in [PatPermission::ReadWrite, PatPermission::Admin] {
            assert!(sink_pat_may_run(permission.bits()));
        }
        for bits in [
            0,
            -1,
            PatPermission::ReadOnly.bits(),
            3,
            8,
            16,
            32,
            1 | (1 << 62),
        ] {
            assert!(!sink_pat_may_run(bits), "{bits}");
        }
    }

    #[test]
    fn a_stored_token_is_unreadable_or_invalid_before_the_database_is_asked() {
        assert_eq!(stored_pat_parts(None), Err(PatRefusal::Unreadable));
        for malformed in [
            "",
            "pat_",
            "pat_id",
            "pat_.secret",
            "pat_id.",
            "id.secret",
            "pat_id.se.cret",
        ] {
            assert_eq!(
                stored_pat_parts(Some(malformed)),
                Err(PatRefusal::Invalid),
                "{malformed}"
            );
        }
        assert_eq!(
            stored_pat_parts(Some("pat_id.secret")),
            Ok(("id", "secret"))
        );
    }

    #[test]
    fn only_an_unexpired_token_with_write_scope_names_its_owner() {
        let now = chrono::Utc::now().fixed_offset();
        let row = |valid_until: Option<DateTimeWithTimeZone>, permission: i64| {
            Some(("owner".to_owned(), valid_until, permission))
        };
        let write = PatPermission::ReadWrite.bits();
        assert_eq!(pat_row_owner(row(None, write), now), Ok("owner".to_owned()));
        assert_eq!(
            pat_row_owner(row(Some(now + chrono::Duration::days(1)), write), now),
            Ok("owner".to_owned())
        );
        for (case, refused) in [
            ("revoked or wrong secret", None),
            (
                "expired",
                row(Some(now - chrono::Duration::seconds(1)), write),
            ),
            ("read only", row(None, PatPermission::ReadOnly.bits())),
            ("legacy bits", row(None, 1 | (1 << 62))),
        ] {
            assert_eq!(
                pat_row_owner(refused, now),
                Err(PatRefusal::Invalid),
                "{case}"
            );
        }
    }

    /// A legacy row runs as its token's owner or not at all: a bad token is never answered
    /// with `sink:{id}`, an unreadable one is a server fault that pauses nothing, and an
    /// inactive project wins over both.
    #[test]
    fn a_token_that_names_nobody_never_runs_the_sink_without_a_user() {
        let active = RunAsVerdict::Allowed(RolePermissions::empty());
        let inactive = RunAsVerdict::Denied(RunAsDenial::AppInactive);
        for (app, refusal, expected) in [
            (
                active,
                PatRefusal::Invalid,
                SinkRunAs::Denied {
                    user_id: None,
                    denial: RunAsDenial::TokenInvalid,
                },
            ),
            (
                active,
                PatRefusal::Unreadable,
                SinkRunAs::Transient {
                    user_id: None,
                    reason: UNREADABLE_PAT,
                },
            ),
            (
                inactive,
                PatRefusal::Invalid,
                SinkRunAs::Denied {
                    user_id: None,
                    denial: RunAsDenial::AppInactive,
                },
            ),
            (
                inactive,
                PatRefusal::Unreadable,
                SinkRunAs::Denied {
                    user_id: None,
                    denial: RunAsDenial::AppInactive,
                },
            ),
        ] {
            let outcome = legacy_refusal(app, refusal);
            assert_ne!(outcome, SinkRunAs::Allowed(None), "{app:?} {refusal:?}");
            assert_eq!(outcome, expected, "{app:?} {refusal:?}");
        }
    }

    #[test]
    fn a_verdict_keeps_the_user_it_was_reached_for() {
        assert_eq!(
            SinkRunAs::judged(
                RunAsVerdict::Allowed(RolePermissions::ExecuteEvents),
                Some("ada")
            ),
            SinkRunAs::Allowed(Some("ada".to_owned()))
        );
        assert_eq!(
            SinkRunAs::judged(RunAsVerdict::Allowed(RolePermissions::empty()), None),
            SinkRunAs::Allowed(None)
        );
        assert_eq!(
            SinkRunAs::judged(RunAsVerdict::Denied(RunAsDenial::NotMember), Some("ada")),
            SinkRunAs::Denied {
                user_id: Some("ada".to_owned()),
                denial: RunAsDenial::NotMember
            }
        );
        assert_eq!(
            SinkRunAs::judged(RunAsVerdict::Transient("bits"), Some("ada")),
            SinkRunAs::Transient {
                user_id: Some("ada".to_owned()),
                reason: "bits"
            }
        );
    }

    #[test]
    fn run_as_answers_survive_an_anyhow_boundary_and_nothing_else_does() {
        let refused = ApiError::coded(
            StatusCode::FORBIDDEN,
            SINK_RUN_AS_REFUSED,
            "This event is paused",
        );
        let unavailable = ApiError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            SINK_RUN_AS_UNAVAILABLE,
            "later",
        );
        for answer in [refused, unavailable] {
            let restored = sink_dispatch_error(flow_like_types::Error::from(answer.clone()));
            assert_eq!(
                (restored.status(), restored.public_code()),
                (answer.status(), answer.public_code())
            );
        }
        assert!(is_run_as_refusal(&sink_dispatch_error(
            flow_like_types::Error::from(ApiError::coded(
                StatusCode::FORBIDDEN,
                SINK_RUN_AS_REFUSED,
                "This event is paused",
            ))
        )));
        let other = sink_dispatch_error(flow_like_types::Error::from(ApiError::forbidden("no")));
        assert_eq!(other.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Every door answers each refusal with 403 `SINK_RUN_AS_REFUSED`; only the one that paused
    /// nothing, an inactive project, can be told apart from the others.
    #[test]
    fn only_an_inactive_project_refusal_is_told_apart() {
        use RunAsDenial::{AccountInactive, AppInactive, CannotTrigger, NotMember, TokenInvalid};
        for denial in [
            AppInactive,
            AccountInactive,
            NotMember,
            CannotTrigger,
            TokenInvalid,
        ] {
            let refusal = run_as_refused(denial);
            assert_eq!(
                is_inactive_project_refusal(&refusal),
                denial == AppInactive,
                "{denial:?}"
            );
            assert_eq!(
                is_inactive_project_refusal(&refusal),
                !denial.pauses(),
                "{denial:?}"
            );
            let restored = sink_dispatch_error(flow_like_types::Error::from(refusal));
            assert_eq!(
                (restored.status(), restored.public_code()),
                (StatusCode::FORBIDDEN, SINK_RUN_AS_REFUSED),
                "{denial:?}"
            );
        }
        let not_available = AppInactive.public_message();
        for other in [
            ApiError::forbidden(not_available),
            ApiError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                SINK_RUN_AS_UNAVAILABLE,
                not_available,
            ),
        ] {
            assert!(!is_inactive_project_refusal(&other), "{other:?}");
        }
    }
}

fn is_multipart_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
        })
        .unwrap_or(false)
}

fn is_urlencoded_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value.split(';').next().is_some_and(|mime| {
                mime.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
        .unwrap_or(false)
}

fn decode_form_component(value: &str) -> String {
    let value = value.replace('+', " ");
    urlencoding::decode(&value)
        .unwrap_or(std::borrow::Cow::Borrowed(value.as_str()))
        .into_owned()
}

fn normalize_form_key(raw_key: &str, fallback: &str) -> (String, bool) {
    let decoded = decode_form_component(raw_key);
    let trimmed = decoded.trim();
    let key = if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    };

    if let Some(stripped) = key.strip_suffix("[]") {
        let stripped = stripped.trim();
        return (
            if stripped.is_empty() {
                fallback.to_string()
            } else {
                stripped.to_string()
            },
            true,
        );
    }

    (key, false)
}

fn insert_payload_value(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: String,
    value: serde_json::Value,
    force_array: bool,
) {
    match map.get_mut(&key) {
        Some(serde_json::Value::Array(values)) => values.push(value),
        Some(existing) => {
            let previous = std::mem::replace(existing, serde_json::Value::Null);
            *existing = serde_json::Value::Array(vec![previous, value]);
        }
        None if force_array => {
            map.insert(key, serde_json::Value::Array(vec![value]));
        }
        None => {
            map.insert(key, value);
        }
    }
}

fn parse_form_encoded_payload(input: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    for pair in input.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let (key, force_array) = normalize_form_key(raw_key, "value");
        let value = serde_json::Value::String(decode_form_component(raw_value));
        insert_payload_value(&mut map, key, value, force_array);
    }

    map
}

fn merge_query_and_body(
    query: serde_json::Map<String, serde_json::Value>,
    body: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match (query.is_empty(), body) {
        (true, None) => None,
        (false, None) => Some(serde_json::Value::Object(query)),
        (true, Some(body)) => Some(body),
        (false, Some(serde_json::Value::Object(mut body_map))) => {
            let mut merged = query;
            merged.append(&mut body_map);
            Some(serde_json::Value::Object(merged))
        }
        (false, Some(body)) => {
            let mut merged = query;
            merged.insert("_body".to_string(), body);
            Some(serde_json::Value::Object(merged))
        }
    }
}

fn sanitize_request_file_name(filename: Option<&str>, fallback_index: usize) -> String {
    let raw = filename
        .and_then(|name| name.rsplit(['/', '\\']).next())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("file");

    let mut sanitized = String::with_capacity(raw.len().min(120));
    for ch in raw.chars().take(120) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    let sanitized = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    if sanitized.is_empty() {
        format!("file-{fallback_index}")
    } else {
        sanitized.to_string()
    }
}

fn sanitize_store_path_segment(value: &str, fallback: &str) -> String {
    crate::credentials::storage_path_segment(value, fallback)
}

fn flow_path_value(path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "store_ref": REQUEST_FILES_STORE_REF,
        "cache_store_ref": null
    })
}

async fn parse_multipart_payload(
    request: Request<Body>,
    query: serde_json::Map<String, serde_json::Value>,
    body_limit: usize,
    file_store: FlowLikeStore,
    file_path_prefix: String,
) -> Result<ParsedHttpRequestPayload, ApiError> {
    let mut multipart = Multipart::from_request(request, &()).await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse multipart HTTP sink payload");
        ApiError::bad_request("Invalid multipart/form-data request")
    })?;

    let mut body_map = serde_json::Map::new();
    let mut total_bytes = 0usize;
    let mut file_count = 0usize;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to read multipart field");
        ApiError::bad_request("Invalid multipart/form-data field")
    })? {
        let raw_name = field.name().unwrap_or("file").to_string();
        let file_name = field.file_name().map(ToOwned::to_owned);
        let (key, force_array) = normalize_form_key(&raw_name, "file");
        let bytes = field.bytes().await.map_err(|e| {
            tracing::warn!(error = %e, "Failed to read multipart field bytes");
            ApiError::bad_request("Invalid multipart/form-data field")
        })?;

        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > body_limit {
            return Err(ApiError::bad_request("Request body exceeds size limit"));
        }

        if file_name.is_some() {
            file_count += 1;
            let file_index = file_count;
            let sanitized_name = sanitize_request_file_name(file_name.as_deref(), file_index);
            let path = format!("{file_path_prefix}/{file_index:04}-{sanitized_name}");
            file_store
                .as_generic()
                .put(
                    &StorePath::from(path.clone()),
                    PutPayload::from_bytes(Bytes::copy_from_slice(&bytes)),
                )
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, path = %path, "Failed to stage multipart file");
                    ApiError::internal_error(anyhow!("Failed to stage multipart file"))
                })?;
            insert_payload_value(&mut body_map, key, flow_path_value(&path), force_array);
        } else {
            let value = serde_json::Value::String(String::from_utf8_lossy(&bytes).to_string());
            insert_payload_value(&mut body_map, key, value, force_array);
        }
    }

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, Some(serde_json::Value::Object(body_map))),
    })
}

async fn parse_http_request_payload(
    request: Request<Body>,
    body_limit: usize,
    file_store: Option<FlowLikeStore>,
    file_path_prefix: Option<String>,
) -> Result<ParsedHttpRequestPayload, ApiError> {
    let (parts, body) = request.into_parts();
    let query = parts
        .uri
        .query()
        .map(parse_form_encoded_payload)
        .unwrap_or_default();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    if is_multipart_content_type(content_type.as_deref()) {
        let file_store = file_store.ok_or_else(|| {
            ApiError::internal_error(anyhow!("Temporary file store is not configured"))
        })?;
        let file_path_prefix = file_path_prefix.ok_or_else(|| {
            ApiError::internal_error(anyhow!("Temporary file path is not configured"))
        })?;
        return parse_multipart_payload(
            Request::from_parts(parts, body),
            query,
            body_limit,
            file_store,
            file_path_prefix,
        )
        .await;
    }

    let body_bytes = axum::body::to_bytes(body, body_limit).await.map_err(|e| {
        tracing::error!("Failed to read body: {}", e);
        ApiError::bad_request("Failed to read request body")
    })?;

    let body_payload = if body_bytes.is_empty() {
        None
    } else if is_urlencoded_content_type(content_type.as_deref()) {
        let body_str = std::str::from_utf8(&body_bytes)
            .map_err(|_| ApiError::bad_request("Invalid form body"))?;
        Some(serde_json::Value::Object(parse_form_encoded_payload(
            body_str,
        )))
    } else {
        match serde_json::from_slice(&body_bytes) {
            Ok(value) => Some(value),
            Err(_) => Some(serde_json::Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            )),
        }
    };

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, body_payload),
    })
}

/// OAuth sign-ins of a run, keyed by provider id.
pub(crate) type OAuthTokens = std::collections::HashMap<String, serde_json::Value>;

/// The OAuth grants saved on the sink, as stored.
fn saved_grants(state: &AppState, sink: &event_sink::Model) -> Option<OAuthTokens> {
    use crate::routes::app::events::db::decrypt_token;
    sink.oauth_tokens_encrypted
        .as_deref()
        .and_then(|encrypted| decrypt_token(encrypted, &state.encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok())
}

/// The OAuth sign-ins a run takes: the event's saved sign-ins, which are the grants of the
/// sink's run-as user, merged with the sign-ins the person who triggered the run `supplied`,
/// already resolved as theirs. A person's sign-in wins for its provider, and only the saved
/// grants of the other providers are resolved, so a saved grant the person replaced cannot
/// fail their run. A run nobody triggered supplies none and takes every saved grant. Which
/// saved grants a run may take at all follows [`SinkUse::takes_saved_grants`].
pub(crate) async fn run_oauth_tokens(
    state: &AppState,
    sink: Option<&event_sink::Model>,
    sink_use: SinkUse,
    supplied: Option<OAuthTokens>,
) -> Result<Option<OAuthTokens>, ApiError> {
    let Some((sink, saved)) = sink
        .filter(|sink| sink_use.takes_saved_grants(sink))
        .and_then(|sink| saved_grants(state, sink).map(|saved| (sink, saved)))
    else {
        return Ok(supplied);
    };
    let supplied = supplied.unwrap_or_default();
    let saved = refresh_sink_grants(state, sink, saved, |provider| {
        !supplied.contains_key(provider)
    })
    .await?;
    Ok(Some(person_over_event(saved, supplied)))
}

/// A person's sign-in for each provider they supplied one for, else the event's saved grant.
fn person_over_event(mut saved: OAuthTokens, supplied: OAuthTokens) -> OAuthTokens {
    saved.retain(|provider, _| !supplied.contains_key(provider));
    saved.extend(supplied);
    saved
}

/// Resolve each connection through its canonical grant before dispatch.
/// Stored snapshots can lag behind refreshes by another browser or event sink.
/// The grants belong to the sink's run-as user; a sink without one has none to use.
pub(crate) async fn maybe_refresh_oauth_tokens(
    state: &AppState,
    sink: &event_sink::Model,
    tokens: OAuthTokens,
) -> Result<OAuthTokens, ApiError> {
    refresh_sink_grants(state, sink, tokens, |_| true).await
}

/// [`maybe_refresh_oauth_tokens`] for the providers `wanted` keeps. The others stay saved as
/// they are, and only the wanted grants are returned.
async fn refresh_sink_grants(
    state: &AppState,
    sink: &event_sink::Model,
    mut tokens: OAuthTokens,
    wanted: impl Fn(&str) -> bool,
) -> Result<OAuthTokens, ApiError> {
    let mut changed = false;
    for (provider_id, value) in tokens
        .iter_mut()
        .filter(|(provider_id, _)| wanted(provider_id))
    {
        let current = crate::routes::oauth::resolve_sink_oauth_token(
            state,
            provider_id,
            value,
            sink.run_as_user_id.as_deref(),
        )
        .await?;
        changed |= current != *value;
        *value = current;
    }
    if changed {
        use crate::routes::app::events::db::encrypt_token;
        let json = serde_json::to_string(&tokens)?;
        let encrypted = encrypt_token(&json, &state.encryption_key);
        let updated = event_sink::Entity::update_many()
            .col_expr(
                event_sink::Column::OauthTokensEncrypted,
                Expr::value(Some(encrypted)),
            )
            .col_expr(
                event_sink::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().fixed_offset()),
            )
            .filter(event_sink::Column::Id.eq(&sink.id))
            .filter(
                event_sink::Column::OauthTokensEncrypted.eq(sink.oauth_tokens_encrypted.clone()),
            )
            .exec(&state.db)
            .await?;
        if updated.rows_affected != 1 {
            return Err(ApiError::service_unavailable(
                "OAuth credentials changed during refresh; retry the workflow",
            ));
        }
    }
    tokens.retain(|provider_id, _| wanted(provider_id));
    Ok(tokens)
}

/// Input for programmatic event triggering
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriggerEventInput {
    /// The event ID to trigger
    pub event_id: String,
    /// Optional payload to pass to the event
    pub payload: Option<serde_json::Value>,
    /// The caller's `Idempotency-Key`, when one was supplied: the per-occurrence
    /// identity that keeps a retried trigger on the same canary variant.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// The Flow-Like person who triggered the run, set only once the caller checked that
    /// they may trigger events in the sink's app. The run acts as them, with their own
    /// profile and private Bits, never the sink's saved profile. `None` runs it as the
    /// sink's run-as user. Never read from a request body.
    #[serde(skip)]
    pub actor: Option<String>,
    /// The OAuth sign-ins the [`Self::actor`] supplied, which the caller resolves as theirs
    /// before the fire. Each wins over the sink's saved grant of its provider; the run takes
    /// the saved grants of every other provider. Never read from a request body.
    #[serde(skip)]
    pub oauth_tokens: Option<OAuthTokens>,
}

/// Response from trigger operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TriggerResponse {
    pub triggered: bool,
    pub run_id: Option<String>,
    pub message: String,
}

/// Record a fire that never became a run.
///
/// Cron schedules and bot services are unattended: the HTTP status they get
/// back is discarded, so without this the only trace of a dead schedule is a
/// log line in a process nobody is watching. Resolving the event row directly
/// is deliberate — the common failures here (orphaned schedule, sink-type
/// mismatch, expired PAT) are exactly the ones where the sink is unusable but
/// the event still knows which board the user meant.
async fn record_trigger_rejection(
    state: &AppState,
    event_id: &str,
    stage: rejection::RejectionStage,
    reason: &str,
    payload: Option<serde_json::Value>,
    run_id: Option<String>,
    actor_user_id: Option<String>,
) -> Option<String> {
    let Some(context) =
        rejection::context_for_event(state, event_id, stage, reason.to_string()).await
    else {
        tracing::warn!(
            event_id = %event_id,
            reason = %reason,
            "Trigger rejected for an event that no longer exists; nothing to record"
        );
        return None;
    };

    let mut context = context
        .with_payload(payload)
        .with_actor(actor_user_id, None);
    if let Some(run_id) = run_id {
        context = context.with_run_id(run_id);
    }

    Some(rejection::record(state, context).await)
}

/// Utility function to trigger an event programmatically.
///
/// Use this in Lambda handlers, SQS processors, cron job workers, etc.
/// The caller resolves the active sink and its event (validated against the
/// sink's app) and hands both rows in, so they are loaded once per fire.
///
/// The run executes as the sink's run-as user, checked by [`resolve_sink_run_as`]: the
/// executor gets a JWT for that user, marked unattended because no person triggered the
/// run, their current profile and the sink's OAuth grants, which belong to them. No stored
/// credential is forwarded. A refusal is an `ApiError` with `SINK_RUN_AS_REFUSED` (403) or
/// `SINK_RUN_AS_UNAVAILABLE` (503) inside the error.
///
/// A run with an [`TriggerEventInput::actor`] executes as that person instead: a JWT for
/// them, their own profile and private Bits, and the OAuth sign-ins they supplied, with the
/// sink's saved grants for every other provider. The run-as user is neither checked nor
/// paused for it, and a sink a release paused still takes it; only an inactive project
/// refuses it, with the same `SINK_RUN_AS_REFUSED` answer.
///
/// # Example
/// ```ignore
/// // In a Lambda handler
/// let result = trigger_event(&state, sink, event, TriggerEventInput {
///     event_id: "event_123".to_string(),
///     payload: Some(json!({"key": "value"})),
///     idempotency_key: None,
///     actor: None,
///     oauth_tokens: None,
/// }).await?;
/// ```
pub async fn trigger_event(
    state: &AppState,
    sink: event_sink::Model,
    event: flow_like::flow::event::Event,
    input: TriggerEventInput,
) -> FlResult<TriggerResponse> {
    trigger_event_dispatch(state, sink, event, input, create_id(), false).await
}

/// Durable transports retain one run identity across delivery retries.
pub(crate) async fn trigger_event_with_run_id(
    state: &AppState,
    sink: event_sink::Model,
    event: flow_like::flow::event::Event,
    input: TriggerEventInput,
    run_id: String,
) -> FlResult<TriggerResponse> {
    trigger_event_dispatch(state, sink, event, input, run_id, true).await
}

/// Who a programmatic fire runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FireActor {
    /// The Flow-Like person who triggered it.
    Person(String),
    /// The sink's run-as user, for a fire no person triggered.
    RunAs,
}

impl FireActor {
    fn of(actor: Option<String>) -> Self {
        actor.map_or(Self::RunAs, Self::Person)
    }

    fn sink_use(&self) -> SinkUse {
        match self {
            Self::Person(_) => SinkUse::Person,
            Self::RunAs => SinkUse::RunAs,
        }
    }

    /// The OAuth sign-ins a person supplied with their fire. A fire nobody triggered has none
    /// of its own and runs on the sink's saved grants alone.
    fn supplied_oauth_tokens(&self, supplied: Option<OAuthTokens>) -> Option<OAuthTokens> {
        match self {
            Self::Person(_) => supplied,
            Self::RunAs => None,
        }
    }

    /// The profile the fire carries: a person's own, never the one saved on the sink, else the
    /// run-as user's as [`crate::execution::sink_dispatch_profile`] picks it.
    async fn profile(
        &self,
        state: &AppState,
        sink: &event_sink::Model,
        run_as: Option<&str>,
    ) -> Option<serde_json::Value> {
        match self {
            Self::Person(person) => {
                crate::execution::fetch_profile_for_dispatch(state, person, None, &sink.app_id)
                    .await
            }
            Self::RunAs => crate::execution::sink_dispatch_profile(state, sink, run_as).await,
        }
    }

    /// The fire's executor token. A fire nobody triggered is marked unattended, so the apps
    /// it calls through connections never take the run-as user for a person.
    fn sign_executor_jwt(&self, params: ExecutionJwtParams) -> Result<String, ExecutionJwtError> {
        match self {
            Self::Person(_) => sign_execution_jwt(params),
            Self::RunAs => sign_unattended_execution_jwt(params),
        }
    }
}

/// The user a fire runs as. A person's fire needs only an active project: the caller checked
/// that the person may trigger the event, and the sink's run-as user plays no part, so it is
/// neither checked nor paused. Any other fire runs as the run-as user that
/// [`resolve_sink_run_as_for_run`] checks.
async fn fire_actor(
    state: &AppState,
    sink: &event_sink::Model,
    actor: &FireActor,
    run_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let FireActor::Person(person) = actor else {
        return resolve_sink_run_as_for_run(state, sink, run_id).await;
    };
    let execute = RolePermissions::ExecuteEvents;
    // Without a user only the app is judged, so the one refusal is an inactive project.
    match run_as_authority(&state.db, &sink.app_id, None, execute).await? {
        RunAsVerdict::Allowed(_) => Ok(Some(person.clone())),
        RunAsVerdict::Denied(denial) => {
            Err(refuse_person_fire(state, sink, person, denial, run_id).await)
        }
        RunAsVerdict::Transient(_) => Err(retry_later()),
    }
}

/// A person's fire the project refused, recorded on the event and answered like any fire
/// the project refuses. The sink stays on.
async fn refuse_person_fire(
    state: &AppState,
    sink: &event_sink::Model,
    person: &str,
    denial: RunAsDenial,
    run_id: Option<&str>,
) -> ApiError {
    tracing::info!(
        sink_id = %sink.id,
        event_id = %sink.event_id,
        app_id = %sink.app_id,
        reason = denial.reason(),
        "Refusing a fire a person triggered"
    );
    let run_id = rejection::undispatched_run(state, run_id).await;
    record_trigger_rejection(
        state,
        &sink.event_id,
        rejection::RejectionStage::Permission,
        denial.reason(),
        None,
        run_id,
        Some(person.to_owned()),
    )
    .await;
    run_as_refused(denial)
}

async fn trigger_event_dispatch(
    state: &AppState,
    sink: event_sink::Model,
    event: flow_like::flow::event::Event,
    input: TriggerEventInput,
    run_id: String,
    retry_dispatch: bool,
) -> FlResult<TriggerResponse> {
    // Check JWT is configured
    if !is_jwt_configured() {
        return Err(anyhow!("Execution JWT signing not configured"));
    }

    // Create run
    let expires_at = chrono::Utc::now().fixed_offset() + chrono::Duration::hours(24);
    let actor = FireActor::of(input.actor);
    let transport_run = retry_dispatch.then_some(run_id.as_str());
    let (actor_user_id, wasm_packages) = tokio::join!(
        fire_actor(state, &sink, &actor, transport_run),
        resolve_wasm_packages(state, &sink.app_id),
    );
    let actor_user_id = actor_user_id?;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    let actor_event_id = Some(sink.event_id.clone());
    debug_assert!(actor_user_id.is_some() || actor_event_id.is_some());

    // Live-variant split, resolved before the run row and the executor JWT.
    // The idempotency key is the only sticky identity a sink fire can offer;
    // without one each fire is an independent weighted draw on the run id.
    let target = variant::resolve_sink_target(&event, input.idempotency_key.as_deref(), &run_id);
    let PinnedFire { sink, board_etag } =
        pin_fire_for(state, sink, &target, actor.sink_use()).await?;

    let input_payload_len = input
        .payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = variant::dispatch_event_json(&event, &target)?;

    // Scoped to the user the fire runs as, or to the sink itself when it runs without one.
    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await?;
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    let executor_jwt = actor.sign_executor_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
        shadow: None,
    })?;

    // The sink's saved grants, refreshed and written back, under a person's own sign-ins.
    let oauth_tokens = run_oauth_tokens(
        state,
        Some(&sink),
        actor.sink_use(),
        actor.supplied_oauth_tokens(input.oauth_tokens),
    )
    .await?;

    // Build dispatch request
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        board_version: target.board_version,
        board_etag,
        node_id: target.node_id.clone(),
        event_json: Some(event_json),
        payload: input.payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        oauth_tokens,
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Sink triggers don't have user context
        profile: actor.profile(state, &sink, actor_user_id.as_deref()).await,
        custom_bit_owner: actor_user_id.clone(),
        wasm_packages,
        channel: None,
        // Programmatic trigger: cron workers, Lambda handlers, queue processors, bot
        // webhooks. Teams answers arrive through the HTTP channel grant every run receives.
        trigger: DispatchTrigger::System,
        shadow: false,
        artifact: None,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(target.board_id.clone()),
        version: Set(None),
        event_id: Set(actor_event_id),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(target.run_variant()),
        variant_name: Set(target.variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
    };

    // Insert run record
    let run_model = match execution_run::Entity::find_by_id(&run_id)
        .one(&state.db)
        .await?
    {
        Some(existing) => {
            if existing.app_id != sink.app_id
                || existing.event_id.as_deref() != Some(&sink.event_id)
            {
                return Err(anyhow!("Run identity belongs to another event"));
            }
            if existing.status != RunStatus::Pending {
                return Ok(TriggerResponse {
                    triggered: true,
                    run_id: Some(run_id),
                    message: "Event already dispatched".into(),
                });
            }
            let existing_subject = existing
                .user_id
                .clone()
                .unwrap_or_else(|| format!("sink:{}", sink.id));
            if existing.board_id != target.board_id || existing_subject != request.user_id {
                return Err(anyhow!("Pending run target or execution actor changed"));
            }
            existing
        }
        None => run.insert(&state.db).await?,
    };
    crate::audit::record_execution_dispatch_for(state, &run_model, "sink").await?;

    // Dispatch (fire and forget for programmatic triggers)
    // Use async dispatch which respects ASYNC_EXECUTION_BACKEND config
    let dispatch_result = state.dispatcher.dispatch_async(request).await;

    match dispatch_result {
        Ok(_) => Ok(TriggerResponse {
            triggered: true,
            run_id: Some(run_id),
            message: "Event triggered successfully".to_string(),
        }),
        Err(e) => {
            // A durable transport owns retries. Marking this run failed would
            // fence the executor even if the queue accepted a timed-out request.
            if !retry_dispatch {
                crate::audit::record_execution_dispatch_failure(state, &run_id, "sink").await?;
            }
            if let crate::execution::DispatchError::Quota(error) = e {
                return Err(error.into());
            }
            Ok(TriggerResponse {
                triggered: false,
                run_id: Some(run_id),
                message: format!("Dispatch failed: {}", e),
            })
        }
    }
}

/// POST/GET/etc /sink/trigger/{app_id}/{path}
/// HTTP endpoint for HTTP sinks
#[utoipa::path(
    post,
    path = "/sink/trigger/http/{app_id}/{path}",
    tag = "sink",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("path" = String, Path, description = "HTTP path for the sink")
    ),
    responses(
        (status = 200, description = "Event triggered successfully"),
        (status = 401, description = "Invalid or missing auth token"),
        (status = 403, description = "SINK_RUN_AS_REFUSED: the event is paused because the user it runs as may no longer run it, or its project is not active"),
        (status = 404, description = "Route not found"),
        (status = 500, description = "Internal server error"),
        (status = 503, description = "SINK_RUN_AS_UNAVAILABLE: the user the event runs as cannot be checked right now, or changed while the request was handled; retry later")
    )
)]
#[tracing::instrument(name = "ANY /sink/trigger/{app_id}/{path}", skip(state, path, request))]
pub async fn trigger_http(
    State(state): State<AppState>,
    Path((app_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;
    let method = request.method().clone();
    let headers = request.headers().clone();

    // Normalize path
    let normalized_path = if path.starts_with('/') {
        path
    } else {
        format!("/{}", path)
    };

    tracing::info!(
        "HTTP sink trigger: {} {} for app {}",
        method.as_str(),
        normalized_path,
        app_id
    );

    // Look up the sink using the unique (appId, path, method) index.
    // Two targeted queries — exact method match first, then a fallback for
    // legacy rows whose method column is still NULL from before this
    // migration landed. Postgres's unique constraint treats NULLs as
    // distinct, so pre-migration rows can coexist with new ones during
    // rollout; once they are re-saved through `sync_event_with_sink_tokens`
    // they'll pick up the explicit method and the fallback becomes a noop.
    // Both reach HTTP-typed events only, so no other event is fired past its own door.
    let method_str = method.as_str().to_ascii_uppercase();

    let exact_match = http_route_sink(&app_id, &normalized_path, Some(&method_str))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match exact_match {
        Some(s) => s,
        None => {
            let legacy = http_route_sink(&app_id, &normalized_path, None)
                .one(&state.db)
                .await
                .map_err(|e| {
                    tracing::error!("Database error: {}", e);
                    ApiError::internal_error(anyhow!("Database error"))
                })?;

            match legacy {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        "No active HTTP sink found for {} {} in app {}",
                        method_str,
                        normalized_path,
                        app_id
                    );
                    return Ok((
                        StatusCode::NOT_FOUND,
                        Json(TriggerResponse {
                            triggered: false,
                            run_id: None,
                            message: "Route not found".to_string(),
                        }),
                    )
                        .into_response());
                }
            }
        }
    };

    // Check auth token if set
    if let Some(expected_token) = &sink.auth_token {
        if !http_auth_token_matches(&headers, expected_token) {
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(TriggerResponse {
                    triggered: false,
                    run_id: None,
                    message: "Invalid or missing auth token".to_string(),
                }),
            )
                .into_response());
        }
    }

    // Create the run id before parsing so multipart files can be staged under
    // a stable, per-run temporary object prefix.
    let run_id = create_id();
    let (actor_user_id, event_result, wasm_packages) = tokio::join!(
        resolve_sink_run_as(&state, &sink),
        get_event_from_db(&state.db, &sink.event_id, &sink.app_id),
        resolve_wasm_packages(&state, &app_id),
    );
    let actor_user_id = actor_user_id?;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;
    let request_file_store = credentials
        .to_store_type(flow_like::credentials::StoreType::Tmp)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to create scratch store: {}", e)))?;
    // Offloaded under the executing subject's own scratch directory rather than a
    // shared tmp/global one: the executor downstream reads this file with the same
    // credentials, and an Azure directory SAS signs exactly one directory.
    let request_file_prefix = format!(
        "tmp/user/{}/apps/{}/runs/{}/request",
        sanitize_store_path_segment(&executor_subject, "user"),
        sanitize_store_path_segment(&app_id, "app"),
        sanitize_store_path_segment(&run_id, "run")
    );

    let parsed_payload = parse_http_request_payload(
        request,
        HTTP_SINK_BODY_LIMIT_BYTES,
        Some(request_file_store),
        Some(request_file_prefix),
    )
    .await?;
    let payload = parsed_payload.payload;

    // Config lives in Event
    let event = event_result
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    // Live-variant split, resolved before the run row and the executor JWT.
    // Webhook fires carry no occurrence identity, so each is an independent
    // weighted draw on the run id.
    let target = variant::resolve_sink_target(&event, None, &run_id);
    let PinnedFire { sink, board_etag } = pin_fire(&state, sink, &target).await?;

    let expires_at = chrono::Utc::now().fixed_offset() + chrono::Duration::hours(24);

    let input_payload_len = payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = variant::dispatch_event_json(&event, &target)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // No person triggers a webhook or bot fire.
    let executor_jwt = sign_unattended_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: target.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
        shadow: None,
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink, tokens).await?),
        None => None,
    };

    // Build dispatch request
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: target.board_id.clone(),
        board_version: target.board_version,
        board_etag,
        node_id: target.node_id.clone(),
        event_json: Some(event_json),
        payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // HTTP sink triggers don't have user context
        profile: crate::execution::sink_dispatch_profile(&state, &sink, actor_user_id.as_deref())
            .await,
        custom_bit_owner: actor_user_id.clone(),
        wasm_packages,
        channel: None,
        // Inbound webhook.
        trigger: DispatchTrigger::System,
        shadow: false,
        artifact: None,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(target.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(target.run_variant()),
        variant_name: Set(target.variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
    };

    tracing::info!(run_id = %run_id, "Dispatching HTTP sink");

    // Persist the run record BEFORE dispatch so infrastructure failures
    // (executor crashes, network drops, timeouts) leave a visible Pending
    // row that can be reconciled, rather than a silently lost workflow.
    let run_model = match run.insert(&state.db).await {
        Ok(run_model) => run_model,
        Err(e) => {
            tracing::error!(run_id = %run_id, error = %e, "Failed to create run record");
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TriggerResponse {
                    triggered: false,
                    run_id: Some(run_id),
                    message: format!("Failed to create run record: {}", e),
                }),
            )
                .into_response());
        }
    };

    crate::audit::record_execution_dispatch_for(&state, &run_model, "sink:http").await?;

    // Requests buffer the final result unless the caller explicitly accepts
    // SSE. Both modes retain the same execution deadline and run bookkeeping.
    const HTTP_SINK_RESULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
    let streaming = wants_http_event_stream(&headers);

    let backend = state.dispatcher.backend();

    match backend {
        ExecutionBackend::LambdaStream => {
            let dispatch_result = state.dispatcher.dispatch_streaming(request).await;

            match dispatch_result {
                Ok((_dispatch_response, byte_stream)) => {
                    let db_arc = crate::audit::ExecutionAuditContext::from(&state);
                    if streaming {
                        return Ok(stream_http_response(
                            byte_stream,
                            run_id,
                            Some(db_arc),
                            HTTP_SINK_RESULT_TIMEOUT,
                            std::convert::identity,
                        ));
                    }
                    tracing::info!(run_id = %run_id, "Got Lambda response, collecting result");
                    let run_id_owned = run_id.clone();
                    let generic_result = collect_generic_result_bytes(
                        byte_stream,
                        run_id_owned,
                        Some(db_arc),
                        HTTP_SINK_RESULT_TIMEOUT,
                    )
                    .await;

                    match generic_result {
                        Some(payload) => {
                            Ok((StatusCode::OK, [(header::VARY, "Accept")], Json(payload))
                                .into_response())
                        }
                        None => Ok((
                            StatusCode::OK,
                            [(header::VARY, "Accept")],
                            Json(TriggerResponse {
                                triggered: true,
                                run_id: Some(run_id),
                                message: "Event triggered".to_string(),
                            }),
                        )
                            .into_response()),
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dispatch Lambda streaming");
                    crate::audit::record_execution_dispatch_failure(&state, &run_id, "sink:http")
                        .await?;
                    Err(ApiError::from(e))
                }
            }
        }
        _ => {
            let dispatch_result = state.dispatcher.dispatch_http_sse(request).await;

            match dispatch_result {
                Ok((_dispatch_response, executor_response)) => {
                    let db_arc = crate::audit::ExecutionAuditContext::from(&state);
                    if streaming {
                        return Ok(stream_http_response(
                            executor_response_bytes(executor_response),
                            run_id,
                            Some(db_arc),
                            HTTP_SINK_RESULT_TIMEOUT,
                            std::convert::identity,
                        ));
                    }
                    tracing::info!(run_id = %run_id, "Got executor response, collecting result");
                    let run_id_owned = run_id.clone();
                    let generic_result = collect_generic_result(
                        executor_response,
                        run_id_owned,
                        Some(db_arc),
                        HTTP_SINK_RESULT_TIMEOUT,
                    )
                    .await;

                    match generic_result {
                        Some(payload) => {
                            Ok((StatusCode::OK, [(header::VARY, "Accept")], Json(payload))
                                .into_response())
                        }
                        None => Ok((
                            StatusCode::OK,
                            [(header::VARY, "Accept")],
                            Json(TriggerResponse {
                                triggered: true,
                                run_id: Some(run_id),
                                message: "Event triggered".to_string(),
                            }),
                        )
                            .into_response()),
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dispatch");
                    crate::audit::record_execution_dispatch_failure(&state, &run_id, "sink:http")
                        .await?;
                    Err(ApiError::from(e))
                }
            }
        }
    }
}

/// Query params for Telegram webhook (optional secret_token as query param)
#[derive(Debug, Deserialize)]
pub struct TelegramQueryParams {
    /// Secret token can be passed as query param as an alternative to header
    pub secret_token: Option<String>,
}

/// POST /sink/trigger/telegram/{event_id}
/// Telegram webhook endpoint - async execution with secret token & IP verification
#[utoipa::path(
    post,
    path = "/sink/trigger/telegram/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID"),
        ("secret_token" = Option<String>, Query, description = "Telegram secret token")
    ),
    responses(
        (status = 200, description = "Webhook received and processing. With `triggered: false` and no run: left to the device that runs this bot, or not run because the user the event runs as may no longer run it (the event is paused) or its project is not active", body = TriggerResponse),
        (status = 401, description = "Invalid or missing secret token, or the trigger has no secret token configured"),
        (status = 403, description = "Request not from Telegram servers"),
        (status = 404, description = "Webhook not found or inactive"),
        (status = 503, description = "The user the event runs as cannot be checked right now, or changed while the update was handled; Telegram retries")
    )
)]
#[tracing::instrument(
    name = "POST /sink/trigger/telegram/{event_id}",
    skip(state, query, headers, body, connect_info)
)]
pub async fn trigger_telegram(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
    Query(query): Query<TelegramQueryParams>,
    headers: HeaderMap,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    body: Body,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;

    let client_ip = connect_info.ip();

    tracing::info!(
        "Telegram webhook trigger for event {} from IP {}",
        event_id,
        client_ip
    );

    // Verify IP is from Telegram (in production)
    // Skip in development/local mode
    let api_base_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let is_development = api_base_url.contains("localhost") || api_base_url.contains("127.0.0.1");

    if !is_development && !is_telegram_ip(&client_ip) {
        tracing::warn!(
            "Telegram webhook request from non-Telegram IP: {}",
            client_ip
        );
        return Ok((
            StatusCode::FORBIDDEN,
            Json(TriggerResponse {
                triggered: false,
                run_id: None,
                message: "Request not from Telegram servers".to_string(),
            }),
        )
            .into_response());
    }

    // Look up the Telegram sink of this event
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .filter(event_sink::Column::SinkType.eq(sink_types::TELEGRAM))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match sink {
        Some(s) => s,
        None => {
            tracing::warn!("No active Telegram sink found for event {}", event_id);
            return Ok((
                StatusCode::NOT_FOUND,
                Json(TriggerResponse {
                    triggered: false,
                    run_id: None,
                    message: "Webhook not found or inactive".to_string(),
                }),
            )
                .into_response());
        }
    };

    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Verify secret token (from header X-Telegram-Bot-Api-Secret-Token or query param)
    let Some(expected_secret) = door_secret(&sink, &event.config) else {
        tracing::warn!(event_id = %event_id, "Telegram trigger has no secret token; refusing");
        return Ok(telegram_unauthorized(
            "This Telegram trigger has no secret token",
        ));
    };
    if !telegram_secret_matches(&headers, query.secret_token.as_deref(), &expected_secret) {
        tracing::warn!(
            "Invalid or missing Telegram secret token for event {}",
            event_id
        );
        return Ok(telegram_unauthorized("Invalid or missing secret token"));
    }

    if bot_on_device(&state, &sink).await? {
        tracing::info!(
            event_id = %event_id,
            app_id = %sink.app_id,
            "Telegram webhook: a device runs this bot, not dispatched"
        );
        return Ok(telegram_runs_on_device());
    }

    // Parse body (Telegram sends JSON)
    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024) // 10MB limit
        .await
        .map_err(|e| {
            tracing::error!("Failed to read body: {}", e);
            ApiError::bad_request("Failed to read request body")
        })?;

    let payload: Option<serde_json::Value> = if !body_bytes.is_empty() {
        match serde_json::from_slice(&body_bytes) {
            Ok(v) => Some(v),
            Err(_) => Some(serde_json::Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            )),
        }
    } else {
        None
    };

    let (run_as_result, wasm_packages) = tokio::join!(
        resolve_sink_run_as(&state, &sink),
        resolve_wasm_packages(&state, &sink.app_id),
    );
    let actor_user_id = match run_as_result {
        Ok(actor_user_id) => actor_user_id,
        Err(error) if is_run_as_refusal(&error) => return Ok(telegram_refused(&error)),
        Err(error) => return Err(error),
    };

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    // Create run
    let run_id = create_id();
    let expires_at = chrono::Utc::now().fixed_offset() + chrono::Duration::hours(24);

    // Live-variant split, resolved before the run row and the executor JWT.
    // Webhook fires carry no occurrence identity, so each is an independent
    // weighted draw on the run id.
    let target = variant::resolve_sink_target(&event, None, &run_id);
    let PinnedFire { sink, board_etag } = pin_fire(&state, sink, &target).await?;

    let input_payload_len = payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = variant::dispatch_event_json(&event, &target)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));

    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // No person triggers a webhook or bot fire.
    let executor_jwt = sign_unattended_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
        shadow: None,
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink, tokens).await?),
        None => None,
    };

    // Build dispatch request (async - no streaming)
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        board_version: target.board_version,
        board_etag,
        node_id: target.node_id.clone(),
        event_json: Some(event_json),
        payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Telegram webhook triggers don't have user context
        profile: crate::execution::sink_dispatch_profile(&state, &sink, actor_user_id.as_deref())
            .await,
        custom_bit_owner: actor_user_id.clone(),
        wasm_packages,
        channel: None,
        // Telegram bot service.
        trigger: DispatchTrigger::System,
        shadow: false,
        artifact: None,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(target.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(target.run_variant()),
        variant_name: Set(target.variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
    };

    tracing::info!(run_id = %run_id, "Dispatching Telegram webhook (async)");

    // Insert run record
    let run_model = run.insert(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record"))
    })?;

    crate::audit::record_execution_dispatch_for(&state, &run_model, "sink:telegram").await?;

    // Dispatch async (fire and forget) - Telegram expects fast response
    // Wait only for durable queue acceptance. A Lambda must not return while its
    // dispatch future exists solely in this request's process.
    if let Err(error) = state.dispatcher.dispatch_async(request).await {
        if let Err(audit_error) =
            crate::audit::record_execution_dispatch_failure(&state, &run_id, "sink:telegram").await
        {
            tracing::error!(%run_id, %audit_error, "Failed to audit webhook dispatch rejection");
        }
        return Err(ApiError::from(error));
    }

    // Return immediately - Telegram expects fast acknowledgement
    Ok((
        StatusCode::OK,
        Json(TriggerResponse {
            triggered: true,
            run_id: Some(run_id),
            message: "Webhook received and processing".to_string(),
        }),
    )
        .into_response())
}

/// Verify Discord Ed25519 signature
/// Discord sends: X-Signature-Ed25519 (signature) and X-Signature-Timestamp (timestamp)
/// The message to verify is: timestamp + body
fn verify_discord_signature(
    public_key_hex: &str,
    signature_hex: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    // Decode the public key from hex
    let public_key_bytes: [u8; 32] = match hex::decode(public_key_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => return false,
    };

    // Decode the signature from hex
    let signature_bytes: [u8; 64] = match hex::decode(signature_hex) {
        Ok(bytes) if bytes.len() == 64 => {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => return false,
    };

    // Create verifying key
    let verifying_key = match VerifyingKey::from_bytes(&public_key_bytes) {
        Ok(key) => key,
        Err(_) => return false,
    };

    // Create signature
    let signature = Signature::from_bytes(&signature_bytes);

    // Build the message: timestamp + body
    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    // Verify the signature
    use ed25519_dalek::Verifier;
    verifying_key.verify(&message, &signature).is_ok()
}

/// POST /sink/trigger/discord/{event_id}
/// Discord interactions webhook endpoint - async execution with Ed25519 signature verification
/// Discord requires responding to PING interactions with PONG, and must respond within 3 seconds
#[utoipa::path(
    post,
    path = "/sink/trigger/discord/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Interaction processed. While a device runs this bot, or when the user the event runs as may no longer run it (the event is paused) or its project is not active, a command gets a notice only its sender sees and starts no run."),
        (status = 401, description = "Invalid signature, or the trigger has no Discord public key configured"),
        (status = 404, description = "Webhook not found or inactive"),
        (status = 503, description = "The user the event runs as cannot be checked right now, or changed while the interaction was handled")
    )
)]
#[tracing::instrument(
    name = "POST /sink/trigger/discord/{event_id}",
    skip(state, headers, body)
)]
pub async fn trigger_discord(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;

    tracing::info!("Discord webhook trigger for event {}", event_id);

    // Read body first (needed for signature verification)
    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024) // 10MB limit
        .await
        .map_err(|e| {
            tracing::error!("Failed to read body: {}", e);
            ApiError::bad_request("Failed to read request body")
        })?;

    // Get signature headers
    let signature = headers
        .get("X-Signature-Ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let timestamp = headers
        .get("X-Signature-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Look up the Discord sink of this event
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .filter(event_sink::Column::SinkType.eq(sink_types::DISCORD))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match sink {
        Some(s) => s,
        None => {
            tracing::warn!("No active Discord sink found for event {}", event_id);
            return Ok((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Webhook not found or inactive"
                })),
            )
                .into_response());
        }
    };

    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Verify the Ed25519 signature with the Discord app's public key
    let Some(public_key) = door_secret(&sink, &event.config) else {
        tracing::warn!(event_id = %event_id, "Discord trigger has no public key; refusing");
        return Ok(discord_unauthorized(
            "This Discord trigger has no public key",
        ));
    };
    // Skip verification in development mode
    let api_base_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let is_development = api_base_url.contains("localhost") || api_base_url.contains("127.0.0.1");
    if !is_development && !verify_discord_signature(&public_key, signature, timestamp, &body_bytes)
    {
        tracing::warn!("Invalid Discord signature for event {}", event_id);
        return Ok(discord_unauthorized("Invalid signature"));
    }

    // Parse the interaction payload
    let interaction: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::error!("Failed to parse Discord interaction: {}", e);
        ApiError::bad_request("Invalid JSON payload")
    })?;

    // Check interaction type
    let interaction_type = interaction
        .get("type")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    // Type 1 = PING - must respond with PONG immediately
    if interaction_type == 1 {
        tracing::info!("Discord PING received, responding with PONG");
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "type": 1  // PONG
            })),
        )
            .into_response());
    }

    if bot_on_device(&state, &sink).await? {
        tracing::info!(
            event_id = %event_id,
            app_id = %sink.app_id,
            "Discord interaction: a device runs this bot, not dispatched"
        );
        return Ok(discord_runs_on_device());
    }

    // For other interaction types (commands, components, etc.), dispatch async
    let (run_as_result, wasm_packages) = tokio::join!(
        resolve_sink_run_as(&state, &sink),
        resolve_wasm_packages(&state, &sink.app_id),
    );
    let actor_user_id = match run_as_result {
        Ok(actor_user_id) => actor_user_id,
        Err(error) if is_run_as_refusal(&error) => return Ok(discord_refused(&error)),
        Err(error) => return Err(error),
    };

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    // Create run
    let run_id = create_id();
    let expires_at = chrono::Utc::now().fixed_offset() + chrono::Duration::hours(24);

    // Live-variant split, resolved before the run row and the executor JWT.
    // Webhook fires carry no occurrence identity, so each is an independent
    // weighted draw on the run id.
    let target = variant::resolve_sink_target(&event, None, &run_id);
    let PinnedFire { sink, board_etag } = pin_fire(&state, sink, &target).await?;

    let input_payload_len = serde_json::to_string(&interaction)
        .map(|s| s.len() as i64)
        .unwrap_or(0);

    let event_json = variant::dispatch_event_json(&event, &target)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));

    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // No person triggers a webhook or bot fire.
    let executor_jwt = sign_unattended_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
        shadow: None,
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink, tokens).await?),
        None => None,
    };

    // Build dispatch request (async - no streaming)
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: target.board_id.clone(),
        board_version: target.board_version,
        board_etag,
        node_id: target.node_id.clone(),
        event_json: Some(event_json),
        payload: Some(interaction.clone()),
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Discord webhook triggers don't have user context
        profile: crate::execution::sink_dispatch_profile(&state, &sink, actor_user_id.as_deref())
            .await,
        custom_bit_owner: actor_user_id.clone(),
        wasm_packages,
        channel: None,
        // Discord bot service.
        trigger: DispatchTrigger::System,
        shadow: false,
        artifact: None,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(target.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        run_variant: Set(target.run_variant()),
        variant_name: Set(target.variant_name.clone()),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        event_version: Set(None),
        nodes: Set(None),
        logs_count: Set(None),
    };

    tracing::info!(run_id = %run_id, "Dispatching Discord webhook (async)");

    // Insert run record
    let run_model = run.insert(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record"))
    })?;

    crate::audit::record_execution_dispatch_for(&state, &run_model, "sink:discord").await?;

    // Dispatch async (fire and forget) - Discord expects response within 3 seconds
    // Wait only for durable queue acceptance. A Lambda must not return while its
    // dispatch future exists solely in this request's process.
    if let Err(error) = state.dispatcher.dispatch_async(request).await {
        if let Err(audit_error) =
            crate::audit::record_execution_dispatch_failure(&state, &run_id, "sink:discord").await
        {
            tracing::error!(%run_id, %audit_error, "Failed to audit webhook dispatch rejection");
        }
        return Err(ApiError::from(error));
    }

    // Discord expects a deferred response for commands (type 5)
    // This tells Discord we're processing and will follow up later
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "type": 5  // DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE
        })),
    )
        .into_response())
}

/// JWT claims for sink trigger service tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkTriggerClaims {
    /// Subject - always "sink-trigger"
    pub sub: String,
    /// Issuer - always "flow-like"
    pub iss: String,
    /// JWT ID - unique identifier for revocation checking
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    /// Which sink types this token can trigger
    pub sink_types: Vec<String>,
    /// Optional: restrict to specific app IDs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_ids: Option<Vec<String>>,
    /// Issued at timestamp
    pub iat: usize,
    /// Expiration timestamp (optional - can be very long-lived)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<usize>,
}

/// Request body for service-to-service trigger
#[derive(Debug, Deserialize, ToSchema)]
pub struct ServiceTriggerRequest {
    /// The event ID to trigger
    pub event_id: String,
    /// The sink type (must match token's allowed sink_types)
    pub sink_type: String,
    /// Optional payload
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// Response from service trigger
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceTriggerResponse {
    pub success: bool,
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `true` when this request carried an `Idempotency-Key` that another replica was
    /// still dispatching; the trigger was accepted exactly once, but its `run_id` is
    /// unknown to this replica.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
    /// Why the trigger was accepted without a run, so the scheduler keeps the schedule and
    /// does not retry. `runs_on_device`: a device runs this schedule or bot, or ran it until
    /// a moment ago, so the hub leaves it to the device. `inactive`: the event's trigger is
    /// off or paused. `refused`: the user the event runs as may no longer run it, so the
    /// event was paused, or its project is not active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl ServiceTriggerResponse {
    fn duplicate_in_flight() -> Self {
        Self {
            success: true,
            run_id: None,
            error: None,
            duplicate: true,
            skipped: None,
        }
    }

    /// Success without a run: schedulers keep the schedule and do not retry.
    fn skipped(reason: &str) -> Self {
        Self {
            success: true,
            run_id: None,
            error: None,
            duplicate: false,
            skipped: Some(reason.to_owned()),
        }
    }

    fn runs_on_device() -> Self {
        Self::skipped(SKIPPED_RUNS_ON_DEVICE)
    }

    fn inactive() -> Self {
        Self::skipped(SKIPPED_INACTIVE)
    }

    fn refused() -> Self {
        Self::skipped(SKIPPED_REFUSED)
    }

    /// A failure the scheduler retries.
    fn failed(error: impl Into<String>) -> Self {
        Self {
            success: false,
            run_id: None,
            error: Some(error.into()),
            duplicate: false,
            skipped: None,
        }
    }
}

const SKIPPED_RUNS_ON_DEVICE: &str = "runs_on_device";
const SKIPPED_INACTIVE: &str = "inactive";
const SKIPPED_REFUSED: &str = "refused";

/// The occurrence a scheduler says it is firing, in Unix seconds. The minute-tick schedulers
/// send it and replay late occurrences; the others send none and fire on time or not at all.
fn scheduled_occurrence(payload: Option<&serde_json::Value>) -> Option<i64> {
    let scheduled_for = payload?.get("scheduled_for")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(scheduled_for)
        .ok()
        .map(|occurrence| occurrence.timestamp())
}

/// Whether the hub leaves this fire to a device. Only a schedule or a bot can be handed to one,
/// so no other sink type is ever looked up, and only a schedule names the time it fires for.
async fn left_to_device(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    sink_type: &str,
    app_id: &str,
    event_id: &str,
    payload: Option<&serde_json::Value>,
) -> Result<bool, ApiError> {
    if !CLAIMED_EVENT_TYPES.contains(&sink_type) {
        return Ok(false);
    }
    let scheduled = if sink_type == "cron" {
        scheduled_occurrence(payload)
    } else {
        None
    };
    crate::instances::schedules::runs_on_device(
        db,
        dialect,
        app_id,
        event_id,
        scheduled,
        chrono::Utc::now().timestamp(),
    )
    .await
}

/// Whether a device holds the bot of this sink, or held it until a moment ago. Each door asks
/// only after its own checks, so a caller it would turn away learns nothing about where the bot
/// runs.
async fn bot_on_device(state: &AppState, sink: &event_sink::Model) -> Result<bool, ApiError> {
    left_to_device(
        &state.db,
        state.db_dialect,
        &sink.sink_type,
        &sink.app_id,
        &sink.event_id,
        None,
    )
    .await
}

/// Telegram's answer while a device holds the bot: a success, because Telegram delivers an
/// update again after any other status, and no run.
fn telegram_runs_on_device() -> Response {
    (
        StatusCode::OK,
        Json(TriggerResponse {
            triggered: false,
            run_id: None,
            message: "This bot runs on a device".to_string(),
        }),
    )
        .into_response()
}

/// Discord's answer to a command while a device holds the bot: a message only its sender sees
/// (response type 4 with the ephemeral flag 64), and no run.
fn discord_runs_on_device() -> Response {
    discord_notice("This bot runs on a device and does not answer commands here.")
}

fn discord_notice(content: &str) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "type": 4,
            "data": {
                "content": content,
                "flags": 64
            }
        })),
    )
        .into_response()
}

/// Telegram's answer when the user the event runs as may no longer run it: a success, so
/// Telegram does not deliver the update again, and no run.
fn telegram_refused(refusal: &ApiError) -> Response {
    (
        StatusCode::OK,
        Json(TriggerResponse {
            triggered: false,
            run_id: None,
            message: refusal
                .public_message()
                .unwrap_or("This event is paused")
                .to_owned(),
        }),
    )
        .into_response()
}

/// Discord's answer when the user the event runs as may no longer run it: a notice only the
/// command's sender sees, and no run.
fn discord_refused(refusal: &ApiError) -> Response {
    discord_notice(refusal.public_message().unwrap_or("This event is paused"))
}

fn telegram_unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(TriggerResponse {
            triggered: false,
            run_id: None,
            message: message.to_owned(),
        }),
    )
        .into_response()
}

fn discord_unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// The listed sinks (event, app, type, active) without the bots a device runs, or whose grace
/// period runs: one claim query for all of them, so no bot service connects such a bot a second
/// time. Schedules stay listed, as in the schedule list: their schedulers keep ticking and are
/// told to skip.
pub(crate) async fn without_device_bots(
    db: &sea_orm::DatabaseConnection,
    sink_type: &str,
    sinks: Vec<(String, String, String, bool)>,
) -> Result<Vec<(String, String, String, bool)>, ApiError> {
    if !BOT_EVENT_TYPES.contains(&sink_type) {
        return Ok(sinks);
    }
    let events = sinks
        .iter()
        .map(|(event_id, app_id, ..)| (app_id.clone(), event_id.clone()))
        .collect::<Vec<_>>();
    let claimed =
        crate::instances::schedules::claimed_events(db, &events, chrono::Utc::now().timestamp())
            .await?;
    Ok(sinks
        .into_iter()
        .filter(|(event_id, app_id, ..)| !claimed.contains(&(app_id.clone(), event_id.clone())))
        .collect())
}

/// Platform-cache namespace holding one record per `Idempotency-Key`.
const TRIGGER_IDEMPOTENCY_NAMESPACE: &str = "sink-trigger-idempotency";
/// How long a finished trigger's response is replayed to retries.
const TRIGGER_IDEMPOTENCY_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// How long an in-flight reservation blocks duplicates. Bounded so a replica that dies
/// mid-dispatch does not swallow the trigger for the whole replay window.
const TRIGGER_IN_FLIGHT_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// How long a duplicate waits for the original to publish its response.
const TRIGGER_DUPLICATE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
const TRIGGER_DUPLICATE_POLL: std::time::Duration = std::time::Duration::from_millis(150);

fn scoped_trigger_idempotency_key(
    token: &str,
    app_id: &str,
    request: &ServiceTriggerRequest,
    key: &str,
) -> String {
    let scope = serde_json::json!([
        token,
        app_id,
        request.event_id,
        request.sink_type,
        request.payload,
        key,
    ]);
    format!("v2:{}", blake3::hash(scope.to_string().as_bytes()).to_hex())
}

#[cfg(test)]
mod idempotency_scope_tests {
    use super::*;

    #[test]
    fn retries_share_responses_only_with_the_same_authorized_operation() {
        let mut request = ServiceTriggerRequest {
            event_id: "event-a".into(),
            sink_type: "cron".into(),
            payload: Some(serde_json::json!({"value": 1})),
        };
        let key = scoped_trigger_idempotency_key("credential-a", "app-a", &request, "retry");
        let cache = moka::sync::Cache::new(10);
        cache.insert(key.clone(), "run-a");
        assert_eq!(
            cache.get(&scoped_trigger_idempotency_key(
                "credential-a",
                "app-a",
                &request,
                "retry"
            )),
            Some("run-a")
        );
        for (token, app, header) in [
            ("credential-b", "app-a", "retry"),
            ("credential-a", "app-b", "retry"),
            ("credential-a", "app-a", "other-retry"),
        ] {
            assert!(
                cache
                    .get(&scoped_trigger_idempotency_key(
                        token, app, &request, header
                    ))
                    .is_none()
            );
        }
        request.event_id = "event-b".into();
        assert_ne!(
            key,
            scoped_trigger_idempotency_key("credential-a", "app-a", &request, "retry")
        );
        request.event_id = "event-a".into();
        request.sink_type = "discord".into();
        assert_ne!(
            key,
            scoped_trigger_idempotency_key("credential-a", "app-a", &request, "retry")
        );
        request.sink_type = "cron".into();
        request.payload = Some(serde_json::json!({"value": 2}));
        assert_ne!(
            key,
            scoped_trigger_idempotency_key("credential-a", "app-a", &request, "retry")
        );
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum TriggerIdempotencyRecord {
    InFlight { since_ms: i64 },
    Done { response: ServiceTriggerResponse },
}

impl TriggerIdempotencyRecord {
    fn in_flight() -> Self {
        Self::InFlight {
            since_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// Cross-replica claim on an `Idempotency-Key`.
pub(super) enum IdempotencyClaim {
    /// No key, or no platform cache: dispatch as usual, guarded only per replica.
    Unreserved,
    /// This request owns the key and must publish its response.
    Owned(crate::cache::PlatformCache),
    /// Another request already produced (or is producing) the response.
    Replay(ServiceTriggerResponse),
}

async fn reserve_trigger(
    cache: &crate::cache::PlatformCache,
    key: &str,
) -> Result<crate::cache::Reservation<TriggerIdempotencyRecord>, crate::cache::CacheStoreError> {
    cache
        .try_insert(
            TRIGGER_IDEMPOTENCY_NAMESPACE,
            key,
            &TriggerIdempotencyRecord::in_flight(),
            TRIGGER_IN_FLIGHT_TTL,
        )
        .await
}

/// Claim `key` across replicas. Runs after every validation step, so a request that is
/// about to be rejected never blocks a later, valid retry.
pub(super) async fn claim_trigger_idempotency(
    state: &AppState,
    key: &str,
    event_id: &str,
) -> IdempotencyClaim {
    use crate::cache::Reservation;

    let cache = match state.cache.platform().await {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!(
                error = %error,
                idempotency_key = %key,
                "Platform cache unavailable; trigger idempotency is per replica for this request"
            );
            return IdempotencyClaim::Unreserved;
        }
    };

    let unreserved = |error: crate::cache::CacheStoreError| {
        tracing::warn!(
            error = %error,
            idempotency_key = %key,
            "Idempotency reservation failed; dispatching without a cross-replica claim"
        );
        IdempotencyClaim::Unreserved
    };

    match reserve_trigger(&cache, key).await {
        Ok(Reservation::Acquired) => return IdempotencyClaim::Owned(cache),
        Ok(Reservation::Held(TriggerIdempotencyRecord::Done { response })) => {
            tracing::info!(
                idempotency_key = %key,
                event_id = %event_id,
                "Replaying idempotent trigger response from another replica"
            );
            return IdempotencyClaim::Replay(response);
        }
        Ok(Reservation::Held(TriggerIdempotencyRecord::InFlight { .. })) => {}
        Err(error) => return unreserved(error),
    }

    // The original is still dispatching somewhere. Give it a moment to publish so the
    // retry gets the real run id; if it lapses without a result, the key is free again.
    let deadline = tokio::time::Instant::now() + TRIGGER_DUPLICATE_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(TRIGGER_DUPLICATE_POLL).await;
        match cache
            .get::<TriggerIdempotencyRecord>(TRIGGER_IDEMPOTENCY_NAMESPACE, key)
            .await
        {
            Ok(Some(TriggerIdempotencyRecord::Done { response })) => {
                return IdempotencyClaim::Replay(response);
            }
            Ok(Some(TriggerIdempotencyRecord::InFlight { .. })) => {}
            Ok(None) => match reserve_trigger(&cache, key).await {
                Ok(Reservation::Acquired) => return IdempotencyClaim::Owned(cache),
                Ok(Reservation::Held(TriggerIdempotencyRecord::Done { response })) => {
                    return IdempotencyClaim::Replay(response);
                }
                Ok(Reservation::Held(TriggerIdempotencyRecord::InFlight { .. })) => {}
                Err(error) => return unreserved(error),
            },
            Err(error) => return unreserved(error),
        }
    }

    tracing::info!(
        idempotency_key = %key,
        event_id = %event_id,
        "Collapsed duplicate trigger onto an in-flight request on another replica"
    );
    IdempotencyClaim::Replay(ServiceTriggerResponse::duplicate_in_flight())
}

/// Gives an owned claim back, so a retry with the same key dispatches again.
pub(super) async fn release_trigger_claim(claim: &IdempotencyClaim, key: Option<&str>) {
    if let (Some(key), IdempotencyClaim::Owned(cache)) = (key, claim)
        && let Err(cleanup) = cache.delete(TRIGGER_IDEMPOTENCY_NAMESPACE, key).await
    {
        tracing::warn!(%cleanup, "Unable to release rejected trigger claim");
    }
}

pub(super) async fn publish_trigger_result(
    cache: &crate::cache::PlatformCache,
    key: &str,
    response: &ServiceTriggerResponse,
) {
    let record = TriggerIdempotencyRecord::Done {
        response: response.clone(),
    };
    if let Err(error) = cache
        .set(
            TRIGGER_IDEMPOTENCY_NAMESPACE,
            key,
            &record,
            TRIGGER_IDEMPOTENCY_TTL,
        )
        .await
    {
        tracing::warn!(
            error = %error,
            idempotency_key = %key,
            "Failed to publish idempotent trigger response; retries on other replicas may re-dispatch after the reservation lapses"
        );
    }
}

/// Validate a sink trigger JWT and extract claims (without DB check)
pub(crate) fn validate_sink_trigger_jwt(
    token: &str,
    secret: &str,
) -> Result<SinkTriggerClaims, ApiError> {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.required_spec_claims.remove("exp");
    validation.validate_exp = false;
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());

    let token_data = jsonwebtoken::decode::<SinkTriggerClaims>(token, &key, &validation)
        .map_err(|e| ApiError::unauthorized(format!("Invalid sink trigger token: {}", e)))?;

    // Verify subject
    if token_data.claims.sub != "sink-trigger" {
        return Err(ApiError::unauthorized("Invalid token subject"));
    }

    // Verify issuer
    if token_data.claims.iss != "flow-like" {
        return Err(ApiError::unauthorized("Invalid token issuer"));
    }

    if let Some(exp) = token_data.claims.exp {
        let now = chrono::Utc::now().timestamp() as usize;
        if exp <= now {
            return Err(ApiError::unauthorized("Sink trigger token has expired"));
        }
    }

    Ok(token_data.claims)
}

/// Tokens known to be revoked. Revocation is permanent, so remembering it is
/// always safe; a token that is still valid is re-checked on every trigger so a
/// revocation takes effect at once on every replica.
static REVOKED_SINK_TOKENS: std::sync::LazyLock<moka::sync::Cache<String, ()>> =
    std::sync::LazyLock::new(|| moka::sync::Cache::builder().max_capacity(1_000).build());

/// Check if a sink token has been revoked
async fn is_token_revoked(db: &sea_orm::DatabaseConnection, jti: &str) -> Result<bool, ApiError> {
    use crate::entity::sink_token;
    use sea_orm::EntityTrait;

    if REVOKED_SINK_TOKENS.contains_key(jti) {
        return Ok(true);
    }

    let token = sink_token::Entity::find_by_id(jti)
        .one(db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    match token {
        Some(t) => {
            if t.revoked {
                REVOKED_SINK_TOKENS.insert(jti.to_string(), ());
            }
            Ok(t.revoked)
        }
        // If token not found in DB, it's either an old token (pre-registration system)
        // or an invalid jti. We allow it for backward compatibility but log a warning.
        None => {
            tracing::warn!(jti = %jti, "Token jti not found in database - allowing for backward compatibility");
            Ok(false)
        }
    }
}

/// POST /sink/trigger/async
///
/// Service-to-service trigger endpoint for internal sink services (cron, discord bot, telegram bot, etc.)
///
/// Authentication: Bearer token with scoped sink trigger JWT
///
/// The JWT must include:
/// - `sub`: "sink-trigger"
/// - `iss`: "flow-like"
/// - `jti`: JWT ID for revocation checking (optional for backward compatibility)
/// - `sink_types`: Array of allowed sink types (e.g., ["cron"] or ["discord"])
///
/// Security: Each service gets a JWT scoped to only its sink type:
/// - Cron service gets JWT with `sink_types: ["cron"]`
/// - Discord bot gets JWT with `sink_types: ["discord"]`
/// - Telegram bot gets JWT with `sink_types: ["telegram"]`
///
/// If a service is compromised, it can only trigger events of its own type.
/// Tokens can be individually revoked via /admin/sinks/{jti}.
///
/// Idempotency: callers may include an `Idempotency-Key` header. The key is
/// reserved in the platform cache before dispatch, so a retry landing on any
/// replica within ~15 minutes gets the original response instead of a second
/// run. A retry that arrives while the original is still dispatching waits
/// briefly and then answers `success: true, duplicate: true` without a run id.
/// This shields downstream flows from EventBridge Scheduler's and Lambda's
/// automatic retries on transient errors.
///
/// A sink that is off answers `success: true, skipped: "inactive"`, and one whose
/// run-as user may no longer run it answers `skipped: "refused"`: the schedule is kept
/// and nothing is retried. Only a missing sink is a 404.
#[utoipa::path(
    post,
    path = "/sink/trigger/async",
    tag = "sink",
    request_body = ServiceTriggerRequest,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Invocation-unique key; repeats within ~15 minutes replay the first response instead of dispatching again")
    ),
    responses(
        (status = 200, description = "Service trigger response. `skipped` names why no run started: `runs_on_device`, `inactive` (the trigger is off or paused) or `refused` (the user the event runs as may no longer run it, or its project is not active). `success: false` asks the scheduler to retry.", body = ServiceTriggerResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for sink type"),
        (status = 404, description = "No sink exists for the event")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /sink/trigger/async", skip_all)]
pub async fn trigger_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ServiceTriggerRequest>,
) -> Result<Json<ServiceTriggerResponse>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Check if token has been revoked (if jti is present)
    if let Some(ref jti) = claims.jti
        && is_token_revoked(&state.db, jti).await?
    {
        tracing::warn!(jti = %jti, "Attempted use of revoked sink token");
        return Err(ApiError::unauthorized("Token has been revoked"));
    }

    // Check if this JWT is allowed to trigger this sink type
    if !claims.sink_types.contains(&request.sink_type) {
        tracing::warn!(
            requested_type = %request.sink_type,
            allowed_types = ?claims.sink_types,
            "Token not authorized for sink type"
        );
        return Err(ApiError::forbidden(format!(
            "Token not authorized for sink type: {}",
            request.sink_type
        )));
    }

    // On or off: only a missing sink is the 404 that makes the AWS scheduler delete the
    // schedule, so a paused sink keeps its schedule for when it is turned back on.
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&request.event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let sink = match sink {
        Some(sink) => sink,
        None => {
            let reason = format!("No active sink found for event {}", request.event_id);
            let _ = record_trigger_rejection(
                &state,
                &request.event_id,
                rejection::RejectionStage::Trigger,
                &reason,
                request.payload.clone(),
                None,
                None,
            )
            .await;
            return Err(ApiError::not_found(reason));
        }
    };

    // Verify sink type matches
    if sink.sink_type != request.sink_type {
        tracing::warn!(
            event_id = %request.event_id,
            expected_type = %sink.sink_type,
            requested_type = %request.sink_type,
            "Sink type mismatch"
        );
        let reason = format!(
            "Sink type mismatch: event {} is of type {}, not {}",
            request.event_id, sink.sink_type, request.sink_type
        );
        let _ = record_trigger_rejection(
            &state,
            &request.event_id,
            rejection::RejectionStage::Trigger,
            &reason,
            request.payload.clone(),
            None,
            None,
        )
        .await;
        return Err(ApiError::bad_request(reason));
    }

    // Check app_id restriction if present in token
    if let Some(ref allowed_apps) = claims.app_ids
        && !allowed_apps.contains(&sink.app_id)
    {
        return Err(ApiError::forbidden(format!(
            "Token not authorized for app: {}",
            sink.app_id
        )));
    }

    // Both cache layers share a credential- and operation-bound key. Check
    // current authorization before replaying a previous dispatch response.
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|header| header.to_str().ok())
        .filter(|key| !key.is_empty())
        .map(|key| scoped_trigger_idempotency_key(token, &sink.app_id, &request, key));
    if let Some(ref key) = idempotency_key
        && let Some(cached) = state.trigger_idempotency.get(key)
    {
        return Ok(Json(cached));
    }

    if !sink.active {
        tracing::info!(
            event_id = %request.event_id,
            app_id = %sink.app_id,
            "Service trigger: the event's trigger is off, not dispatched"
        );
        return Ok(Json(ServiceTriggerResponse::inactive()));
    }

    // A schedule or bot a device runs is skipped with a success: schedulers keep ticking, so
    // nothing is replayed once it returns to the hub. Never the 404 above, which makes the
    // AWS scheduler delete the schedule.
    if left_to_device(
        &state.db,
        state.db_dialect,
        &sink.sink_type,
        &sink.app_id,
        &request.event_id,
        request.payload.as_ref(),
    )
    .await?
    {
        tracing::info!(
            event_id = %request.event_id,
            app_id = %sink.app_id,
            sink_type = %sink.sink_type,
            "Service trigger: a device runs this schedule or bot, not dispatched"
        );
        return Ok(Json(ServiceTriggerResponse::runs_on_device()));
    }

    // Get the event to access its config for additional payload
    let event = get_event_from_db(&state.db, &request.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Merge payloads: event config payload (base) + request payload (override)
    // event.config is stored as JSON bytes (Vec<u8>) in the database
    let event_payload: Option<serde_json::Value> = if event.config.is_empty() {
        None
    } else {
        serde_json::from_slice::<serde_json::Value>(&event.config)
            .ok()
            .and_then(|config| config.get("payload").cloned())
    };
    let merged_payload = merge_payloads(event_payload, request.payload);

    let claim = match idempotency_key.as_deref() {
        Some(key) => claim_trigger_idempotency(&state, key, &request.event_id).await,
        None => IdempotencyClaim::Unreserved,
    };
    if let IdempotencyClaim::Replay(response) = claim {
        if let Some(key) = idempotency_key {
            state.trigger_idempotency.insert(key, response.clone());
        }
        return Ok(Json(response));
    }

    tracing::info!(
        event_id = %request.event_id,
        sink_type = %request.sink_type,
        app_id = %sink.app_id,
        "Service trigger: triggering event"
    );

    // Use the existing trigger_event utility
    let response = match trigger_event(
        &state,
        sink,
        event,
        TriggerEventInput {
            event_id: request.event_id.clone(),
            payload: merged_payload.clone(),
            idempotency_key: idempotency_key.clone(),
            actor: None,
            oauth_tokens: None,
        },
    )
    .await
    {
        Ok(result) if result.triggered => ServiceTriggerResponse {
            success: true,
            run_id: result.run_id,
            error: None,
            duplicate: false,
            skipped: None,
        },
        // The run row already exists but was never dispatched; finalize it with
        // the reason instead of leaving it Pending until the sweeper times it
        // out with no explanation.
        Ok(result) => {
            tracing::error!(message = %result.message, "Service trigger was not dispatched");
            let _ = record_trigger_rejection(
                &state,
                &request.event_id,
                rejection::RejectionStage::Dispatch,
                &result.message,
                merged_payload,
                result.run_id.clone(),
                None,
            )
            .await;
            ServiceTriggerResponse {
                success: false,
                run_id: result.run_id,
                error: Some(result.message),
                duplicate: false,
                skipped: None,
            }
        }
        Err(e) => match e.downcast_ref::<ApiError>() {
            Some(error) if error.public_code() == "PLAN_LIMIT_EXCEEDED" => {
                release_trigger_claim(&claim, idempotency_key.as_deref()).await;
                return Err(error.clone());
            }
            // Recorded by the run-as check. Not replayed, so the scheduler's retry asks again.
            Some(error) if error.public_code() == SINK_RUN_AS_UNAVAILABLE => {
                release_trigger_claim(&claim, idempotency_key.as_deref()).await;
                return Ok(Json(ServiceTriggerResponse::failed(
                    error.public_message().unwrap_or("Try again later"),
                )));
            }
            // Recorded, and the sink paused, by the run-as check.
            Some(error) if is_run_as_refusal(error) => ServiceTriggerResponse::refused(),
            _ => {
                tracing::error!(error = %e, "Service trigger failed");
                let reason = e.to_string();
                let run_id = record_trigger_rejection(
                    &state,
                    &request.event_id,
                    rejection::RejectionStage::Trigger,
                    &reason,
                    merged_payload,
                    None,
                    None,
                )
                .await;
                ServiceTriggerResponse {
                    success: false,
                    run_id,
                    error: Some(reason),
                    duplicate: false,
                    skipped: None,
                }
            }
        },
    };

    if let Some(key) = idempotency_key {
        state
            .trigger_idempotency
            .insert(key.clone(), response.clone());
        if let IdempotencyClaim::Owned(cache) = claim {
            publish_trigger_result(&cache, &key, &response).await;
        }
    }

    Ok(Json(response))
}

/// GET /sink/schedules
///
/// List all active cron schedules. Used by docker-compose sink service
/// to sync its in-memory scheduler with the database.
#[utoipa::path(
    get,
    path = "/sink/schedules",
    tag = "sink",
    responses(
        (status = 200, description = "List of cron schedules", body = Vec<CronScheduleInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for cron schedules")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/schedules", skip(state, headers))]
pub async fn get_cron_sinks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CronScheduleInfo>>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Only allow tokens with cron access to list schedules
    if !claims.sink_types.contains(&"cron".to_string()) {
        return Err(ApiError::forbidden(
            "Token not authorized to list cron schedules",
        ));
    }

    // Get all active cron sinks
    let sinks = event_sink::Entity::find()
        .select_only()
        .columns([
            event_sink::Column::EventId,
            event_sink::Column::AppId,
            event_sink::Column::Active,
            event_sink::Column::CronExpression,
        ])
        .filter(event_sink::Column::SinkType.eq("cron"))
        .filter(event_sink::Column::Active.eq(true))
        .into_tuple::<(String, String, bool, Option<String>)>()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let schedules: Vec<CronScheduleInfo> = sinks
        .into_iter()
        .filter_map(|(event_id, app_id, active, cron_expression)| {
            cron_expression.map(|expr| CronScheduleInfo {
                // The docker-compose cron worker keys its in-memory and Redis
                // state by this id. Keep it equal to event_id so last-triggered
                // updates address the same record that schedule sync stores.
                id: event_id.clone(),
                event_id,
                cron_expression: expr,
                app_id,
                enabled: active,
                last_triggered: None,
                next_trigger: None,
            })
        })
        .collect();

    Ok(Json(schedules))
}

/// Cron schedule info returned by list_cron_schedules
#[derive(Debug, Serialize, ToSchema)]
pub struct CronScheduleInfo {
    pub id: String,
    pub event_id: String,
    pub cron_expression: String,
    pub app_id: String,
    pub enabled: bool,
    pub last_triggered: Option<chrono::DateTime<chrono::Utc>>,
    pub next_trigger: Option<chrono::DateTime<chrono::Utc>>,
}

/// Sink config info returned by list_sink_configs
#[derive(Debug, Serialize, ToSchema)]
pub struct SinkConfigInfo {
    pub event_id: String,
    pub app_id: String,
    pub sink_type: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

/// Query parameters for list_sink_configs
#[derive(Debug, Deserialize, ToSchema)]
pub struct SinkConfigsQuery {
    pub sink_type: String,
}

/// GET /sink/configs?sink_type=discord
///
/// List all active sink configs for a specific sink type.
/// Used by sink services (Discord bot, Telegram bot) to sync their configs.
#[utoipa::path(
    get,
    path = "/sink/configs",
    tag = "sink",
    params(
        ("sink_type" = String, Query, description = "Sink type to filter by")
    ),
    responses(
        (status = 200, description = "List of sink configs. A bot that a device runs, or ran until a moment ago, is left out.", body = Vec<SinkConfigInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for sink type")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/configs", skip_all)]
pub async fn get_sink_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SinkConfigsQuery>,
) -> Result<Json<Vec<SinkConfigInfo>>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Only allow tokens with access to the requested sink type
    if !claims.sink_types.contains(&query.sink_type)
        && !claims.sink_types.contains(&"*".to_string())
    {
        return Err(ApiError::forbidden(format!(
            "Token not authorized for sink type: {}",
            query.sink_type
        )));
    }

    // Get all active sinks of the requested type
    let sinks = event_sink::Entity::find()
        .select_only()
        .columns([
            event_sink::Column::EventId,
            event_sink::Column::AppId,
            event_sink::Column::SinkType,
            event_sink::Column::Active,
        ])
        .filter(event_sink::Column::SinkType.eq(&query.sink_type))
        .filter(event_sink::Column::Active.eq(true))
        .into_tuple::<(String, String, String, bool)>()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;
    let sinks = without_device_bots(&state.db, &query.sink_type, sinks).await?;

    if sinks.is_empty() {
        return Ok(Json(Vec::new()));
    }

    // Fetch events to get config data
    let event_ids: Vec<String> = sinks
        .iter()
        .map(|(event_id, ..)| event_id.clone())
        .collect();
    let mut event_configs: std::collections::HashMap<String, serde_json::Value> =
        event::Entity::find()
            .select_only()
            .columns([event::Column::Id, event::Column::Config])
            .filter(event::Column::Id.is_in(event_ids))
            .into_tuple::<(String, Option<serde_json::Value>)>()
            .all(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?
            .into_iter()
            .filter_map(|(id, config)| config.map(|config| (id, config)))
            .collect();

    let configs: Vec<SinkConfigInfo> = sinks
        .into_iter()
        .map(|(event_id, app_id, sink_type, active)| {
            let config = event_configs.remove(&event_id);
            SinkConfigInfo {
                event_id,
                app_id,
                sink_type,
                active,
                config,
            }
        })
        .collect();

    Ok(Json(configs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};

    const TEST_SECRET: &str = "test-sink-secret";

    fn authorization_headers(value: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        }
        headers
    }

    fn sign_test_sink_claims(exp: Option<usize>) -> String {
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = SinkTriggerClaims {
            sub: "sink-trigger".to_string(),
            iss: "flow-like".to_string(),
            jti: None,
            sink_types: vec!["cron".to_string()],
            app_ids: None,
            iat: now,
            exp,
        };

        jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test sink token should encode")
    }

    #[test]
    fn accepts_long_lived_sink_token_without_exp() {
        let token = sign_test_sink_claims(None);
        let claims = validate_sink_trigger_jwt(&token, TEST_SECRET)
            .expect("sink token without exp should validate");

        assert_eq!(claims.sub, "sink-trigger");
        assert_eq!(claims.iss, "flow-like");
        assert_eq!(claims.sink_types, vec!["cron".to_string()]);
        assert_eq!(claims.exp, None);
    }

    #[test]
    fn rejects_expired_sink_token_when_exp_is_present() {
        let expired_at = (chrono::Utc::now() - chrono::Duration::minutes(1)).timestamp() as usize;
        let token = sign_test_sink_claims(Some(expired_at));

        assert!(validate_sink_trigger_jwt(&token, TEST_SECRET).is_err());
    }

    #[test]
    fn http_auth_accepts_valid_token() {
        let headers = authorization_headers(Some(TEST_SECRET));

        assert!(http_auth_token_matches(&headers, TEST_SECRET));
    }

    #[test]
    fn http_auth_rejects_invalid_or_missing_token() {
        let invalid = authorization_headers(Some("best-sink-secret"));

        assert!(!http_auth_token_matches(&invalid, TEST_SECRET));
        assert!(!http_auth_token_matches(&HeaderMap::new(), TEST_SECRET));
    }

    #[test]
    fn http_auth_preserves_bearer_token_normalization() {
        let bearer = authorization_headers(Some("bEaReR   test-sink-secret"));
        let raw = authorization_headers(Some(TEST_SECRET));

        assert!(http_auth_token_matches(&bearer, "Bearer test-sink-secret"));
        assert!(http_auth_token_matches(&raw, "Bearer test-sink-secret"));
    }

    #[test]
    fn telegram_auth_accepts_header_or_query_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            TEST_SECRET.parse().unwrap(),
        );

        assert!(telegram_secret_matches(&headers, None, TEST_SECRET));
        assert!(telegram_secret_matches(
            &HeaderMap::new(),
            Some(TEST_SECRET),
            TEST_SECRET
        ));
    }

    #[test]
    fn telegram_auth_rejects_invalid_or_missing_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            "best-sink-secret".parse().unwrap(),
        );

        assert!(!telegram_secret_matches(&headers, None, TEST_SECRET));
        assert!(!telegram_secret_matches(
            &HeaderMap::new(),
            None,
            TEST_SECRET
        ));
        assert!(!telegram_secret_matches(
            &headers,
            Some(TEST_SECRET),
            TEST_SECRET
        ));
    }

    /// Schedulers read a skipped fire as a success without a run: the AWS function deletes
    /// its schedule only on the 404 "No active sink", and the others retry only a failure.
    #[test]
    fn a_schedule_a_device_runs_is_answered_as_a_success_without_a_run() {
        assert_eq!(
            serde_json::to_value(ServiceTriggerResponse::runs_on_device()).unwrap(),
            serde_json::json!({"success": true, "run_id": null, "skipped": "runs_on_device"})
        );
        let dispatched = ServiceTriggerResponse {
            success: true,
            run_id: Some("run".into()),
            error: None,
            duplicate: false,
            skipped: None,
        };
        assert_eq!(
            serde_json::to_value(&dispatched).unwrap(),
            serde_json::json!({"success": true, "run_id": "run"}),
            "a dispatched trigger answers as it did before"
        );
        let cached_by_an_older_replica: ServiceTriggerResponse =
            serde_json::from_str(r#"{"success":true,"run_id":"run"}"#).unwrap();
        assert_eq!(cached_by_an_older_replica.skipped, None);
    }

    #[test]
    fn the_scheduled_time_is_taken_from_the_trigger_when_it_names_one() {
        let payload = |value: serde_json::Value| serde_json::json!({ "scheduled_for": value });
        assert_eq!(
            scheduled_occurrence(Some(&payload("2026-10-02T02:00:00Z".into()))),
            Some(1_790_906_400)
        );
        assert_eq!(
            scheduled_occurrence(Some(&payload("2026-10-02T04:00:00+02:00".into()))),
            Some(1_790_906_400)
        );
        for unreadable in [
            payload("tomorrow".into()),
            payload(1_790_906_400_i64.into()),
            serde_json::json!({ "other": true }),
            serde_json::json!("2026-10-02T02:00:00Z"),
        ] {
            assert_eq!(scheduled_occurrence(Some(&unreadable)), None);
        }
        assert_eq!(scheduled_occurrence(None), None);
    }

    #[tokio::test]
    async fn only_schedules_and_bots_are_looked_up_and_an_unanswered_lookup_does_not_fire() {
        let db = sea_orm::DatabaseConnection::default();
        let dialect = crate::db::DbDialect::Postgres;
        for sink_type in [
            "http",
            "api",
            "quick_action",
            "generic_form",
            "webhook",
            "user_mail",
        ] {
            assert!(
                !left_to_device(&db, dialect, sink_type, "app", "event", None)
                    .await
                    .expect("no database is asked about a sink that is not a schedule or a bot")
            );
        }
        for sink_type in ["cron", "telegram", "discord"] {
            assert!(
                left_to_device(&db, dialect, sink_type, "app", "event", None)
                    .await
                    .is_err(),
                "a {sink_type} whose place cannot be read is not run"
            );
        }
        assert_eq!(CLAIMED_EVENT_TYPES, ["cron", "telegram", "discord"]);
    }

    async fn answered(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    /// A non-2xx makes Telegram deliver the update again and Discord show a failure, so both
    /// doors answer with a success that starts nothing.
    #[tokio::test]
    async fn a_bot_door_answers_without_a_run_while_a_device_holds_the_bot() {
        assert_eq!(
            answered(telegram_runs_on_device()).await,
            (
                StatusCode::OK,
                serde_json::json!({"triggered":false,"run_id":null,"message":"This bot runs on a device"})
            )
        );
        assert_eq!(
            answered(discord_runs_on_device()).await,
            (
                StatusCode::OK,
                serde_json::json!({"type":4,"data":{"content":"This bot runs on a device and does not answer commands here.","flags":64}})
            )
        );
    }

    /// The configs of other sink types are listed as before, without asking where anything
    /// runs; a bot list whose claims cannot be read is not handed out.
    #[tokio::test]
    async fn only_a_bot_list_asks_which_bots_a_device_runs() {
        let db = sea_orm::DatabaseConnection::default();
        let sinks = vec![(
            "event".to_owned(),
            "app".to_owned(),
            "cron".to_owned(),
            true,
        )];
        for sink_type in ["cron", "http", "webhook"] {
            assert_eq!(
                without_device_bots(&db, sink_type, sinks.clone())
                    .await
                    .unwrap(),
                sinks
            );
        }
        for sink_type in BOT_EVENT_TYPES {
            assert!(
                without_device_bots(&db, sink_type, Vec::new())
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                without_device_bots(&db, sink_type, sinks.clone())
                    .await
                    .is_err(),
                "{sink_type}"
            );
        }
    }

    /// The body of the handler `name` in this file.
    fn handler<'a>(source: &'a str, name: &str) -> &'a str {
        let body = source
            .split_once(&format!("\npub async fn {name}("))
            .unwrap_or_else(|| panic!("handler {name} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    fn in_order(body: &str, name: &str, needles: &[&str]) {
        let positions = needles
            .iter()
            .map(|needle| {
                body.find(needle)
                    .unwrap_or_else(|| panic!("{name} calls {needle}"))
            })
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{name} must run {needles:?} in this order"
        );
    }

    /// Each door asks where the bot runs only after the checks it makes today, so a caller it
    /// turns away learns nothing about it, and before anything that starts a run. Discord's
    /// PING is answered whoever holds the bot: Discord checks the address with it. The
    /// caller is authenticated, with the door's own sink type and secret, before the run-as
    /// user is checked, so an unauthenticated caller can neither pause nor record anything.
    #[test]
    fn each_bot_door_asks_after_its_own_checks_and_before_any_run() {
        let source = include_str!("trigger.rs");
        let telegram = handler(source, "trigger_telegram");
        in_order(
            telegram,
            "trigger_telegram",
            &[
                "is_telegram_ip(&client_ip)",
                "event_sink::Entity::find()",
                ".filter(event_sink::Column::SinkType.eq(sink_types::TELEGRAM))",
                "door_secret(&sink, &event.config)",
                "\"This Telegram trigger has no secret token\"",
                "telegram_secret_matches(",
                "if bot_on_device(&state, &sink).await? {",
                "return Ok(telegram_runs_on_device());",
                "axum::body::to_bytes(",
                "resolve_sink_run_as(&state, &sink)",
                "if is_run_as_refusal(&error) => return Ok(telegram_refused(&error))",
                "create_id()",
                "run.insert(&state.db)",
                "dispatch_async(request)",
            ],
        );
        let discord = handler(source, "trigger_discord");
        in_order(
            discord,
            "trigger_discord",
            &[
                "event_sink::Entity::find()",
                ".filter(event_sink::Column::SinkType.eq(sink_types::DISCORD))",
                "door_secret(&sink, &event.config)",
                "\"This Discord trigger has no public key\"",
                "verify_discord_signature(",
                "if interaction_type == 1 {",
                "\"type\": 1  // PONG",
                "if bot_on_device(&state, &sink).await? {",
                "return Ok(discord_runs_on_device());",
                "resolve_sink_run_as(&state, &sink)",
                "if is_run_as_refusal(&error) => return Ok(discord_refused(&error))",
                "create_id()",
                "run.insert(&state.db)",
                "dispatch_async(request)",
            ],
        );
        let http = handler(source, "trigger_http");
        in_order(
            http,
            "trigger_http",
            &[
                "http_route_sink(&app_id, &normalized_path, Some(&method_str))",
                "http_route_sink(&app_id, &normalized_path, None)",
                "http_auth_token_matches(",
                "resolve_sink_run_as(&state, &sink)",
                "parse_http_request_payload(",
                "run.insert(&state.db)",
            ],
        );
        for (name, body, answer) in [
            (
                "trigger_telegram",
                telegram,
                "return Ok(telegram_runs_on_device());",
            ),
            (
                "trigger_discord",
                discord,
                "return Ok(discord_runs_on_device());",
            ),
        ] {
            assert_eq!(body.matches("bot_on_device(").count(), 1, "{name}");
            let check = body
                .find("if bot_on_device(&state, &sink).await? {")
                .unwrap();
            let skip = &body[check..body.find(answer).unwrap()];
            assert_eq!(
                skip.matches(".await").count(),
                1,
                "{name}: a skip records and writes nothing"
            );
        }
    }

    /// The bot services' list leaves out what a device runs with one claim query for all
    /// listed events, never a lookup per event, and before any event config is read.
    #[test]
    fn the_bot_list_asks_once_for_every_listed_event() {
        let source = include_str!("trigger.rs");
        let configs = handler(source, "get_sink_configs");
        in_order(
            configs,
            "get_sink_configs",
            &[
                "validate_sink_trigger_jwt(",
                "event_sink::Entity::find()",
                "without_device_bots(&state.db, &query.sink_type, sinks)",
                "event::Entity::find()",
            ],
        );
        for lookup in ["runs_on_device(", "left_to_device(", "bot_on_device("] {
            assert!(!configs.contains(lookup), "{lookup}");
        }
        let filter = source
            .split_once("\npub(crate) async fn without_device_bots(")
            .expect("the filter exists")
            .1;
        let filter = filter.split_once("\n}\n").map_or(filter, |(body, _)| body);
        assert_eq!(filter.matches("claimed_events(").count(), 1);
        assert!(!filter.contains("for ") && !filter.contains("runs_on_device("));
    }

    /// The schedulers keep ticking for a schedule a device runs and are told to skip, so
    /// nothing is replayed once it returns to the hub. The skip comes before the event is
    /// read, before any idempotency claim and before any dispatch.
    #[test]
    fn the_schedule_list_keeps_a_claimed_schedule_and_a_skip_precedes_any_dispatch() {
        let source = include_str!("trigger.rs");
        let listing = source
            .split_once("\npub async fn get_cron_sinks(")
            .expect("the schedule list exists")
            .1;
        let listing = listing
            .split_once("\n}\n")
            .map_or(listing, |(body, _)| body);
        assert!(!listing.contains("left_to_device") && !listing.contains("runs_on_device"));
        let funnel = source
            .split_once("\npub async fn trigger_service(")
            .expect("the funnel exists")
            .1;
        let funnel = funnel.split_once("\n}\n").map_or(funnel, |(body, _)| body);
        let skip = funnel.find("left_to_device(").expect("the funnel asks");
        for later in [
            "get_event_from_db(",
            "claim_trigger_idempotency(",
            "trigger_event(",
        ] {
            assert!(skip < funnel.find(later).expect(later), "{later}");
        }
        let skipped = &funnel[skip..funnel.find("get_event_from_db(").unwrap()];
        assert!(
            !skipped.contains("record_trigger_rejection("),
            "a skipped fire is no rejected run"
        );
    }

    fn sink(webhook_secret: Option<&str>) -> event_sink::Model {
        let now = chrono::Utc::now().fixed_offset();
        event_sink::Model {
            id: "sink".into(),
            event_id: "event".into(),
            app_id: "app".into(),
            sink_type: "telegram".into(),
            active: true,
            path: None,
            auth_token: None,
            webhook_secret: webhook_secret.map(str::to_owned),
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: None,
            oauth_tokens_encrypted: None,
            profile_json: None,
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: None,
            run_as_released_at: None,
        }
    }

    /// A bot door verifies the secret stored on the sink, else the one its event config
    /// holds, and without either it has nothing to verify and refuses.
    #[test]
    fn a_bot_door_verifies_the_sink_secret_else_the_event_config() {
        let config = br#"{"webhook_secret":" from-config "}"#;
        assert_eq!(
            door_secret(&sink(Some("from-sink")), config),
            Some("from-sink".to_owned())
        );
        assert_eq!(
            door_secret(&sink(None), config),
            Some("from-config".to_owned())
        );
        assert_eq!(
            door_secret(&sink(Some("  ")), config),
            Some("from-config".to_owned())
        );
        for config in [
            &b""[..],
            br#"{}"#,
            br#"{"webhook_secret":""}"#,
            br#"{"webhook_secret":42}"#,
            b"not json",
        ] {
            assert_eq!(door_secret(&sink(None), config), None);
        }
    }

    /// Both lookups of the HTTP door reach only events typed as HTTP: a page, chat or bot
    /// event keeps its UI route in the same columns, and one that was an HTTP event before
    /// may still hold its method, so neither can be fired past its own door.
    #[test]
    fn the_http_door_serves_http_events_only() {
        use sea_orm::{DbBackend, QueryTrait};
        for (method, method_filter) in [
            (Some("POST"), r#""method" = 'POST'"#),
            (None, r#""method" IS NULL"#),
        ] {
            let sql = http_route_sink("app", "/hook", method)
                .build(DbBackend::Postgres)
                .to_string();
            for fragment in [
                r#"INNER JOIN "public"."Event""#,
                method_filter,
                r#""path" = '/hook'"#,
                r#""active" = TRUE"#,
                r#""eventType" IN ('api', 'http', 'webhook')"#,
            ] {
                assert!(sql.contains(fragment), "{fragment} missing from {sql}");
            }
        }
    }

    /// Each transport answers a refusal so that it stops: Telegram and Discord get a success
    /// that starts nothing, and a scheduler keeps its schedule without retrying.
    #[tokio::test]
    async fn refusals_answer_each_transport_without_a_retry() {
        let refusal = ApiError::coded(
            StatusCode::FORBIDDEN,
            SINK_RUN_AS_REFUSED,
            "This event is paused",
        );
        assert_eq!(
            answered(telegram_refused(&refusal)).await,
            (
                StatusCode::OK,
                serde_json::json!({"triggered":false,"run_id":null,"message":"This event is paused"})
            )
        );
        assert_eq!(
            answered(discord_refused(&refusal)).await,
            (
                StatusCode::OK,
                serde_json::json!({"type":4,"data":{"content":"This event is paused","flags":64}})
            )
        );
        assert_eq!(
            serde_json::to_value(ServiceTriggerResponse::refused()).unwrap(),
            serde_json::json!({"success": true, "run_id": null, "skipped": "refused"})
        );
        assert_eq!(
            serde_json::to_value(ServiceTriggerResponse::inactive()).unwrap(),
            serde_json::json!({"success": true, "run_id": null, "skipped": "inactive"})
        );
        assert_eq!(
            serde_json::to_value(ServiceTriggerResponse::failed("later")).unwrap(),
            serde_json::json!({"success": false, "run_id": null, "error": "later"})
        );
        assert_eq!(
            answered(telegram_unauthorized("no secret")).await,
            (
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"triggered":false,"run_id":null,"message":"no secret"})
            )
        );
        assert_eq!(
            answered(discord_unauthorized("no key")).await,
            (
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"error":"no key"})
            )
        );
    }

    fn function<'a>(source: &'a str, signature: &'a str) -> &'a str {
        let body = source
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    /// The fire-time check writes nothing itself: a legacy row is never bound to its token's
    /// owner, and only a refusal pauses, through the guarded release in the service.
    #[test]
    fn the_run_as_check_binds_nothing() {
        let source = include_str!("trigger.rs");
        for signature in [
            "\npub(crate) async fn resolve_sink_run_as(",
            "\npub(crate) async fn resolve_sink_run_as_for_run(",
            "\npub(crate) async fn sink_run_as_authority(",
            "\nasync fn legacy_pat_owner(",
            "\nasync fn refuse_sink_run(",
            "\nasync fn sink_run_as_unavailable(",
        ] {
            let body = function(source, signature);
            for write in ["update_many()", ".update(", ".insert(", "Expr::value("] {
                assert!(!body.contains(write), "{signature} writes with {write}");
            }
        }
        let unavailable = function(source, "\nasync fn sink_run_as_unavailable(");
        assert!(
            !unavailable.contains("release_"),
            "a server fault pauses nothing"
        );
        let refused = function(source, "\nasync fn refuse_sink_run(");
        in_order(
            refused,
            "refuse_sink_run",
            &[
                "if denial.pauses() {",
                "release_member_sinks(",
                "release_legacy_sink(",
                "record_trigger_rejection(",
                "run_as_refused(denial)",
            ],
        );
    }

    /// The service trigger answers a refusal it did not record: the run-as check already
    /// did, so a schedule never collects a second, generic rejection per fire.
    #[test]
    fn the_service_trigger_records_no_rejection_of_its_own_for_a_refusal() {
        let source = include_str!("trigger.rs");
        let funnel = handler(source, "trigger_service");
        let errors = &funnel[funnel
            .find("Err(e) => match e.downcast_ref::<ApiError>() {")
            .expect("the error arms")..];
        let run_as_arms = &errors[..errors.find("_ => {").expect("the generic arm")];
        assert!(
            run_as_arms.contains("is_run_as_refusal(error) => ServiceTriggerResponse::refused()")
        );
        assert!(run_as_arms.contains("SINK_RUN_AS_UNAVAILABLE"));
        assert!(!run_as_arms.contains("record_trigger_rejection("));
        let lookup = &funnel[funnel.find("event_sink::Entity::find()").unwrap()..];
        let lookup = &lookup[..lookup.find(".one(&state.db)").unwrap()];
        assert!(
            !lookup.contains("Column::Active"),
            "a sink that is off is skipped, not reaped as missing"
        );
        in_order(
            funnel,
            "trigger_service",
            &[
                "state.trigger_idempotency.get(key)",
                "if !sink.active {",
                "ServiceTriggerResponse::inactive()",
                "left_to_device(",
                "trigger_event(",
            ],
        );
    }

    /// A refused retry of a durable transport hands its run id to the rejection only while
    /// no run went out under it, so an already dispatched run is never failed by it.
    #[test]
    fn a_refused_retry_leaves_a_dispatched_run_alone() {
        let source = include_str!("trigger.rs");
        let refused = function(source, "\nasync fn refuse_sink_run(");
        in_order(
            refused,
            "refuse_sink_run",
            &[
                "let run_id = rejection::undispatched_run(state, run_id).await;",
                "record_trigger_rejection(",
            ],
        );
        assert!(!refused.contains("run_id.map(str::to_owned)"));
    }

    /// A fire keeps going only while the sink, read again after its draft was fixed, is on and
    /// runs as the identity the fire took. Its grants and profile may have changed meanwhile.
    /// A person's fire also keeps going on a sink a release paused, never on one switched off.
    #[test]
    fn a_fire_keeps_going_only_while_the_sink_runs_as_it_did() {
        let read = event_sink::Model {
            run_as_user_id: Some("previous".into()),
            ..sink(None)
        };
        let released = event_sink::Model {
            active: false,
            run_as_user_id: None,
            run_as_released_at: Some(chrono::Utc::now().fixed_offset()),
            ..sink(None)
        };
        for (case, read, current, run_as, person) in [
            (
                "released when read",
                &released,
                released.clone(),
                false,
                true,
            ),
            ("released meanwhile", &read, released.clone(), false, false),
            (
                "switched off",
                &read,
                event_sink::Model {
                    active: false,
                    ..read.clone()
                },
                false,
                false,
            ),
            (
                "turned back on by someone else",
                &released,
                event_sink::Model {
                    active: true,
                    run_as_user_id: Some("editor".into()),
                    run_as_released_at: None,
                    ..released.clone()
                },
                false,
                false,
            ),
        ] {
            assert_eq!(
                runs_as_read(read, &current, SinkUse::RunAs),
                run_as,
                "{case}"
            );
            assert_eq!(
                runs_as_read(read, &current, SinkUse::Person),
                person,
                "{case}"
            );
        }
        let cases = [
            ("unchanged", read.clone(), true),
            (
                "grants and profile changed",
                event_sink::Model {
                    oauth_tokens_encrypted: Some("refreshed".into()),
                    profile_json: Some(serde_json::json!({"id": "profile"})),
                    ..read.clone()
                },
                true,
            ),
            (
                "rebound to an editor",
                event_sink::Model {
                    run_as_user_id: Some("editor".into()),
                    ..read.clone()
                },
                false,
            ),
            (
                "legacy token changed",
                event_sink::Model {
                    pat_encrypted: Some("pat".into()),
                    ..read.clone()
                },
                false,
            ),
            (
                "paused",
                event_sink::Model {
                    active: false,
                    ..read.clone()
                },
                false,
            ),
        ];
        for (case, current, keeps_going) in cases {
            for sink_use in [SinkUse::RunAs, SinkUse::Person] {
                assert_eq!(
                    runs_as_read(&read, &current, sink_use),
                    keeps_going,
                    "{case} {sink_use:?}"
                );
            }
        }
    }

    /// A release pauses a sink for runs as the user it ran as; a person's run goes on. A sink a
    /// person switched off stops every run, and so does one the release found switched off.
    #[test]
    fn only_runs_as_the_released_user_stop_at_a_release() {
        let now = chrono::Utc::now().fixed_offset();
        for (case, active, released_at, run_as, person) in [
            ("on", true, None, true, true),
            ("paused by a release", false, Some(now), false, true),
            ("switched off by a person", false, None, false, false),
        ] {
            let sink = event_sink::Model {
                active,
                run_as_released_at: released_at,
                ..sink(None)
            };
            assert_eq!(SinkUse::RunAs.admits(&sink), run_as, "{case}");
            assert_eq!(SinkUse::Person.admits(&sink), person, "{case}");
        }
    }

    /// Every run takes the sign-ins saved with an event that runs as someone. A person's run
    /// leaves the grants of a legacy sink alone, which belong to nobody the hub can resolve
    /// them for, and borrows nothing from a sink without saved sign-ins; a run as the sink's
    /// user always borrows its identity.
    #[test]
    fn a_person_borrows_only_saved_sign_ins_of_someone_the_sink_runs_as() {
        let bound = event_sink::Model {
            run_as_user_id: Some("saver".into()),
            oauth_tokens_encrypted: Some("grants".into()),
            ..sink(None)
        };
        let legacy = event_sink::Model {
            run_as_user_id: None,
            pat_encrypted: Some("pat".into()),
            ..bound.clone()
        };
        let without_grants = event_sink::Model {
            oauth_tokens_encrypted: None,
            ..bound.clone()
        };
        for (case, sink, run_as, person) in [
            ("bound with grants", &bound, true, true),
            ("legacy with grants", &legacy, true, false),
            ("bound without grants", &without_grants, false, false),
        ] {
            assert_eq!(SinkUse::RunAs.takes_saved_grants(sink), run_as, "{case}");
            assert_eq!(SinkUse::Person.takes_saved_grants(sink), person, "{case}");
            assert!(SinkUse::RunAs.borrows_from(sink), "{case}");
            assert_eq!(SinkUse::Person.borrows_from(sink), person, "{case}");
        }
    }

    /// A Latest fire reads the sink again only once its draft is fixed, and the read again is
    /// what decides: the sink it returns, or the retryable refusal.
    #[test]
    fn a_latest_fire_rechecks_the_sink_after_its_draft_is_fixed() {
        let source = include_str!("trigger.rs");
        in_order(
            function(source, "\npub(crate) async fn pin_fire_for("),
            "pin_fire_for",
            &[
                "if target.board_version.is_some() {",
                "ensure_draft_board_snapshot(",
                "sink: recheck_fire_for(state, &sink, sink_use).await?,",
                "board_etag: Some(draft.e_tag.trim().to_owned()),",
            ],
        );
        in_order(
            function(source, "\npub(crate) async fn recheck_fire_for("),
            "recheck_fire_for",
            &[
                "event_sink::Entity::find_by_id(sink.id.clone())",
                "Some(current) if runs_as_read(sink, &current, sink_use) => Ok(current),",
                "_ => Err(fire_overtaken(sink)),",
            ],
        );
        for (wrapper, delegate) in [
            (
                "\npub(crate) async fn pin_fire(",
                "pin_fire_for(state, sink, target, SinkUse::RunAs)",
            ),
            (
                "\npub(crate) async fn recheck_fire(",
                "recheck_fire_for(state, sink, SinkUse::RunAs)",
            ),
        ] {
            assert!(function(source, wrapper).contains(delegate), "{wrapper}");
        }
    }

    /// Every sink fire fixes the draft it runs after it read its body and resolved its target,
    /// and only then takes the sink's grants and profile and starts a run, so an edit saved
    /// meanwhile never runs as the identity the fire read first.
    #[test]
    fn every_fire_pins_its_draft_before_it_uses_the_sink() {
        let source = include_str!("trigger.rs");
        let tail = [
            "resolve_sink_target(",
            "pin_fire(",
            ".oauth_tokens_encrypted",
            "sink_dispatch_profile(",
            "run.insert(&state.db)",
        ];
        let http = handler(source, "trigger_http");
        in_order(
            http,
            "trigger_http",
            &[&["parse_http_request_payload("][..], &tail[..]].concat(),
        );
        let telegram = handler(source, "trigger_telegram");
        in_order(
            telegram,
            "trigger_telegram",
            &[&["axum::body::to_bytes("][..], &tail[..]].concat(),
        );
        let discord = handler(source, "trigger_discord");
        in_order(discord, "trigger_discord", &tail);
        let programmatic = function(source, "\nasync fn trigger_event_dispatch(");
        let as_actor = tail.map(|needle| match needle {
            "pin_fire(" => "pin_fire_for(state, sink, &target, actor.sink_use())",
            ".oauth_tokens_encrypted" => "run_oauth_tokens(",
            "sink_dispatch_profile(" => "actor.profile(state, &sink",
            needle => needle,
        });
        in_order(programmatic, "trigger_event_dispatch", &as_actor);
        for (name, body) in [
            ("trigger_http", http),
            ("trigger_telegram", telegram),
            ("trigger_discord", discord),
            ("trigger_event_dispatch", programmatic),
        ] {
            assert!(
                body.contains("board_etag,") && !body.contains("board_etag: None"),
                "{name} dispatches a draft it did not pin"
            );
        }
    }

    /// The person a run acts as, and the sign-ins they supplied, come from the caller's
    /// authentication, never from a body.
    #[test]
    fn a_trigger_input_never_reads_or_writes_its_actor() {
        let input: TriggerEventInput = serde_json::from_value(serde_json::json!({
            "event_id": "event",
            "payload": {"lat": 1.0},
            "actor": "someone-else",
            "oauth_tokens": {"google": {"access_token": "smuggled"}},
        }))
        .unwrap();
        assert_eq!(input.actor, None);
        assert_eq!(input.oauth_tokens, None);
        let person = TriggerEventInput {
            actor: Some("ada".into()),
            oauth_tokens: Some(OAuthTokens::from([(
                "google".to_owned(),
                serde_json::json!({"access_token": "ada"}),
            )])),
            ..input
        };
        let written = serde_json::to_value(&person).unwrap();
        assert_eq!(written.get("actor"), None);
        assert_eq!(written.get("oauth_tokens"), None);
        assert_eq!(written["event_id"], "event");
    }

    /// Every fire takes the sign-ins saved with the event. A person's fire also takes the ones
    /// the person supplied, and a fire nobody triggered has none of its own. A person's fire may
    /// use a sink a release paused.
    #[test]
    fn a_person_fire_brings_their_sign_ins_and_a_fire_nobody_triggered_none() {
        assert_eq!(FireActor::of(None), FireActor::RunAs);
        assert_eq!(
            FireActor::of(Some("ada".into())),
            FireActor::Person("ada".into())
        );
        let supplied = || {
            Some(OAuthTokens::from([(
                "google".to_owned(),
                serde_json::json!({"access_token": "ada"}),
            )]))
        };
        assert_eq!(
            FireActor::Person("ada".into()).supplied_oauth_tokens(supplied()),
            supplied()
        );
        assert_eq!(FireActor::RunAs.supplied_oauth_tokens(supplied()), None);
        assert_eq!(FireActor::Person("ada".into()).sink_use(), SinkUse::Person);
        assert_eq!(FireActor::RunAs.sink_use(), SinkUse::RunAs);
    }

    /// Provider by provider, a person's own sign-in wins over the grant saved with the event,
    /// and the saved grants fill every provider the person did not sign in to.
    #[test]
    fn a_persons_sign_in_wins_over_the_events_saved_grant_per_provider() {
        let tokens = |entries: &[(&str, &str)]| -> OAuthTokens {
            entries
                .iter()
                .map(|(provider, owner)| {
                    (
                        (*provider).to_owned(),
                        serde_json::json!({"access_token": owner}),
                    )
                })
                .collect()
        };
        let saved = tokens(&[("google", "event"), ("slack", "event")]);
        for (case, supplied, expected) in [
            (
                "no sign-in of their own",
                tokens(&[]),
                tokens(&[("google", "event"), ("slack", "event")]),
            ),
            (
                "one provider",
                tokens(&[("google", "person")]),
                tokens(&[("google", "person"), ("slack", "event")]),
            ),
            (
                "every provider",
                tokens(&[("google", "person"), ("slack", "person")]),
                tokens(&[("google", "person"), ("slack", "person")]),
            ),
            (
                "a provider the event has no grant for",
                tokens(&[("github", "person")]),
                tokens(&[
                    ("github", "person"),
                    ("google", "event"),
                    ("slack", "event"),
                ]),
            ),
        ] {
            assert_eq!(
                person_over_event(saved.clone(), supplied),
                expected,
                "{case}"
            );
        }
        assert_eq!(
            person_over_event(tokens(&[]), tokens(&[("google", "person")])),
            tokens(&[("google", "person")])
        );
    }

    /// A run resolves only the saved grants of providers the person left to the event, so a
    /// saved grant the person replaced never fails their run, and the write-back keeps every
    /// saved grant. Without saved grants the person's own sign-ins are all the run gets.
    #[test]
    fn only_the_saved_grants_a_run_uses_are_resolved() {
        let source = include_str!("trigger.rs");
        let run = function(source, "\npub(crate) async fn run_oauth_tokens(");
        in_order(
            run,
            "run_oauth_tokens",
            &[
                ".filter(|sink| sink_use.takes_saved_grants(sink))",
                "saved_grants(state, sink)",
                "return Ok(supplied);",
                "!supplied.contains_key(provider)",
                "Ok(Some(person_over_event(saved, supplied)))",
            ],
        );
        let refresh = function(source, "\nasync fn refresh_sink_grants(");
        in_order(
            refresh,
            "refresh_sink_grants",
            &[
                ".filter(|(provider_id, _)| wanted(provider_id))",
                "resolve_sink_oauth_token(",
                "serde_json::to_string(&tokens)?",
                "tokens.retain(|provider_id, _| wanted(provider_id));",
            ],
        );
        assert!(
            function(source, "\npub(crate) async fn maybe_refresh_oauth_tokens(")
                .contains("refresh_sink_grants(state, sink, tokens, |_| true)")
        );
    }

    /// A fire nobody triggered is signed unattended, so the apps it calls through connections
    /// never take its run-as user for a person; a person's fire is not. Webhook and bot doors
    /// only fire unattended.
    #[test]
    fn only_a_fire_nobody_triggered_is_signed_unattended() {
        crate::backend_jwt::init_for_tests();
        let params = || ExecutionJwtParams {
            user_id: "run-as".into(),
            technical_user_id: None,
            run_id: "run".into(),
            app_id: "app".into(),
            board_id: "board".into(),
            event_id: Some("event".into()),
            app_chain: None,
            correlation: None,
            callback_url: "http://localhost:8080".into(),
            token_type: TokenType::Executor,
            ttl_seconds: Some(60),
            shadow: None,
        };
        for (actor, unattended) in [
            (FireActor::RunAs, true),
            (FireActor::Person("ada".into()), false),
        ] {
            let token = actor.sign_executor_jwt(params()).unwrap();
            let claims = crate::execution::verify_execution_jwt(&token).unwrap();
            assert_eq!(claims.unattended, unattended, "{actor:?}");
        }

        let source = include_str!("trigger.rs");
        for name in ["trigger_http", "trigger_telegram", "trigger_discord"] {
            let body = handler(source, name);
            assert!(
                body.contains("sign_unattended_execution_jwt(ExecutionJwtParams {"),
                "{name}"
            );
            assert!(!body.contains("sign_execution_jwt("), "{name}");
        }
        let dispatch = function(source, "\nasync fn trigger_event_dispatch(");
        assert!(dispatch.contains("actor.sign_executor_jwt(ExecutionJwtParams {"));
        assert!(!dispatch.contains("sign_execution_jwt("));
    }

    /// A person's fire is judged by the project alone: the sink's run-as user is neither checked
    /// nor paused for it, and a refusal is recorded as the person's. A fire nobody triggered keeps
    /// the run-as check. The run then carries the person's own profile; the sink's saved profile
    /// only serves the run-as user, and its saved grants serve a person only once the draft is
    /// pinned and only for providers the person supplied no sign-in for.
    #[test]
    fn a_person_fire_runs_as_them_and_leaves_the_run_as_user_alone() {
        let source = include_str!("trigger.rs");
        let actor = function(source, "\nasync fn fire_actor(");
        in_order(
            actor,
            "fire_actor",
            &[
                "let FireActor::Person(person) = actor else {",
                "return resolve_sink_run_as_for_run(state, sink, run_id).await;",
                "run_as_authority(&state.db, &sink.app_id, None, execute)",
                "RunAsVerdict::Allowed(_) => Ok(Some(person.clone())),",
                "refuse_person_fire(state, sink, person, denial, run_id)",
                "RunAsVerdict::Transient(_) => Err(retry_later()),",
            ],
        );
        assert_eq!(actor.matches("resolve_sink_run_as").count(), 1);
        let refused = function(source, "\nasync fn refuse_person_fire(");
        in_order(
            refused,
            "refuse_person_fire",
            &[
                "let run_id = rejection::undispatched_run(state, run_id).await;",
                "record_trigger_rejection(",
                "Some(person.to_owned()),",
                "run_as_refused(denial)",
            ],
        );
        for body in [actor, refused] {
            for pause in ["refuse_sink_run(", "release_", "update_many()", ".update("] {
                assert!(!body.contains(pause), "a person's fire pauses with {pause}");
            }
        }

        let profile = function(source, "\n    async fn profile(");
        in_order(
            profile,
            "FireActor::profile",
            &[
                "Self::Person(person) => {",
                "fetch_profile_for_dispatch(state, person, None, &sink.app_id)",
                "Self::RunAs => crate::execution::sink_dispatch_profile(state, sink, run_as)",
            ],
        );

        let dispatch = function(source, "\nasync fn trigger_event_dispatch(");
        for run_as_only in [
            "resolve_sink_run_as",
            ".oauth_tokens_encrypted",
            "sink_dispatch_profile(",
        ] {
            assert!(!dispatch.contains(run_as_only), "{run_as_only}");
        }
        in_order(
            dispatch,
            "trigger_event_dispatch",
            &[
                "let actor = FireActor::of(input.actor);",
                "fire_actor(state, &sink, &actor, transport_run)",
                "pin_fire_for(state, sink, &target, actor.sink_use())",
                "run_oauth_tokens(",
                "actor.supplied_oauth_tokens(input.oauth_tokens)",
                "user_id: executor_subject,",
                "profile: actor.profile(state, &sink, actor_user_id.as_deref()).await,",
                "custom_bit_owner: actor_user_id.clone(),",
                "user_id: Set(actor_user_id),",
            ],
        );
    }
}
