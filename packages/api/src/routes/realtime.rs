//! One bounded poll carries account notifications and independently replayable runs.
//! Notification revisions fingerprint the account's inbox; run cursors acknowledge only
//! events actually returned to this consumer.

use crate::{
    entity::{
        bot_turn, conversation_member, inbox_entry, invitation, notification,
        wasm_package_invitation,
    },
    error::ApiError,
    execution::{
        ExecutionClaims, TokenType,
        state::{
            EventReplay, ExecutionEventRecord, ExecutionRunRecord, ExecutionStateStore,
            ReplayCursor, RunStatus, StateStoreError,
        },
        verify_user_jwt,
    },
    middleware::jwt::{AppUser, fresh_pat_permissions},
    permission::{
        pat_permission::PatPermission,
        role_permission::{RolePermissions, has_role_permission},
    },
    routes::{execution::progress::get_state_store, registry::users::pending_package_invitations},
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, header},
    routing::post,
};
use flow_like_types::{
    anyhow,
    channel::now_unix,
    realtime::{
        MAX_REALTIME_EVENTS, MAX_REALTIME_RESPONSE_BYTES, MAX_REALTIME_RUNS,
        MAX_REALTIME_TIMEOUT_MS, REALTIME_ERROR_NOT_FOUND, REALTIME_ERROR_UNAVAILABLE,
        RealtimeConnectRequest, RealtimeConnectResponse, RealtimeNotificationOverview,
        RealtimeNotificationUpdate, RealtimePollRequest, RealtimePollResponse, RealtimeRunEvent,
        RealtimeRunSubscription, RealtimeRunUpdate, RealtimeSubscriptionError,
        RealtimeTransportPreference,
    },
    tokio::time::{Duration, Instant, sleep_until, timeout},
};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
    QueryTrait,
    prelude::DateTimeWithTimeZone,
    sea_query::{Alias, Expr, Query, SelectStatement},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RUN_UPDATE_BYTES: usize = 256 * 1024;
/// Run status text beyond this many encoded bytes is cut, so a long run error
/// cannot crowd events out of a response. The revision covers the whole text.
const MAX_STATUS_TEXT_BYTES: usize = 8 * 1024;
const MAX_RUN_METADATA_BYTES: usize = 2 * MAX_STATUS_TEXT_BYTES + 1024;
const ERROR_RESERVE_BYTES: usize = MAX_REALTIME_RUNS * 1024;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Clients renew a grant when it expires, and every renewal asks the provider
/// again (an STS AssumeRole on AWS).
const HTTP_FALLBACK_TTL_SECS: i64 = 300;

/// `REALTIME_POLL_MAX_WAIT_MS`. A held poll keeps its Lambda environment busy for
/// the whole wait, so Lambda answers at once unless configured otherwise.
static MAX_POLL_WAIT_MS: LazyLock<u64> = LazyLock::new(|| {
    max_poll_wait_ms(
        std::env::var("REALTIME_POLL_MAX_WAIT_MS").ok().as_deref(),
        std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some(),
    )
});

fn max_poll_wait_ms(configured: Option<&str>, on_lambda: bool) -> u64 {
    configured
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(if on_lambda {
            0
        } else {
            MAX_REALTIME_TIMEOUT_MS
        })
        .min(MAX_REALTIME_TIMEOUT_MS)
}

fn poll_wait(requested_ms: u64, max_wait_ms: u64) -> Duration {
    Duration::from_millis(requested_ms.min(max_wait_ms))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/poll", post(poll))
        .route("/connect", post(connect))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
}

#[utoipa::path(
    post,
    path = "/realtime/connect",
    tag = "realtime",
    request_body(content = Object, description = "RealtimeConnectRequest: transport is configured (default) or http. Account identity and provider topics are derived by the API."),
    responses(
        (status = 200, description = "Receive-only provider grant for account invalidations, with its expiry as a time and as seconds from now, and the longest wait /realtime/poll holds a request (0: it answers at once). Event data is replayed through /realtime/poll. HTTP is returned for five minutes if the provider cannot issue a grant.", body = Object),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "An account credential is required")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /realtime/connect", skip_all)]
pub async fn connect(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<RealtimeConnectRequest>,
) -> Result<(HeaderMap, Json<RealtimeConnectResponse>), ApiError> {
    let sub = user.sub()?;
    if let AppUser::PAT(pat) = &user {
        fresh_pat_permissions(pat, &state).await?;
    }
    let prefer_http = request.transport == RealtimeTransportPreference::Http;
    let attempt = match timeout(
        Duration::from_secs(5),
        state.channels.realtime_connect(&sub, prefer_http),
    )
    .await
    {
        Ok(Ok(grant)) => Ok(grant),
        Ok(Err(error)) => Err(crate::channel::redact_provider_error(&error)),
        Err(_) => Err("timed out".to_owned()),
    };
    let grant = match attempt {
        Ok(grant) => grant,
        Err(reason) => {
            tracing::warn!(
                transport = state.channels.transport(),
                %reason,
                "Realtime grant unavailable; using HTTP replay"
            );
            let mut grant = state
                .channels
                .realtime_connect(&sub, true)
                .await
                .map_err(ApiError::internal_error)?;
            grant.expires_at = now_unix() + HTTP_FALLBACK_TTL_SECS;
            grant
        }
    };
    Ok((
        no_store(),
        Json(with_poll_limits(grant, now_unix(), *MAX_POLL_WAIT_MS)),
    ))
}

fn with_poll_limits(
    mut grant: RealtimeConnectResponse,
    now: i64,
    max_poll_wait_ms: u64,
) -> RealtimeConnectResponse {
    grant.expires_in = Some(u64::try_from(grant.expires_at.saturating_sub(now)).unwrap_or(0));
    grant.max_poll_wait_ms = max_poll_wait_ms;
    grant
}

#[utoipa::path(
    post,
    path = "/realtime/poll",
    tag = "realtime",
    request_body(content = Object, description = "RealtimePollRequest: optional notifications revision, up to 16 run subscriptions with independent after_sequence and revision, and timeout_ms up to 20000, held at most for the max_poll_wait_ms of the connect response. Runs require current ReadBoards and ReadLogs access, or the invocation's poll_token bound to this account and run; an expired or foreign poll_token falls back to the account's access. Capability subscriptions exclude technical-user runs. PATs additionally require current ReadWrite or Admin access for runs; ReadOnly PATs can subscribe to notifications."),
    responses(
        (status = 200, description = "Versioned notification invalidation and run replay batches. Events continue without gaps from after_sequence, and a finished run reports has_more false only after its last event. A run that does not fit this response comes back without events and with has_more true. Runs that cannot be read right now are reported as unavailable (try again later); missing runs and runs this account cannot read as not_found. Notification revisions fingerprint the inbox, not a durable change log.", body = Object),
        (status = 400, description = "Invalid or unbounded subscriptions"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "An account credential is required")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /realtime/poll", skip_all)]
pub async fn poll(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<RealtimePollRequest>,
) -> Result<(HeaderMap, Json<RealtimePollResponse>), ApiError> {
    // A run credential cannot open another account's inbox. The request contains
    // no account identifier; the normal account credential supplies it.
    let sub = user.sub()?;
    request.validate().map_err(ApiError::bad_request)?;
    let deadline = Instant::now() + poll_wait(request.timeout_ms, *MAX_POLL_WAIT_MS);
    let roles = AccountRoles {
        state: &state,
        user: &user,
    };
    let mut readers = RunReaders::new(&sub, runs_allowed(&state, &user, &request).await?, roles);
    let store = run_store(&state, &request).await;

    loop {
        let response = collect(&state.db, &request, store.as_deref(), &mut readers).await?;
        if has_changes(&response) || Instant::now() >= deadline {
            return Ok((no_store(), Json(response)));
        }
        sleep_until(deadline.min(Instant::now() + POLL_INTERVAL)).await;
    }
}

/// PATs read runs only at ReadWrite or above. The middleware already checked that
/// the PAT is current, so a poll without runs asks nothing more.
async fn runs_allowed(
    state: &AppState,
    user: &AppUser,
    request: &RealtimePollRequest,
) -> Result<bool, ApiError> {
    match user {
        AppUser::PAT(pat) if !request.runs.is_empty() => Ok(pat_can_read_run_events(
            fresh_pat_permissions(pat, state).await?,
        )),
        _ => Ok(true),
    }
}

async fn run_store(
    state: &AppState,
    request: &RealtimePollRequest,
) -> Option<Arc<dyn ExecutionStateStore>> {
    if request.runs.is_empty() {
        return None;
    }
    get_state_store(state)
        .await
        .inspect_err(|error| tracing::warn!(error = %error, "Realtime run replay is unavailable"))
        .ok()
}

async fn collect<R: AppRoles>(
    db: &DatabaseConnection,
    request: &RealtimePollRequest,
    store: Option<&dyn ExecutionStateStore>,
    readers: &mut RunReaders<'_, R>,
) -> Result<RealtimePollResponse, ApiError> {
    let mut response = RealtimePollResponse::default();
    if let Some(subscription) = &request.notifications {
        let update = notification_update(db, readers.sub).await?;
        if subscription.revision.as_deref() != Some(update.revision.as_str()) {
            response.notifications = Some(update);
        }
    }
    replay_runs(store, readers, &request.runs, &mut response).await;
    Ok(response)
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers
}

/// The polling account's current role permissions in an app; `None` without membership.
trait AppRoles {
    async fn permissions(&self, app_id: &str) -> Result<Option<RolePermissions>, ApiError>;
}

struct AccountRoles<'a> {
    state: &'a AppState,
    user: &'a AppUser,
}

impl AppRoles for AccountRoles<'_> {
    async fn permissions(&self, app_id: &str) -> Result<Option<RolePermissions>, ApiError> {
        match self.user.app_permission(app_id, self.state).await {
            Ok(permission) => Ok(Some(permission.permissions)),
            Err(error) if error.status().is_client_error() => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// Read decisions for one held poll. A run's owner and app never change, so each
/// run is decided once and each app's role is looked up once.
struct RunReaders<'a, R> {
    sub: &'a str,
    runs_allowed: bool,
    roles: R,
    apps: HashMap<String, Option<RolePermissions>>,
    runs: HashMap<String, bool>,
}

impl<'a, R: AppRoles> RunReaders<'a, R> {
    fn new(sub: &'a str, runs_allowed: bool, roles: R) -> Self {
        Self {
            sub,
            runs_allowed,
            roles,
            apps: HashMap::new(),
            runs: HashMap::new(),
        }
    }

    async fn may_read(
        &mut self,
        run: &ExecutionRunRecord,
        poll_token: Option<&str>,
    ) -> Result<bool, ApiError> {
        if let Some(allowed) = self.runs.get(&run.id) {
            return Ok(*allowed);
        }
        // An invocation capability grants replay of this run without granting log
        // access to the rest of the app. It outlives neither its expiry nor its
        // account; past those the account's own access decides.
        let allowed =
            if poll_token.is_some_and(|token| can_read_with_poll_token(token, run, self.sub)) {
                true
            } else {
                let permissions = match self.apps.get(&run.app_id) {
                    Some(permissions) => *permissions,
                    None => {
                        let permissions = self.roles.permissions(&run.app_id).await?;
                        self.apps.insert(run.app_id.clone(), permissions);
                        permissions
                    }
                };
                permissions.is_some_and(|permissions| {
                    can_read_run(permissions, run.user_id.as_deref(), self.sub)
                })
            };
        self.runs.insert(run.id.clone(), allowed);
        Ok(allowed)
    }
}

async fn replay_runs<R: AppRoles>(
    store: Option<&dyn ExecutionStateStore>,
    readers: &mut RunReaders<'_, R>,
    subscriptions: &[RealtimeRunSubscription],
    response: &mut RealtimePollResponse,
) {
    if subscriptions.is_empty() {
        return;
    }
    let event_limit = MAX_REALTIME_EVENTS / subscriptions.len();
    let mut budget = ResponseBudget::new(response, subscriptions.len());
    for subscription in subscriptions {
        let outcome = match store {
            Some(store) => replay_run(store, readers, subscription, event_limit, &mut budget).await,
            None => Err(REALTIME_ERROR_UNAVAILABLE),
        };
        match outcome {
            // Consumers rotate run order between requests. A deferred run's
            // cursor stays unchanged and its events remain replayable.
            Ok(Some(update)) => response.runs.push(update),
            Ok(None) => {}
            Err(code) => response.errors.push(RealtimeSubscriptionError {
                run_id: subscription.run_id.clone(),
                code: code.to_owned(),
            }),
        }
    }
}

/// One run's update, or the subscription error code it reports instead. Missing
/// and forbidden runs have the same public response.
async fn replay_run<R: AppRoles>(
    store: &dyn ExecutionStateStore,
    readers: &mut RunReaders<'_, R>,
    subscription: &RealtimeRunSubscription,
    event_limit: usize,
    budget: &mut ResponseBudget,
) -> Result<Option<RealtimeRunUpdate>, &'static str> {
    if !readers.runs_allowed {
        return Err(REALTIME_ERROR_NOT_FOUND);
    }
    let run = match store.get_run(&subscription.run_id).await {
        Ok(Some(run)) => run,
        Ok(None) | Err(StateStoreError::NotFound) => return Err(REALTIME_ERROR_NOT_FOUND),
        Err(error) => return Err(unavailable(&subscription.run_id, error)),
    };
    match readers
        .may_read(&run, subscription.poll_token.as_deref())
        .await
    {
        Ok(true) => {}
        Ok(false) => return Err(REALTIME_ERROR_NOT_FOUND),
        Err(error) => return Err(unavailable(&run.id, error)),
    }
    let cursor = ReplayCursor {
        after_sequence: subscription.after_sequence,
        limit: event_limit,
        terminal: run.status.is_terminal(),
    };
    let replay = store
        .replay_events(&run.id, cursor)
        .await
        .map_err(|error| unavailable(&run.id, error))?;
    run_update(&run, subscription, replay, budget).map_err(|error| unavailable(&run.id, error))
}

fn unavailable(run_id: &str, error: impl std::fmt::Display) -> &'static str {
    tracing::warn!(run_id, error = %error, "Realtime run replay is unavailable");
    REALTIME_ERROR_UNAVAILABLE
}

fn can_read_run(permissions: RolePermissions, run_user: Option<&str>, sub: &str) -> bool {
    // Opaque events can contain reply capabilities. Reading another person's
    // run logs must not also grant control over their pending interactions.
    run_user == Some(sub)
        && has_role_permission(
            &permissions,
            RolePermissions::ReadBoards | RolePermissions::ReadLogs,
        )
}

fn poll_token_matches_run(claims: &ExecutionClaims, run: &ExecutionRunRecord, sub: &str) -> bool {
    claims.token_type == TokenType::User
        && claims.sub == sub
        && run.user_id.as_deref() == Some(sub)
        && claims.run_id == run.id
        && claims.app_id == run.app_id
        && claims.board_id == run.board_id
        && claims.technical_user_id.is_none()
        && run.technical_user_id.is_none()
}

fn can_read_with_poll_token(token: &str, run: &ExecutionRunRecord, sub: &str) -> bool {
    verify_user_jwt(token).is_ok_and(|claims| poll_token_matches_run(&claims, run, sub))
}

fn pat_can_read_run_events(permissions: i64) -> bool {
    // Reply credentials embedded in opaque events must not promote a read-only PAT.
    PatPermission::from_bits(permissions).is_some_and(|level| level >= PatPermission::ReadWrite)
}

/// Everything an inbox change moves: notifications are only inserted, marked read
/// or deleted, so their counts and newest timestamps change with every mutation.
/// Invites carry their newest creation time, so replacing one is not a no-op.
/// Conversation entries are only inserted and read markers only move forward, so the
/// newest entry and the newest marker move with every delivery and most reads. A bot turn's
/// update time moves with each queueing, claim and finish of the owner's bots.
#[derive(Debug, FromQueryResult, Serialize)]
struct InboxFingerprint {
    app_invites: i64,
    newest_app_invite: Option<DateTimeWithTimeZone>,
    package_invites: i64,
    newest_package_invite: Option<DateTimeWithTimeZone>,
    notifications: i64,
    unread: i64,
    newest: Option<DateTimeWithTimeZone>,
    last_read: Option<DateTimeWithTimeZone>,
    newest_entry: Option<DateTimeWithTimeZone>,
    last_entry_read: Option<DateTimeWithTimeZone>,
    newest_bot_turn: Option<DateTimeWithTimeZone>,
}

async fn notification_update(
    db: &DatabaseConnection,
    sub: &str,
) -> Result<RealtimeNotificationUpdate, ApiError> {
    let fingerprint = InboxFingerprint::find_by_statement(
        db.get_database_backend()
            .build(&inbox_fingerprint_query(sub)),
    )
    .one(db)
    .await?
    .ok_or_else(|| ApiError::internal_error(anyhow!("Inbox fingerprint returned no row")))?;
    let count = |value: i64| u64::try_from(value).unwrap_or(0);
    Ok(RealtimeNotificationUpdate {
        revision: revision(&fingerprint).map_err(ApiError::internal_error)?,
        overview: RealtimeNotificationOverview {
            invites_count: count(fingerprint.app_invites) + count(fingerprint.package_invites),
            notifications_count: count(fingerprint.notifications),
            unread_count: count(fingerprint.unread),
        },
    })
}

/// One statement per check, so a held poll costs one round trip per pass.
fn inbox_fingerprint_query(sub: &str) -> SelectStatement {
    use sea_orm::sea_query::ExprTrait;

    let notifications =
        || notification::Entity::find().filter(notification::Column::UserId.eq(sub));
    let app_invites = || invitation::Entity::find().filter(invitation::Column::UserId.eq(sub));
    let now = chrono::Utc::now().fixed_offset();
    let mut package_invites = pending_package_invitations(sub, now)
        .select_only()
        .column_as(Expr::col(wasm_package_invitation::Column::Id).count(), "n")
        .into_query();
    package_invites.clear_order_by();
    let mut newest_package_invite = pending_package_invitations(sub, now)
        .select_only()
        .column_as(
            Expr::col(wasm_package_invitation::Column::CreatedAt).max(),
            "n",
        )
        .into_query();
    newest_package_invite.clear_order_by();
    Query::select()
        .expr_as(
            app_invites()
                .select_only()
                .column_as(Expr::col(invitation::Column::Id).count(), "n")
                .into_query(),
            Alias::new("app_invites"),
        )
        .expr_as(
            app_invites()
                .select_only()
                .column_as(Expr::col(invitation::Column::CreatedAt).max(), "n")
                .into_query(),
            Alias::new("newest_app_invite"),
        )
        .expr_as(package_invites, Alias::new("package_invites"))
        .expr_as(newest_package_invite, Alias::new("newest_package_invite"))
        .expr_as(
            notifications()
                .select_only()
                .column_as(Expr::col(notification::Column::Id).count(), "n")
                .into_query(),
            Alias::new("notifications"),
        )
        .expr_as(
            notifications()
                .select_only()
                .column_as(Expr::col(notification::Column::Id).count(), "n")
                .filter(notification::Column::Read.eq(false))
                .into_query(),
            Alias::new("unread"),
        )
        .expr_as(
            notifications()
                .select_only()
                .column_as(Expr::col(notification::Column::CreatedAt).max(), "n")
                .into_query(),
            Alias::new("newest"),
        )
        .expr_as(
            notifications()
                .select_only()
                .column_as(Expr::col(notification::Column::ReadAt).max(), "n")
                .into_query(),
            Alias::new("last_read"),
        )
        .expr_as(
            inbox_entry::Entity::find()
                .select_only()
                .column_as(Expr::col(inbox_entry::Column::CreatedAt).max(), "n")
                .filter(crate::routes::inbox::watched_inboxes(
                    inbox_entry::Column::InboxId,
                    sub,
                ))
                .into_query(),
            Alias::new("newest_entry"),
        )
        .expr_as(
            conversation_member::Entity::find()
                .select_only()
                .column_as(
                    Expr::col(conversation_member::Column::LastReadAt).max(),
                    "n",
                )
                .filter(crate::routes::inbox::watched_inboxes(
                    conversation_member::Column::PrincipalId,
                    sub,
                ))
                .into_query(),
            Alias::new("last_entry_read"),
        )
        .expr_as(
            bot_turn::Entity::find()
                .select_only()
                .column_as(Expr::col(bot_turn::Column::UpdatedAt).max(), "n")
                .filter(bot_turn::Column::OwnerId.eq(sub))
                .into_query(),
            Alias::new("newest_bot_turn"),
        )
        .take()
}

fn revision(value: &impl Serialize) -> flow_like_types::Result<String> {
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, value)?;
    Ok(hasher.finalize().to_hex().to_string())
}

struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The bytes `value` adds to a JSON response, counted without buffering them.
fn encoded_len(value: &impl Serialize) -> usize {
    let mut count = ByteCount(0);
    serde_json::to_writer(&mut count, value).map_or(usize::MAX, |()| count.0)
}

fn bounded_text(text: &str) -> String {
    let mut encoded = 2;
    for (index, character) in text.char_indices() {
        encoded += match character {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            control if control < ' ' => 6,
            other => other.len_utf8(),
        };
        if encoded > MAX_STATUS_TEXT_BYTES {
            return text[..index].to_owned();
        }
    }
    text.to_owned()
}

/// Event bytes a response can still take once its envelope, the metadata of every
/// subscribed run and every possible subscription error are set aside.
struct ResponseBudget {
    /// What one event may take when no other run has used any.
    capacity: usize,
    remaining: usize,
}

impl ResponseBudget {
    fn new(envelope: &RealtimePollResponse, runs: usize) -> Self {
        let reserved = encoded_len(envelope)
            .saturating_add(ERROR_RESERVE_BYTES)
            .saturating_add(runs.saturating_mul(MAX_RUN_METADATA_BYTES));
        let capacity = MAX_REALTIME_RESPONSE_BYTES.saturating_sub(reserved);
        Self {
            capacity,
            remaining: capacity,
        }
    }

    fn take(&mut self, bytes: usize) -> bool {
        let fits = bytes <= self.remaining;
        if fits {
            self.remaining -= bytes;
        }
        fits
    }
}

fn status_name(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "PENDING",
        RunStatus::Running => "RUNNING",
        RunStatus::Completed => "COMPLETED",
        RunStatus::Failed => "FAILED",
        RunStatus::Cancelled => "CANCELLED",
        RunStatus::Timeout => "TIMEOUT",
    }
}

fn realtime_event(event: ExecutionEventRecord) -> flow_like_types::Result<RealtimeRunEvent> {
    let created_at = chrono::DateTime::from_timestamp_millis(event.created_at)
        .map(|time| time.to_rfc3339())
        .ok_or_else(|| anyhow!("Invalid execution event timestamp {}", event.created_at))?;
    Ok(RealtimeRunEvent {
        event_id: Some(event.id),
        sequence: event.sequence,
        event_type: event.event_type,
        payload: event.payload,
        created_at,
    })
}

fn run_update(
    run: &ExecutionRunRecord,
    subscription: &RealtimeRunSubscription,
    replay: EventReplay,
    budget: &mut ResponseBudget,
) -> flow_like_types::Result<Option<RealtimeRunUpdate>> {
    let status = status_name(&run.status);
    let revision = run_revision(run, status, replay.has_more)?;
    if replay.events.is_empty() && subscription.revision.as_deref() == Some(revision.as_str()) {
        return Ok(None);
    }

    let stored_has_more = replay.has_more;
    let mut update = RealtimeRunUpdate {
        run_id: run.id.clone(),
        revision,
        status: status.to_owned(),
        progress: run.progress,
        current_step: run.current_step.as_deref().map(bounded_text),
        error: run.error_message.as_deref().map(bounded_text),
        events: Vec::new(),
        after_sequence: subscription.after_sequence,
        has_more: replay.has_more,
    };
    let mut run_bytes = encoded_len(&update);
    for event in replay.events {
        if event.run_id != run.id
            || event.sequence <= update.after_sequence
            || event.sequence == i32::MAX
        {
            return Err(anyhow!(
                "Execution store returned an invalid realtime event sequence"
            ));
        }
        let event = realtime_event(event)?;
        let size = encoded_len(&event).saturating_add(1);
        let first = update.events.is_empty();
        if first && size > budget.capacity {
            return Err(anyhow!(
                "Execution event {} encodes to {size} bytes, more than the {} bytes a realtime response holds for events",
                event.sequence,
                budget.capacity
            ));
        }
        // The soft cap keeps one busy run from starving the others, but a run's
        // next event always goes out, alone if need be.
        if (!first && run_bytes.saturating_add(size) > MAX_RUN_UPDATE_BYTES) || !budget.take(size) {
            update.has_more = true;
            break;
        }
        run_bytes += size;
        update.after_sequence = event.sequence;
        update.events.push(event);
    }
    if update.has_more != stored_has_more {
        update.revision = run_revision(run, status, update.has_more)?;
    }
    Ok(Some(update))
}

/// A terminal run's closing page can carry no events, so completion is part of its revision.
fn run_revision(
    run: &ExecutionRunRecord,
    status: &str,
    has_more: bool,
) -> flow_like_types::Result<String> {
    revision(&(
        &run.id,
        status,
        run.progress,
        &run.current_step,
        &run.error_message,
        run.status.is_terminal() && !has_more,
    ))
}

fn has_changes(response: &RealtimePollResponse) -> bool {
    response.notifications.is_some() || !response.runs.is_empty() || !response.errors.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{
        ExecutionJwtParams, sign_execution_jwt,
        state::{CreateRunInput, EventQuery, RunMode, RunVariant, UpdateRunInput, replay_prefix},
    };
    use flow_like_types::realtime::RealtimeTransport;
    use serde_json::json;
    use std::cell::Cell;

    /// axum's default `DefaultBodyLimit`, which `POST /execution/events` keeps.
    const EVENT_CALLBACK_BODY_LIMIT: usize = 2 * 1024 * 1024;

    fn run() -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run-a".into(),
            board_id: "board-a".into(),
            version: None,
            event_id: None,
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
            started_at: Some(1000),
            completed_at: None,
            expires_at: None,
            user_id: Some("user-a".into()),
            technical_user_id: None,
            app_id: "app-a".into(),
            created_at: 1000,
            updated_at: 1000,
        }
    }

    fn named(id: &str) -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: id.into(),
            ..run()
        }
    }

    fn subscription() -> RealtimeRunSubscription {
        RealtimeRunSubscription {
            run_id: "run-a".into(),
            poll_token: None,
            after_sequence: -1,
            revision: None,
        }
    }

    fn subscribe(run_id: &str) -> RealtimeRunSubscription {
        RealtimeRunSubscription {
            run_id: run_id.into(),
            ..subscription()
        }
    }

    fn event(sequence: i32, payload: serde_json::Value) -> ExecutionEventRecord {
        event_of("run-a", sequence, payload)
    }

    fn event_of(run_id: &str, sequence: i32, payload: serde_json::Value) -> ExecutionEventRecord {
        ExecutionEventRecord {
            id: format!("event-{sequence}"),
            run_id: run_id.into(),
            sequence,
            event_type: "chunk".into(),
            payload,
            delivered: false,
            expires_at: 100_000,
            created_at: 1000,
        }
    }

    fn page(events: Vec<ExecutionEventRecord>, has_more: bool) -> EventReplay {
        EventReplay { events, has_more }
    }

    fn budget() -> ResponseBudget {
        ResponseBudget::new(&RealtimePollResponse::default(), 1)
    }

    fn changed(update: flow_like_types::Result<Option<RealtimeRunUpdate>>) -> RealtimeRunUpdate {
        update.unwrap().expect("expected an update")
    }

    fn token_params() -> ExecutionJwtParams {
        ExecutionJwtParams {
            user_id: "user-a".into(),
            technical_user_id: None,
            run_id: "run-a".into(),
            app_id: "app-a".into(),
            board_id: "board-a".into(),
            event_id: None,
            app_chain: None,
            correlation: None,
            callback_url: String::new(),
            token_type: TokenType::User,
            ttl_seconds: Some(300),
            shadow: None,
        }
    }

    struct Roles {
        members: HashMap<&'static str, RolePermissions>,
        offline: Option<&'static str>,
        lookups: Cell<usize>,
    }

    impl Roles {
        fn of(members: &[(&'static str, RolePermissions)]) -> Self {
            Self {
                members: members.iter().copied().collect(),
                offline: None,
                lookups: Cell::new(0),
            }
        }
    }

    impl AppRoles for Roles {
        async fn permissions(&self, app_id: &str) -> Result<Option<RolePermissions>, ApiError> {
            self.lookups.set(self.lookups.get() + 1);
            if self.offline == Some(app_id) {
                return Err(ApiError::internal_error(anyhow!(
                    "membership lookup failed"
                )));
            }
            Ok(self.members.get(app_id).copied())
        }
    }

    fn member() -> Roles {
        Roles::of(&[(
            "app-a",
            RolePermissions::ReadBoards | RolePermissions::ReadLogs,
        )])
    }

    /// Runs map to `None` when reading them fails.
    #[derive(Debug, Default)]
    struct Store {
        runs: HashMap<String, Option<ExecutionRunRecord>>,
        events: Vec<ExecutionEventRecord>,
    }

    impl Store {
        fn with(runs: Vec<ExecutionRunRecord>, events: Vec<ExecutionEventRecord>) -> Self {
            Self {
                runs: runs
                    .into_iter()
                    .map(|run| (run.id.clone(), Some(run)))
                    .collect(),
                events,
            }
        }
    }

    #[async_trait::async_trait]
    impl ExecutionStateStore for Store {
        fn backend_name(&self) -> &'static str {
            "test"
        }

        async fn create_run(
            &self,
            _input: CreateRunInput,
        ) -> Result<ExecutionRunRecord, StateStoreError> {
            unimplemented!()
        }

        async fn get_run(
            &self,
            run_id: &str,
        ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
            match self.runs.get(run_id) {
                Some(Some(run)) => Ok(Some(run.clone())),
                Some(None) => Err(StateStoreError::Connection("store offline".into())),
                None => Ok(None),
            }
        }

        async fn get_run_for_app(
            &self,
            _run_id: &str,
            _app_id: &str,
        ) -> Result<Option<ExecutionRunRecord>, StateStoreError> {
            unimplemented!()
        }

        async fn update_run(
            &self,
            _run_id: &str,
            _input: UpdateRunInput,
        ) -> Result<ExecutionRunRecord, StateStoreError> {
            unimplemented!()
        }

        async fn list_runs_for_app(
            &self,
            _app_id: &str,
            _limit: i32,
            _cursor: Option<&str>,
        ) -> Result<Vec<ExecutionRunRecord>, StateStoreError> {
            unimplemented!()
        }

        async fn delete_expired_runs(&self) -> Result<i64, StateStoreError> {
            unimplemented!()
        }

        async fn push_events(
            &self,
            _events: Vec<crate::execution::state::CreateEventInput>,
        ) -> Result<i32, StateStoreError> {
            unimplemented!()
        }

        async fn get_events(
            &self,
            query: EventQuery,
        ) -> Result<Vec<ExecutionEventRecord>, StateStoreError> {
            Ok(self
                .events
                .iter()
                .filter(|event| event.run_id == query.run_id)
                .filter(|event| {
                    query
                        .after_sequence
                        .is_none_or(|after| event.sequence > after)
                })
                .take(query.limit.map_or(usize::MAX, |limit| limit as usize))
                .cloned()
                .collect())
        }

        async fn replay_events(
            &self,
            run_id: &str,
            cursor: ReplayCursor,
        ) -> Result<EventReplay, StateStoreError> {
            replay_prefix(self, run_id, cursor).await
        }

        async fn get_max_sequence(&self, _run_id: &str) -> Result<i32, StateStoreError> {
            unimplemented!()
        }

        async fn mark_events_delivered(
            &self,
            _run_id: &str,
            _event_ids: &[String],
        ) -> Result<(), StateStoreError> {
            unimplemented!()
        }

        async fn delete_expired_events(&self) -> Result<i64, StateStoreError> {
            unimplemented!()
        }
    }

    async fn replay_all(
        store: Option<&dyn ExecutionStateStore>,
        readers: &mut RunReaders<'_, Roles>,
        subscriptions: &[RealtimeRunSubscription],
    ) -> RealtimePollResponse {
        let mut response = RealtimePollResponse::default();
        replay_runs(store, readers, subscriptions, &mut response).await;
        response
    }

    fn errors(response: &RealtimePollResponse) -> Vec<(&str, &str)> {
        response
            .errors
            .iter()
            .map(|error| (error.run_id.as_str(), error.code.as_str()))
            .collect()
    }

    #[test]
    fn lambda_answers_at_once_and_every_hold_is_clamped() {
        assert_eq!(max_poll_wait_ms(None, true), 0);
        assert_eq!(max_poll_wait_ms(None, false), MAX_REALTIME_TIMEOUT_MS);
        assert_eq!(max_poll_wait_ms(Some("5000"), true), 5000);
        assert_eq!(max_poll_wait_ms(Some(" 1500 "), false), 1500);
        assert_eq!(
            max_poll_wait_ms(Some("600000"), false),
            MAX_REALTIME_TIMEOUT_MS
        );
        assert_eq!(max_poll_wait_ms(Some("soon"), true), 0);
        assert_eq!(poll_wait(MAX_REALTIME_TIMEOUT_MS, 0), Duration::ZERO);
        assert_eq!(poll_wait(20_000, 5_000), Duration::from_millis(5_000));
        assert_eq!(poll_wait(1_000, 5_000), Duration::from_millis(1_000));
    }

    #[test]
    fn every_grant_states_its_lifetime_and_the_poll_hold() {
        let grant = RealtimeConnectResponse {
            protocol_version: 1,
            expires_at: 1_900,
            expires_in: None,
            max_poll_wait_ms: MAX_REALTIME_TIMEOUT_MS,
            transport: RealtimeTransport::Http {},
        };
        let stated = with_poll_limits(grant.clone(), 1_000, 0);
        assert_eq!(stated.expires_in, Some(900));
        assert_eq!(stated.max_poll_wait_ms, 0);
        let lapsed = with_poll_limits(
            RealtimeConnectResponse {
                expires_at: 900,
                ..grant
            },
            1_000,
            MAX_REALTIME_TIMEOUT_MS,
        );
        assert_eq!(lapsed.expires_in, Some(0));
        assert_eq!(lapsed.max_poll_wait_ms, MAX_REALTIME_TIMEOUT_MS);
    }

    #[test]
    fn execution_permission_alone_does_not_reveal_other_run_output() {
        assert!(!can_read_run(
            RolePermissions::ExecuteEvents | RolePermissions::ExecuteBoards,
            Some("user-a"),
            "user-a",
        ));
        assert!(!can_read_run(
            RolePermissions::ReadLogs,
            Some("user-a"),
            "user-a"
        ));
        assert!(!can_read_run(
            RolePermissions::ReadBoards,
            Some("user-a"),
            "user-a"
        ));
        assert!(can_read_run(
            RolePermissions::ReadBoards | RolePermissions::ReadLogs,
            Some("user-a"),
            "user-a"
        ));
        assert!(can_read_run(
            RolePermissions::Owner,
            Some("user-a"),
            "user-a"
        ));
        assert!(can_read_run(
            RolePermissions::Admin,
            Some("user-a"),
            "user-a"
        ));
    }

    #[test]
    fn app_admin_cannot_obtain_another_users_reply_capabilities() {
        for permissions in [
            RolePermissions::Owner,
            RolePermissions::Admin,
            RolePermissions::ReadBoards | RolePermissions::ReadLogs,
        ] {
            assert!(!can_read_run(permissions, Some("user-b"), "user-a"));
            assert!(!can_read_run(permissions, None, "user-a"));
        }
    }

    #[test]
    fn readonly_and_legacy_pats_cannot_obtain_reply_capabilities() {
        assert!(!pat_can_read_run_events(PatPermission::ReadOnly.bits()));
        assert!(pat_can_read_run_events(PatPermission::ReadWrite.bits()));
        assert!(pat_can_read_run_events(PatPermission::Admin.bits()));
        for invalid in [0, 3, 5, 6, 7, i64::MAX, -1] {
            assert!(!pat_can_read_run_events(invalid));
        }
    }

    #[test]
    fn invocation_capability_matches_only_its_account_run_and_app() {
        let claims: ExecutionClaims = serde_json::from_value(json!({
            "sub": "user-a", "run_id": "run-a", "app_id": "app-a", "board_id": "board-a",
            "callback_url": "", "typ": "user", "iss": "test", "aud": "test",
            "iat": 1, "nbf": 1, "exp": 4_102_444_800_i64, "jti": "test-token"
        }))
        .unwrap();
        let mut stored = run();
        assert!(poll_token_matches_run(&claims, &stored, "user-a"));
        assert!(!poll_token_matches_run(&claims, &stored, "user-b"));
        for field in ["sub", "run_id", "app_id", "board_id", "technical_user_id"] {
            let mut value = serde_json::to_value(&claims).unwrap();
            value[field] = json!("different-authority");
            let foreign = serde_json::from_value(value).unwrap();
            assert!(
                !poll_token_matches_run(&foreign, &stored, "user-a"),
                "{field}"
            );
        }
        let mut executor = claims.clone();
        executor.token_type = TokenType::Executor;
        assert!(!poll_token_matches_run(&executor, &stored, "user-a"));
        stored.user_id = Some("user-b".into());
        assert!(!poll_token_matches_run(&claims, &stored, "user-a"));
        stored.user_id = None;
        assert!(!poll_token_matches_run(&claims, &stored, "user-a"));
        stored.user_id = Some("user-a".into());
        stored.technical_user_id = Some("api-key".into());
        assert!(!poll_token_matches_run(&claims, &stored, "user-a"));
    }

    #[test]
    fn replay_verifies_the_signed_user_capability_and_expiry() {
        crate::backend_jwt::init_for_tests();
        let params = token_params();
        let token = sign_execution_jwt(params.clone()).unwrap();
        assert!(can_read_with_poll_token(&token, &run(), "user-a"));
        assert!(!can_read_with_poll_token(&token, &run(), "user-b"));
        assert!(!can_read_with_poll_token(
            &format!("{token}.tampered"),
            &run(),
            "user-a"
        ));
        for invalid in [
            ExecutionJwtParams {
                ttl_seconds: Some(-300),
                ..params.clone()
            },
            ExecutionJwtParams {
                token_type: TokenType::Executor,
                ..params.clone()
            },
            ExecutionJwtParams {
                run_id: "run-b".into(),
                ..params.clone()
            },
            ExecutionJwtParams {
                technical_user_id: Some("api-key".into()),
                ..params
            },
        ] {
            let token = sign_execution_jwt(invalid).unwrap();
            assert!(!can_read_with_poll_token(&token, &run(), "user-a"));
        }
    }

    #[tokio::test]
    async fn a_lapsed_or_foreign_poll_token_falls_back_to_the_accounts_access() {
        crate::backend_jwt::init_for_tests();
        let expired = sign_execution_jwt(ExecutionJwtParams {
            ttl_seconds: Some(-300),
            ..token_params()
        })
        .unwrap();
        let foreign = sign_execution_jwt(ExecutionJwtParams {
            run_id: "run-b".into(),
            ..token_params()
        })
        .unwrap();
        for token in [expired.as_str(), foreign.as_str(), "not-a-token"] {
            let mut readers = RunReaders::new("user-a", true, member());
            assert!(readers.may_read(&run(), Some(token)).await.unwrap());
            let mut outsiders = RunReaders::new("user-a", true, Roles::of(&[]));
            assert!(!outsiders.may_read(&run(), Some(token)).await.unwrap());
        }

        let valid = sign_execution_jwt(token_params()).unwrap();
        let mut readers = RunReaders::new("user-a", true, Roles::of(&[]));
        assert!(readers.may_read(&run(), Some(&valid)).await.unwrap());
        assert_eq!(readers.roles.lookups.get(), 0);
    }

    #[tokio::test]
    async fn a_held_poll_looks_up_each_app_once() {
        let mut readers = RunReaders::new("user-a", true, member());
        for _ in 0..3 {
            assert!(readers.may_read(&run(), None).await.unwrap());
            assert!(readers.may_read(&named("run-b"), None).await.unwrap());
        }
        assert_eq!(readers.roles.lookups.get(), 1);
    }

    #[tokio::test]
    async fn one_runs_failure_never_fails_the_poll_or_the_other_runs() {
        let mut store = Store::with(
            vec![
                run(),
                ExecutionRunRecord {
                    user_id: Some("user-b".into()),
                    ..named("run-foreign")
                },
                ExecutionRunRecord {
                    app_id: "app-down".into(),
                    ..named("run-down")
                },
            ],
            vec![event(0, json!("hello"))],
        );
        store.runs.insert("run-offline".into(), None);
        let mut readers = RunReaders::new(
            "user-a",
            true,
            Roles {
                offline: Some("app-down"),
                ..member()
            },
        );
        let subscriptions = [
            "run-offline",
            "run-a",
            "run-missing",
            "run-foreign",
            "run-down",
        ]
        .map(subscribe);
        let response = replay_all(Some(&store), &mut readers, &subscriptions).await;
        assert_eq!(response.runs.len(), 1);
        assert_eq!(response.runs[0].run_id, "run-a");
        assert_eq!(response.runs[0].events[0].payload, json!("hello"));
        assert_eq!(
            errors(&response),
            [
                ("run-offline", REALTIME_ERROR_UNAVAILABLE),
                ("run-missing", REALTIME_ERROR_NOT_FOUND),
                ("run-foreign", REALTIME_ERROR_NOT_FOUND),
                ("run-down", REALTIME_ERROR_UNAVAILABLE),
            ]
        );

        let response = replay_all(None, &mut readers, &subscriptions[..2]).await;
        assert_eq!(
            errors(&response),
            [
                ("run-offline", REALTIME_ERROR_UNAVAILABLE),
                ("run-a", REALTIME_ERROR_UNAVAILABLE),
            ]
        );

        let mut readonly = RunReaders::new("user-a", false, member());
        let response = replay_all(Some(&store), &mut readonly, &subscriptions[1..2]).await;
        assert_eq!(errors(&response), [("run-a", REALTIME_ERROR_NOT_FOUND)]);
    }

    #[test]
    fn account_credentials_are_required_before_polling() {
        assert_eq!(AppUser::Unauthorized.sub().unwrap_err().status(), 401);
        assert_eq!(
            AppUser::APIKey(crate::middleware::jwt::ApiKey {
                key_id: "key".into(),
                api_key: "secret".into(),
                app_id: "app-a".into(),
                creator_user_id: Some("user-a".into()),
            })
            .sub()
            .unwrap_err()
            .status(),
            403
        );
    }

    #[test]
    fn cursor_advances_only_over_the_returned_prefix() {
        let events = (0..2)
            .map(|sequence| event(sequence, json!(sequence)))
            .collect();
        let update = changed(run_update(
            &run(),
            &subscription(),
            page(events, true),
            &mut budget(),
        ));
        assert_eq!(update.events.len(), 2);
        assert_eq!(update.events[0].event_id.as_deref(), Some("event-0"));
        assert_eq!(update.after_sequence, 1);
        assert!(update.has_more);

        let mut second_consumer = subscription();
        second_consumer.after_sequence = 0;
        let update = changed(run_update(
            &run(),
            &second_consumer,
            page(vec![event(1, json!(1))], false),
            &mut budget(),
        ));
        assert_eq!(update.events[0].sequence, 1);
        assert!(!update.has_more);
        assert_eq!(second_consumer.after_sequence, 0);
    }

    #[test]
    fn state_revisions_do_not_acknowledge_events_or_hide_terminal_changes() {
        let first = changed(run_update(
            &run(),
            &subscription(),
            EventReplay::default(),
            &mut budget(),
        ));
        let mut subscription = subscription();
        subscription.revision = Some(first.revision);
        assert!(
            run_update(&run(), &subscription, EventReplay::default(), &mut budget())
                .unwrap()
                .is_none()
        );
        assert!(
            run_update(
                &run(),
                &subscription,
                page(vec![event(0, json!("late"))], false),
                &mut budget()
            )
            .unwrap()
            .is_some()
        );
        let mut finished = run();
        finished.status = RunStatus::Completed;
        let update = changed(run_update(
            &finished,
            &subscription,
            EventReplay::default(),
            &mut budget(),
        ));
        assert_eq!(update.status, "COMPLETED");
        assert_eq!(update.after_sequence, -1);
        assert!(!update.has_more);
    }

    #[test]
    fn a_page_cut_by_the_soft_cap_does_not_claim_completion() {
        let mut finished = run();
        finished.status = RunStatus::Completed;
        let half = || json!("x".repeat(MAX_RUN_UPDATE_BYTES / 2 + 1));
        let cut = changed(run_update(
            &finished,
            &subscription(),
            page(vec![event(0, half()), event(1, half())], false),
            &mut budget(),
        ));
        assert!(cut.has_more);
        assert_eq!(cut.events.len(), 1);
        assert_eq!(
            cut.revision,
            run_revision(&finished, "COMPLETED", true).unwrap()
        );
    }

    #[test]
    fn a_finished_run_reports_completion_after_a_last_page_that_had_more() {
        let mut finished = run();
        finished.status = RunStatus::Completed;
        let last = changed(run_update(
            &finished,
            &subscription(),
            page(vec![event(0, json!("last"))], true),
            &mut budget(),
        ));
        assert!(last.has_more);
        let mut subscription = subscription();
        subscription.revision = Some(last.revision);
        subscription.after_sequence = 0;
        let closing = changed(run_update(
            &finished,
            &subscription,
            EventReplay::default(),
            &mut budget(),
        ));
        assert!(closing.events.is_empty());
        assert!(!closing.has_more);
        subscription.revision = Some(closing.revision);
        assert!(
            run_update(
                &finished,
                &subscription,
                EventReplay::default(),
                &mut budget()
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn foreign_and_out_of_order_store_events_are_never_delivered() {
        let mut foreign = event(0, json!("secret"));
        foreign.run_id = "other-run".into();
        assert!(
            run_update(
                &run(),
                &subscription(),
                page(vec![foreign], false),
                &mut budget()
            )
            .is_err()
        );
        assert!(
            run_update(
                &run(),
                &subscription(),
                page(vec![event(1, json!(1)), event(0, json!(0))], false),
                &mut budget()
            )
            .is_err()
        );
    }

    #[test]
    fn an_event_over_the_run_cap_is_delivered_alone() {
        let huge = json!("x".repeat(MAX_RUN_UPDATE_BYTES));
        let update = changed(run_update(
            &run(),
            &subscription(),
            page(
                vec![event(0, huge.clone()), event(1, json!("small"))],
                false,
            ),
            &mut budget(),
        ));
        assert_eq!(update.events.len(), 1);
        assert_eq!(update.events[0].payload, huge);
        assert_eq!(update.after_sequence, 0);
        assert!(update.has_more);

        let update = changed(run_update(
            &run(),
            &subscription(),
            page(vec![event(0, json!("small")), event(1, huge)], false),
            &mut budget(),
        ));
        assert_eq!(update.after_sequence, 0);
        assert_eq!(update.events.len(), 1);
        assert!(update.has_more);
    }

    #[test]
    fn a_run_past_the_response_budget_is_deferred_without_an_error() {
        let mut budget = budget();
        budget.remaining = 64;
        let mut subscription = subscription();
        subscription.after_sequence = 4;
        let update = changed(run_update(
            &run(),
            &subscription,
            page(vec![event(5, json!("x".repeat(1024)))], false),
            &mut budget,
        ));
        assert!(update.events.is_empty());
        assert_eq!(update.after_sequence, 4);
        assert!(update.has_more);
        assert_eq!(budget.remaining, 64);
    }

    #[test]
    fn an_event_no_response_could_carry_is_reported_instead_of_retried() {
        let mut budget = budget();
        let beyond = json!("x".repeat(budget.capacity));
        assert!(
            run_update(
                &run(),
                &subscription(),
                page(vec![event(0, beyond)], false),
                &mut budget
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn the_largest_callback_event_fits_an_otherwise_empty_response() {
        let long_text = "\u{1}\"".repeat(MAX_STATUS_TEXT_BYTES);
        let mut runs: Vec<_> = (0..MAX_REALTIME_RUNS - 1)
            .map(|index| ExecutionRunRecord {
                current_step: Some(long_text.clone()),
                error_message: Some(long_text.clone()),
                status: RunStatus::Cancelled,
                progress: i32::MIN,
                ..named(&format!("{index:02}{}", "\"".repeat(254)))
            })
            .collect();
        runs.push(named("run-big"));
        let payload = json!("x".repeat(EVENT_CALLBACK_BODY_LIMIT));
        let store = Store::with(runs.clone(), vec![event_of("run-big", 0, payload.clone())]);
        let subscriptions: Vec<_> = runs
            .iter()
            .map(|run| RealtimeRunSubscription {
                after_sequence: if run.id == "run-big" {
                    -1
                } else {
                    i32::MAX - 1
                },
                ..subscribe(&run.id)
            })
            .collect();
        let mut readers = RunReaders::new("user-a", true, member());
        let mut response = RealtimePollResponse {
            notifications: Some(RealtimeNotificationUpdate {
                revision: "r".repeat(64),
                overview: RealtimeNotificationOverview {
                    invites_count: u64::MAX,
                    notifications_count: u64::MAX,
                    unread_count: u64::MAX,
                },
            }),
            ..Default::default()
        };
        replay_runs(Some(&store), &mut readers, &subscriptions, &mut response).await;
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let big = response
            .runs
            .iter()
            .find(|update| update.run_id == "run-big")
            .unwrap();
        assert_eq!(big.events.len(), 1);
        assert_eq!(big.events[0].payload, payload);
        assert!(!big.has_more);
        assert!(encoded_len(&response) <= MAX_REALTIME_RESPONSE_BYTES);
    }

    #[test]
    fn run_metadata_stays_inside_its_reservation() {
        let text = "\u{1}".repeat(MAX_STATUS_TEXT_BYTES) + "\"\\\n\u{e9}\u{1F600}";
        let update = changed(run_update(
            &ExecutionRunRecord {
                current_step: Some(text.clone()),
                error_message: Some(text),
                status: RunStatus::Cancelled,
                progress: i32::MIN,
                ..named(&"\"".repeat(256))
            },
            &RealtimeRunSubscription {
                after_sequence: i32::MIN,
                ..subscription()
            },
            EventReplay::default(),
            &mut budget(),
        ));
        assert!(encoded_len(&update) < MAX_RUN_METADATA_BYTES);
        for text in [update.current_step.unwrap(), update.error.unwrap()] {
            assert!(encoded_len(&text) <= MAX_STATUS_TEXT_BYTES);
        }
        assert_eq!(bounded_text("short \u{1F600}"), "short \u{1F600}");
        let emoji = "\u{1F600}".repeat(MAX_STATUS_TEXT_BYTES);
        assert!(encoded_len(&bounded_text(&emoji)) <= MAX_STATUS_TEXT_BYTES);
    }

    #[tokio::test]
    async fn a_full_response_defers_runs_and_keeps_room_for_errors() {
        let runs: Vec<_> = (0..MAX_REALTIME_RUNS)
            .map(|index| named(&format!("run-{index:02}")))
            .collect();
        let events = runs
            .iter()
            .map(|run| event_of(&run.id, 0, json!("x".repeat(300 * 1024))))
            .collect();
        let store = Store::with(runs.clone(), events);
        let subscriptions: Vec<_> = runs.iter().map(|run| subscribe(&run.id)).collect();
        let mut readers = RunReaders::new("user-a", true, member());
        let mut response = replay_all(Some(&store), &mut readers, &subscriptions).await;
        let delivered = response
            .runs
            .iter()
            .filter(|update| !update.events.is_empty())
            .count();
        assert!(delivered > 0 && delivered < MAX_REALTIME_RUNS);
        for deferred in response
            .runs
            .iter()
            .filter(|update| update.events.is_empty())
        {
            assert_eq!(deferred.after_sequence, -1);
            assert!(deferred.has_more);
        }
        assert_eq!(response.runs.len(), MAX_REALTIME_RUNS);
        assert!(response.errors.is_empty());
        for _ in 0..MAX_REALTIME_RUNS {
            response.errors.push(RealtimeSubscriptionError {
                run_id: "\"".repeat(256),
                code: REALTIME_ERROR_UNAVAILABLE.into(),
            });
        }
        assert!(encoded_len(&response) <= MAX_REALTIME_RESPONSE_BYTES);
    }

    #[tokio::test]
    async fn inbox_revisions_follow_every_notification_and_invitation_change() {
        use sea_orm::{ConnectOptions, ConnectionTrait, Database};

        let db = Database::connect(
            ConnectOptions::new("sqlite::memory:")
                .max_connections(1)
                .clone(),
        )
        .await
        .unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        for statement in [
            r#"CREATE TABLE public."Notification" (
                id TEXT PRIMARY KEY, "userId" TEXT NOT NULL, read BOOLEAN NOT NULL,
                "createdAt" TEXT NOT NULL, "readAt" TEXT
            )"#,
            r#"CREATE TABLE public."Invitation" (
                id TEXT PRIMARY KEY, "userId" TEXT NOT NULL, "createdAt" TEXT NOT NULL
            )"#,
            r#"CREATE TABLE public."WasmPackageInvitation" (
                id TEXT PRIMARY KEY, "inviteeId" TEXT NOT NULL, status TEXT NOT NULL,
                "createdAt" TEXT NOT NULL, "expiresAt" TEXT
            )"#,
            r#"CREATE TABLE public."InboxEntry" (
                id TEXT PRIMARY KEY, "inboxId" TEXT NOT NULL, "createdAt" TEXT NOT NULL
            )"#,
            r#"CREATE TABLE public."ConversationMember" (
                "conversationId" TEXT NOT NULL, "principalId" TEXT NOT NULL, "lastReadAt" TEXT
            )"#,
            r#"CREATE TABLE public."Bot" (
                id TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "deletedAt" TEXT
            )"#,
            r#"CREATE TABLE public."BotTurn" (
                id TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "updatedAt" TEXT NOT NULL
            )"#,
            r#"INSERT INTO public."BotTurn" VALUES
                ('turn-a','user-a','2026-01-01T00:00:00Z'),
                ('turn-b','user-b','2026-01-01T00:00:00Z')"#,
            r#"INSERT INTO public."InboxEntry" VALUES
                ('entry-a','user-a','2026-01-01T00:00:00Z'),
                ('entry-b','user-b','2026-12-01T00:00:00Z')"#,
            r#"INSERT INTO public."ConversationMember" VALUES
                ('conversation-1','user-a',NULL),
                ('conversation-1','user-b','2026-12-01T00:00:00Z')"#,
            r#"INSERT INTO public."Notification" VALUES
                ('notice-1','user-a',false,'2026-01-01T00:00:00Z',NULL),
                ('notice-2','user-a',true,'2026-01-02T00:00:00Z','2026-01-03T00:00:00Z'),
                ('foreign','user-b',false,'2026-02-01T00:00:00Z',NULL)"#,
            r#"INSERT INTO public."Invitation" VALUES
                ('invite-1','user-a','2026-01-01T00:00:00Z'),
                ('invite-2','user-b','2026-01-01T00:00:00Z')"#,
            r#"INSERT INTO public."WasmPackageInvitation" VALUES
                ('package-1','user-a','PENDING','2026-01-01T00:00:00Z',NULL),
                ('package-2','user-a','PENDING','2020-01-01T00:00:00Z','2020-02-01T00:00:00Z'),
                ('package-3','user-a','ACCEPTED','2026-01-01T00:00:00Z',NULL)"#,
        ] {
            db.execute_unprepared(statement).await.unwrap();
        }

        let first = notification_update(&db, "user-a").await.unwrap();
        assert_eq!(
            first.overview,
            RealtimeNotificationOverview {
                invites_count: 2,
                notifications_count: 2,
                unread_count: 1,
            }
        );
        assert_eq!(
            first.revision,
            notification_update(&db, "user-a").await.unwrap().revision
        );
        assert_eq!(
            notification_update(&db, "user-a' OR 1=1")
                .await
                .unwrap()
                .overview,
            RealtimeNotificationOverview::default()
        );

        let mut revisions = vec![first.revision];
        for mutation in [
            r#"UPDATE public."Notification" SET read=true, "readAt"='2026-01-04T00:00:00Z' WHERE id='notice-1'"#,
            r#"DELETE FROM public."Notification" WHERE id='notice-2'"#,
            r#"INSERT INTO public."Notification" VALUES ('notice-3','user-a',true,'2026-01-05T00:00:00Z','2026-01-05T00:00:00Z')"#,
            r#"INSERT INTO public."Invitation" VALUES ('invite-3','user-a','2026-01-07T00:00:00Z')"#,
            r#"UPDATE public."Invitation" SET id='invite-4', "createdAt"='2026-01-08T00:00:00Z' WHERE id='invite-1'"#,
            r#"UPDATE public."WasmPackageInvitation" SET "createdAt"='2026-01-09T00:00:00Z' WHERE id='package-1'"#,
            r#"UPDATE public."WasmPackageInvitation" SET status='ACCEPTED' WHERE id='package-1'"#,
            r#"INSERT INTO public."InboxEntry" VALUES ('entry-a2','user-a','2026-01-06T00:00:00Z')"#,
            r#"UPDATE public."ConversationMember" SET "lastReadAt"='2026-01-06T00:00:00Z' WHERE "principalId"='user-a'"#,
            r#"UPDATE public."BotTurn" SET "updatedAt"='2026-01-10T00:00:00Z' WHERE id='turn-a'"#,
            r#"INSERT INTO public."BotTurn" VALUES ('turn-a2','user-a','2026-01-11T00:00:00Z')"#,
        ] {
            let unrelated = r#"INSERT INTO public."Notification" VALUES (hex(randomblob(8)),'user-b',false,'2026-03-01T00:00:00Z',NULL)"#;
            db.execute_unprepared(unrelated).await.unwrap();
            db.execute_unprepared(
                r#"UPDATE public."BotTurn" SET "updatedAt"=datetime('now') WHERE id='turn-b'"#,
            )
            .await
            .unwrap();
            assert_eq!(
                &notification_update(&db, "user-a").await.unwrap().revision,
                revisions.last().unwrap(),
                "another account's inbox changed it"
            );
            db.execute_unprepared(mutation).await.unwrap();
            let revision = notification_update(&db, "user-a").await.unwrap().revision;
            assert!(!revisions.contains(&revision), "{mutation}");
            revisions.push(revision);
        }
    }
}
