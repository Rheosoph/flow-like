#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod claim;
mod file_tracking;
mod gcs_events;
mod media_transformation;
mod push_auth;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{
    HeaderMap, StatusCode, Uri,
    header::{AUTHORIZATION, HOST},
};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE as BASE64_URL_SAFE};
use claim::{ClaimDecision, ClaimError, ClaimGuard, ClaimState, ClaimStore, InFlightWait};
use file_tracking::{FileTrackingContext, FileTrackingError};
use flow_like_compiler::{CompilerConfig, CompilerError, compile};
use flow_like_executor::{
    ExecutionRequest, ExecutorConfig, ExecutorError, MAX_REMOTE_PAYLOAD_BYTES, ResolveError,
    fetch_bounded, report_queue_failure, resolve_payload_from_str,
};
use flow_like_gcp_data::firestore::FirestoreClient;
use flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env;
use flow_like_gcp_data::postgres::{self, IamPostgresConfig};
use flow_like_storage::object_store::gcp::GoogleCloudStorageBuilder;
use flow_like_types_contracts::dispatch::{CompilationJob, CompilationJobRef};
use gcs_events::{GcsEvent, GcsEventError};
use media_transformation::{MediaTransformationContext, MediaTransformationError};
use push_auth::{PushAuthError, PushTokenVerifier};
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::future::IntoFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Pub/Sub refuses to publish a message whose data exceeds 10 MiB, so this is
/// the exact largest body that can ever have been sent. Anything the API needs
/// to hand over above that arrives as a claim-check reference instead.
const MAX_MESSAGE_DATA_BYTES: usize = 10 * 1024 * 1024;
/// The push envelope carries that data base64-encoded (a 4:3 expansion) inside a
/// JSON wrapper. The ceiling is enforced by the router so a hostile push cannot
/// make the process allocate before a single byte has been parsed.
const MAX_PUSH_BODY_BYTES: usize = 14 * 1024 * 1024;
const ENVELOPE_VERSION: u32 = 1;

/// Pub/Sub refuses a `dead_letter_policy` with fewer than five delivery
/// attempts. A message delivered more than five times has therefore either
/// already been copied to the dead-letter topic, or belongs to a subscription
/// with no policy to copy it — and in both cases holding on to it buys nothing,
/// so this is the point at which the worker acks a message it has given up on.
/// The worker's own ceiling (`GCP_QUEUE_MAX_DELIVERY_ATTEMPTS`, default 3 to
/// match the AWS `maxReceiveCount` and the Azure dequeue cap) governs when it
/// stops doing *work*; this constant governs when it stops asking the broker to
/// try again.
const PUBSUB_DEAD_LETTER_MIN_ATTEMPTS: i64 = 5;

/// Seconds reserved between the worker's own processing deadline and the Cloud
/// Run request timeout. The deadline has to fire first: a request Cloud Run
/// kills leaves the claim `in_progress` with nobody to release it, and every
/// redelivery then parks on the claim until it ages out instead of doing work.
const SETTLE_MARGIN_SECS: u64 = 30;

/// Endpoint overrides for the three GCP services this worker touches. Ignoring
/// one would leave an operator believing an override took effect; honouring one
/// would point the worker's credentials at whoever set it. `flow_like_gcp_data`
/// enforces the credential half of this list.
const FORBIDDEN_ENDPOINT_SETTINGS: &[&str] = &[
    "PUBSUB_EMULATOR_HOST",
    "FIRESTORE_EMULATOR_HOST",
    "DATASTORE_EMULATOR_HOST",
    "STORAGE_EMULATOR_HOST",
    "CLOUDSDK_API_ENDPOINT_OVERRIDES_PUBSUB",
    "CLOUDSDK_API_ENDPOINT_OVERRIDES_STORAGE",
    "GOOGLE_STORAGE_CUSTOM_ENDPOINT",
    // `object_store`'s GCS builder reads all four of these as key-file sources.
    "GOOGLE_SERVICE_ACCOUNT",
    "GOOGLE_SERVICE_ACCOUNT_PATH",
    "GOOGLE_SERVICE_ACCOUNT_KEY",
    "SERVICE_ACCOUNT",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info")
                .add_directive("hyper=warn".parse().expect("valid filter"))
                .add_directive("rustls=warn".parse().expect("valid filter"))
                .add_directive("tokio=warn".parse().expect("valid filter"))
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    if config.workload == Workload::Execution {
        flow_like_catalog::initialize();
    }

    // Every workload needs Firestore, because every workload needs the claim.
    let firestore = FirestoreClient::from_env()?;
    let claims = ClaimStore::new(
        firestore.clone(),
        config.claims_collection.clone(),
        config.workload.to_string(),
        config.claim_stale_after,
        config.claim_retention,
    )?;

    let verifier = PushTokenVerifier::new(config.push_service_account.clone())?;
    // Failing here rather than on the first delivery turns "this project cannot
    // reach Google's key set" into a boot failure with a revision that never
    // takes traffic, instead of a service that nacks everything it is sent.
    verifier.warm().await?;

    tracing::info!(
        workload = %config.workload,
        project = %config.project_id,
        subscription = %config.subscription,
        push_audience = %config.push_audience,
        push_service_account = %config.push_service_account,
        claims_collection = %config.claims_collection,
        claim_owner = %claims.owner(),
        max_delivery_attempts = config.max_delivery_attempts,
        process_timeout_seconds = config.process_timeout.as_secs(),
        request_timeout_seconds = config.request_timeout.as_secs(),
        ack_deadline_seconds = config.ack_deadline.as_secs(),
        "starting keyless GCP Pub/Sub push worker"
    );
    if config.process_timeout > config.ack_deadline {
        // Not an error: this is the ack-deadline gap the claim exists for. It
        // is logged because it has a cost the operator should have chosen: every
        // redelivery of a run that outlives the ack deadline is parked and then
        // nacked, and each nack counts toward the subscription's
        // max_delivery_attempts, so a legitimately long run can be copied to the
        // dead-letter topic by the broker while its owner is still working.
        tracing::warn!(
            process_timeout_seconds = config.process_timeout.as_secs(),
            ack_deadline_seconds = config.ack_deadline.as_secs(),
            "process deadline exceeds the Pub/Sub ack deadline; long runs will be redelivered \
             while in flight and each parked redelivery counts against max_delivery_attempts"
        );
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_shutdown = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        let _ = signal_shutdown.send(true);
    });

    let (workload, database_lifecycle) = WorkloadContext::build(&config, &firestore).await?;
    let state = Arc::new(WorkerState {
        config,
        workload,
        claims,
        verifier,
        accepting: AtomicBool::new(true),
        shutdown: shutdown_rx.clone(),
    });

    // The probe contract shared with the gcp-api and gcp-executor images and
    // pinned by modules/gcp/runtime: liveness is `/health/live`, readiness is
    // `/health/ready` (and `/health`, which the Azure-era tooling probes). A
    // liveness route that is missing is not a probe that is skipped — Cloud Run
    // reads the 404 as a failure and restarts the container every
    // failure_threshold × period, which for a worker means no delivery longer
    // than that ever completes.
    let app = Router::new()
        .route("/", post(handle_push))
        .route("/health", get(handle_ready))
        .route("/health/ready", get(handle_ready))
        .route("/health/live", get(handle_live))
        .layer(DefaultBodyLimit::max(MAX_PUSH_BODY_BYTES))
        .with_state(state.clone());

    let address = format!("0.0.0.0:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(address = %address, "Pub/Sub push endpoint is listening");

    // The PostgreSQL pool holds one Cloud SQL IAM token that cannot be refreshed
    // in place (see flow_like_gcp_data::postgres). The worker therefore closes
    // its health endpoint and stops accepting deliveries once the token enters
    // its safety window, and exits before expiry; Cloud Run starts a fresh
    // instance with a fresh token. Refused deliveries are nacks, so nothing is
    // lost — Pub/Sub simply redelivers them to the replacement.
    let hard_stop = async {
        match database_lifecycle {
            Some(lifecycle) => {
                let drain_state = state.clone();
                let drain_lifecycle = lifecycle.clone();
                tokio::spawn(async move {
                    drain_lifecycle.wait_until_drain().await;
                    drain_state.accepting.store(false, Ordering::SeqCst);
                    tracing::warn!(
                        token_lifetime_seconds = drain_lifecycle.remaining_seconds(),
                        drain_seconds = postgres::READINESS_DRAIN_SECONDS,
                        "database access token entered its safety window; refusing new deliveries"
                    );
                    tokio::time::sleep(Duration::from_secs(postgres::READINESS_DRAIN_SECONDS))
                        .await;
                    let _ = shutdown_tx.send(true);
                });
                lifecycle.wait_until_hard_stop().await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(hard_stop);

    let shutdown_state = state.clone();
    let mut graceful = shutdown_rx.clone();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*graceful.borrow() {
                if graceful.changed().await.is_err() {
                    break;
                }
            }
            shutdown_state.accepting.store(false, Ordering::SeqCst);
        })
        .into_future();
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => result?,
        _ = &mut hard_stop => {
            tracing::error!(
                graceful_shutdown_seconds = postgres::GRACEFUL_SHUTDOWN_SECONDS,
                "forcing worker rotation before the database access token expires"
            );
        }
    }
    Ok(())
}

struct WorkerState {
    config: Config,
    workload: WorkloadContext,
    claims: ClaimStore,
    verifier: PushTokenVerifier,
    accepting: AtomicBool,
    shutdown: watch::Receiver<bool>,
}

#[derive(serde::Serialize)]
struct HealthResponse {
    status: &'static str,
    workload: String,
    version: &'static str,
}

/// Liveness for the Cloud Run probe: is the process alive and serving.
///
/// It stays `200` through the database-token drain deliberately. The drain is
/// the process doing its job — it stops accepting deliveries, waits out
/// `READINESS_DRAIN_SECONDS`, and exits on its own so Cloud Run starts an
/// instance with a fresh Cloud SQL IAM token. Failing liveness during that
/// window would buy nothing (the exit is already scheduled) and would risk
/// restarting an instance that still has a delivery in flight, which is the
/// same restart-a-healthy-container mistake modules/gcp/runtime warns against
/// for the API.
async fn handle_live(State(state): State<Arc<WorkerState>>) -> Response {
    health_response(StatusCode::OK, "healthy", &state)
}

/// Readiness: whether this instance will accept a delivery right now. Reported
/// on `/health/ready` and, for the tooling that grew up on the Azure worker, on
/// `/health`. `503` during the drain is what tells an operator's check why
/// deliveries are being nacked with `worker_draining`.
async fn handle_ready(State(state): State<Arc<WorkerState>>) -> Response {
    if state.accepting.load(Ordering::SeqCst) {
        health_response(StatusCode::OK, "ready", &state)
    } else {
        health_response(StatusCode::SERVICE_UNAVAILABLE, "draining", &state)
    }
}

fn health_response(status_code: StatusCode, status: &'static str, state: &WorkerState) -> Response {
    (
        status_code,
        Json(HealthResponse {
            status,
            workload: state.config.workload.to_string(),
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
        .into_response()
}

async fn handle_push(
    State(state): State<Arc<WorkerState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Authentication runs before anything looks at the body. An unauthenticated
    // caller must not be able to reach the JSON parser, let alone the executor.
    let expected_audience = match state.config.push_audience.resolve(&headers, &uri) {
        Ok(audience) => audience,
        Err(detail) => {
            tracing::warn!(
                detail,
                "rejected a Pub/Sub push delivery whose expected audience could not be derived"
            );
            return (StatusCode::UNAUTHORIZED, "push_audience_unresolvable").into_response();
        }
    };
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if let Err(error) = state
        .verifier
        .verify(authorization, &expected_audience)
        .await
    {
        return match error {
            PushAuthError::KeySetUnavailable(detail) => {
                tracing::error!(detail = %detail, "Google's OIDC key set is unreachable");
                Settlement::Retry("push_key_set_unavailable").into_response()
            }
            error => {
                tracing::warn!(
                    error = %error,
                    expected_audience = %expected_audience,
                    expected_service_account = %state.config.push_service_account,
                    "rejected an unauthenticated Pub/Sub push delivery"
                );
                (StatusCode::UNAUTHORIZED, "push_token_rejected").into_response()
            }
        };
    }

    if !state.accepting.load(Ordering::SeqCst) {
        return Settlement::Retry("worker_draining").into_response();
    }

    let delivery = match serde_json::from_slice::<PushDelivery>(&body) {
        Ok(delivery) => delivery,
        Err(error) => {
            // A nack. The body can never become valid, so this ends in the
            // dead-letter topic — which is exactly where an unparseable delivery
            // belongs, and the only place its bytes are preserved for inspection.
            tracing::error!(error = %error, "push delivery was not a Pub/Sub envelope");
            return Settlement::DeadLetter("invalid_push_envelope").into_response();
        }
    };

    // The subscription name is pinned as well as the pusher identity. The
    // identity says "Pub/Sub sent this"; the subscription says "and it came from
    // the topic this worker was built to drain", which is what stops a
    // repointed subscription from feeding compilation jobs to the file tracker.
    if delivery.subscription != state.config.subscription {
        tracing::error!(
            received = %delivery.subscription,
            expected = %state.config.subscription,
            "push delivery arrived from a subscription this worker does not serve"
        );
        return (StatusCode::FORBIDDEN, "unexpected_subscription").into_response();
    }

    let settlement = process_delivery(&state, delivery).await;
    tracing::debug!(
        outcome = settlement.code(),
        "settled a Pub/Sub push delivery"
    );
    settlement.into_response()
}

async fn process_delivery(state: &WorkerState, delivery: PushDelivery) -> Settlement {
    let config = &state.config;
    let message_id = delivery.message.message_id();
    if message_id.is_empty() {
        tracing::error!("push delivery carried no messageId; it cannot be de-duplicated");
        return Settlement::DeadLetter("missing_message_id");
    }

    // ---------------------------------------------------------------------
    // The claim. Everything below this point assumes exactly one live worker
    // per messageId, and this is what makes that true.
    // ---------------------------------------------------------------------
    // One budget across every park in this delivery, not one per park: a chain
    // of takeovers by other replicas would otherwise keep this request alive
    // past the deadline Cloud Run gave it.
    let park_started = tokio::time::Instant::now();
    let mut guard = loop {
        match state.claims.acquire(&config.subscription, message_id).await {
            Ok(ClaimDecision::Owned(guard)) => break guard,
            Ok(ClaimDecision::InFlight {
                owner,
                attempts,
                age_seconds,
            }) => {
                let remaining = config
                    .in_flight_park_budget
                    .saturating_sub(park_started.elapsed());
                if remaining.is_zero() {
                    return Settlement::Retry("owner_in_flight");
                }
                // The ack-deadline gap in the flesh: Pub/Sub redelivered a
                // message another delivery is still working on. This is NOT
                // acked. Acking any live ackId removes the message from the
                // subscription, and if the owner then dies the message is gone
                // — no redelivery, no DLQ copy, a run left `Running` until its
                // TTL. On AWS/Azure the visibility timeout guarantees the
                // message comes back after owner death; here the claim has to.
                // So the redelivery parks on the claim instead: it re-enters
                // `acquire` the moment the owner settles (→ duplicate handling
                // below) or stops heartbeating (→ this delivery takes over and
                // does the work), and nacks if the owner is still alive when
                // the budget runs out, so the broker asks again later. The
                // budget is the staleness window, which fits inside this
                // request's own deadline by construction, and a dead owner is
                // detected within it by definition.
                tracing::info!(
                    message_id,
                    owner = %owner,
                    attempts,
                    age_seconds,
                    park_seconds = remaining.as_secs(),
                    "parking a redelivery of a message another worker still holds"
                );
                let mut shutdown = state.shutdown.clone();
                let waited = tokio::select! {
                    waited = state.claims.wait_while_in_flight(
                        &config.subscription,
                        message_id,
                        remaining,
                        config.in_flight_poll,
                    ) => waited,
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Settlement::Retry("worker_shutdown");
                        }
                        continue;
                    }
                };
                match waited {
                    Ok(InFlightWait::Changed) => continue,
                    Ok(InFlightWait::StillInFlight) => {
                        tracing::info!(
                            message_id,
                            owner = %owner,
                            "owner still in flight after the park budget; nacking for a later look"
                        );
                        return Settlement::Retry("owner_in_flight");
                    }
                    Err(error) => {
                        tracing::error!(message_id, error = %error, "claim could not be observed");
                        return Settlement::Retry("claim_store_unavailable");
                    }
                }
            }
            Ok(ClaimDecision::Settled {
                state: ClaimState::DeadLettered,
                attempts,
                outcome_code,
            }) => {
                tracing::warn!(
                    message_id,
                    attempts,
                    failure_code = %outcome_code,
                    "redelivery of a message already marked dead"
                );
                return if give_up_and_ack(attempts, config.max_delivery_attempts) {
                    Settlement::Ack("dead_letter_window_exhausted")
                } else {
                    Settlement::DeadLetter("already_dead_lettered")
                };
            }
            Ok(ClaimDecision::Settled {
                state: claim_state, ..
            }) => {
                tracing::info!(
                    message_id,
                    claim_state = %claim_state,
                    "shedding a redelivery of a finished message"
                );
                return Settlement::Ack("duplicate_settled");
            }
            Err(error) => {
                // Fail closed. The claim is the only thing between a redelivery
                // and a duplicate run, so a claim store that cannot be reached
                // is a hard stop rather than a warning to work through.
                tracing::error!(message_id, error = %error, "claim could not be acquired");
                return Settlement::Retry("claim_store_unavailable");
            }
        }
    };

    // The broker's count is authoritative when it exists, because it is the same
    // counter the subscription's `dead_letter_policy` uses, so the worker's
    // give-up point and the broker's routing point coincide. Pub/Sub omits it
    // entirely when no policy is configured, and then the claim's own count —
    // the direct analogue of Azure's `DequeueCount` — stands in. Deliveries shed
    // as duplicates or parked on a live owner never reach this line, so a job
    // that legitimately outlives the ack deadline is not counted out *here* for
    // being slow — though the broker's own counter does advance on every parked
    // redelivery it nacks, which is the trade-off `main` warns about at boot.
    let attempts = delivery
        .delivery_attempt
        .map(i64::from)
        .unwrap_or_else(|| guard.attempts());

    if attempts > config.max_delivery_attempts {
        return dead_letter(
            state,
            &delivery.message,
            guard,
            attempts,
            "max_delivery_attempts_exceeded",
        )
        .await;
    }

    let prepared = match prepare(&delivery.message, config.workload) {
        Ok(prepared) => prepared,
        Err(code) => return dead_letter(state, &delivery.message, guard, attempts, code).await,
    };

    let outcome = {
        let processing = process_message(&state.workload, config.workload, prepared);
        tokio::pin!(processing);

        let deadline = tokio::time::sleep(config.process_timeout);
        tokio::pin!(deadline);

        // The claim heartbeat is this port's visibility renewal. It runs inside
        // the same `select!` as the work rather than in a spawned task, for the
        // reason the Azure worker gives about pop receipts: a concurrency token
        // owned by two tasks is exactly the race that silently redelivers
        // finished jobs.
        let mut heartbeat = tokio::time::interval(config.claim_heartbeat);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat.tick().await;

        let mut shutdown = state.shutdown.clone();
        loop {
            tokio::select! {
                outcome = &mut processing => break outcome,
                _ = heartbeat.tick() => {
                    match state.claims.heartbeat(&mut guard).await {
                        Ok(()) => {}
                        Err(ClaimError::Lost) => {
                            // Another replica decided this owner was dead and
                            // took the message. Settling now would overwrite its
                            // claim, so the work is dropped instead.
                            tracing::warn!(message_id, attempts, "claim_lost");
                            break ProcessOutcome::ClaimLost;
                        }
                        Err(error) => tracing::warn!(
                            message_id,
                            error = %error,
                            "claim heartbeat failed; work continues"
                        ),
                    }
                }
                _ = &mut deadline => break ProcessOutcome::Retryable("processing_timeout"),
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break ProcessOutcome::Interrupted;
                    }
                }
            }
        }
    };

    match outcome {
        ProcessOutcome::Success => {
            state.claims.complete(guard).await;
            tracing::info!(message_id, attempts, "queue workload completed");
            Settlement::Ack("completed")
        }
        ProcessOutcome::Permanent(code) => {
            dead_letter(state, &delivery.message, guard, attempts, code).await
        }
        ProcessOutcome::Retryable(code) => {
            state.claims.release(guard, code).await;
            tracing::warn!(
                message_id,
                attempts,
                failure_code = code,
                "queue workload failed and was released for redelivery"
            );
            Settlement::Retry(code)
        }
        ProcessOutcome::Interrupted => {
            state.claims.release(guard, "worker_shutdown").await;
            Settlement::Retry("worker_shutdown")
        }
        // Deliberately no settle: the claim belongs to somebody else now.
        ProcessOutcome::ClaimLost => Settlement::Retry("claim_lost"),
    }
}

/// Terminal handling for a message that will never succeed.
///
/// The run record is closed before anything else. Dead-lettering alone would
/// leave the execution `Running` until its TTL expires, and pollers would watch
/// a run that can never finish. The claim is then marked so that every later
/// redelivery of the same `messageId` skips both the work and this write.
async fn dead_letter(
    state: &WorkerState,
    message: &PubsubMessage,
    guard: ClaimGuard,
    attempts: i64,
    reason: &'static str,
) -> Settlement {
    fail_dead_lettered_execution_run(message, reason, state.config.workload).await;
    state.claims.dead_letter(guard, reason).await;

    tracing::error!(
        message_id = message.message_id(),
        subscription = %state.config.subscription,
        attempts,
        reason,
        "pubsub_message_dead_lettered"
    );

    // Nacking is what lets the subscription's `dead_letter_policy` carry the
    // message — and its attributes — into the dead-letter topic, where the GCS
    // sink preserves it. Acking here would destroy the only forensic copy. Once
    // the broker has had its five attempts the message is either already in the
    // dead-letter topic or has no policy to take it, and the worker acks so a
    // known-dead message stops circulating.
    if give_up_and_ack(attempts, state.config.max_delivery_attempts) {
        Settlement::Ack("dead_letter_window_exhausted")
    } else {
        Settlement::DeadLetter(reason)
    }
}

fn give_up_and_ack(attempts: i64, ceiling: i64) -> bool {
    attempts > ceiling.max(PUBSUB_DEAD_LETTER_MIN_ATTEMPTS)
}

/// Best-effort terminal `Failed` for the run carried by an execution message
/// that is about to be dead-lettered.
///
/// The identity material (executor JWT, signed broker job ID) is re-derived from
/// the message body, so this covers every dead-letter path whose payload is
/// still readable; unreadable or unfetchable payloads are skipped. A failure
/// here must never block the dead-letter decision.
async fn fail_dead_lettered_execution_run(
    message: &PubsubMessage,
    reason: &str,
    workload: Workload,
) {
    if workload != Workload::Execution {
        return;
    }
    let Ok(PreparedMessage::Dispatch { payload, .. }) = prepare(message, Workload::Execution)
    else {
        return;
    };
    let payload = match resolve_payload_from_str(&payload).await {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "cannot resolve dead-lettered execution payload; run record stays non-terminal"
            );
            return;
        }
    };
    if let Err(error) = report_queue_failure(
        &payload.executor_jwt,
        &payload.job_id,
        &format!("queue delivery dead-lettered: {reason}"),
        &ExecutorConfig::from_env(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            job_id = %payload.job_id,
            "failed to persist terminal Failed status for a dead-lettered execution"
        );
    }
}

#[derive(Debug)]
enum PreparedMessage {
    /// The API's dispatch envelope, unwrapped.
    Dispatch { job_id: String, payload: String },
    /// A Cloud Storage notification. Unlike the Azure port this never round-trips
    /// through a string, so the `event.id != job_id` check that guarded that
    /// round trip has nothing left to compare and is gone; the bucket check
    /// inside each handler is the guard that mattered.
    ObjectEvent(Box<GcsEvent>),
}

/// Decode the message and unwrap whatever contract its producer speaks.
fn prepare(message: &PubsubMessage, workload: Workload) -> Result<PreparedMessage, &'static str> {
    let data = message.decoded_data().ok_or("invalid_message_encoding")?;
    if data.len() > MAX_MESSAGE_DATA_BYTES {
        return Err("message_too_large");
    }

    // Object-event workloads are fed by `google_storage_notification`, which puts
    // the routing facts in the message attributes and the object resource in the
    // body; there is no dispatch envelope to unwrap.
    if workload.is_object_event_workload() {
        let event = GcsEvent::parse(&message.attributes, &data).map_err(|error| match error {
            GcsEventError::Malformed => workload.invalid_payload_code(),
            GcsEventError::UnsupportedType => "unsupported_object_event_type",
        })?;
        return Ok(PreparedMessage::ObjectEvent(Box::new(event)));
    }

    let envelope = serde_json::from_slice::<DispatchEnvelope>(&data)
        .map_err(|_| workload.invalid_payload_code())?;
    if envelope.v != ENVELOPE_VERSION {
        return Err("envelope_version_unsupported");
    }
    let payload =
        serde_json::to_string(&envelope.payload).map_err(|_| workload.invalid_payload_code())?;

    Ok(PreparedMessage::Dispatch {
        job_id: envelope.job_id,
        payload,
    })
}

async fn process_message(
    context: &WorkloadContext,
    workload: Workload,
    prepared: PreparedMessage,
) -> ProcessOutcome {
    match prepared {
        PreparedMessage::ObjectEvent(event) => match context {
            WorkloadContext::FileTracking(ctx) => match file_tracking::process(ctx, &event).await {
                Ok(()) => ProcessOutcome::Success,
                Err(FileTrackingError::Permanent(code)) => ProcessOutcome::Permanent(code),
                Err(FileTrackingError::Retryable(code)) => ProcessOutcome::Retryable(code),
            },
            WorkloadContext::MediaTransformation(ctx) => {
                match media_transformation::process(ctx, &event).await {
                    Ok(()) => ProcessOutcome::Success,
                    Err(MediaTransformationError::Permanent(code)) => {
                        ProcessOutcome::Permanent(code)
                    }
                    Err(MediaTransformationError::Retryable(code)) => {
                        ProcessOutcome::Retryable(code)
                    }
                }
            }
            WorkloadContext::Dispatch => ProcessOutcome::Permanent("workload_context_mismatch"),
        },
        PreparedMessage::Dispatch { job_id, payload } => match workload {
            Workload::Execution => {
                // The payload is either inline or a claim-check reference to a
                // staged object, so the job ID is only known after resolution.
                let payload = match resolve_payload_from_str(&payload).await {
                    Ok(payload) => payload,
                    Err(ResolveError::Fetch(_)) => {
                        return ProcessOutcome::Retryable("execution_payload_fetch_failed");
                    }
                    Err(ResolveError::Deserialize(_)) => {
                        return ProcessOutcome::Permanent("invalid_execution_payload");
                    }
                };
                // Pub/Sub mints its own `messageId`, so the identity the producer
                // signed travels in the envelope instead of in broker metadata.
                // The check itself is unchanged.
                if payload.job_id != job_id {
                    return ProcessOutcome::Permanent("execution_message_id_mismatch");
                }
                let request = match ExecutionRequest::try_from(payload) {
                    Ok(request) => request,
                    Err(_) => return ProcessOutcome::Permanent("invalid_execution_request"),
                };

                let executor_config =
                    ExecutorConfig::from_env().with_required_terminal_status_ack();
                match flow_like_executor::execute(request, executor_config).await {
                    // Strict queue mode only returns success after the API has
                    // durably acknowledged a terminal run status. User-code
                    // failure is therefore a completed delivery, while callback
                    // failure nacks the delivery for a retry.
                    Ok(_) => ProcessOutcome::Success,
                    Err(ExecutorError::InvalidRequest(_)) => {
                        ProcessOutcome::Permanent("execution_claims_mismatch")
                    }
                    Err(_) => ProcessOutcome::Retryable("execution_failed"),
                }
            }
            Workload::Compilation => {
                let job = match resolve_compilation_job(&payload).await {
                    Ok(job) => job,
                    Err(outcome) => return outcome,
                };
                if job.job_id != job_id {
                    return ProcessOutcome::Permanent("compilation_message_id_mismatch");
                }

                match compile(job, &CompilerConfig::from_env()).await {
                    Ok(_) => ProcessOutcome::Success,
                    Err(CompilerError::InvalidJob(_) | CompilerError::Jwt(_)) => {
                        ProcessOutcome::Permanent("compilation_claims_mismatch")
                    }
                    Err(_) => ProcessOutcome::Retryable("compilation_failed"),
                }
            }
            Workload::FileTracking | Workload::MediaTransformation => {
                ProcessOutcome::Permanent("workload_context_mismatch")
            }
        },
    }
}

/// Resolve a message body into a concrete [`CompilationJob`].
///
/// Mirrors `flow_like_executor::resolve_payload_from_str` for compilation, which
/// has no shared resolver. A failed fetch of the staged object is a transient
/// condition and stays retryable; anything that cannot be parsed is permanently
/// invalid.
async fn resolve_compilation_job(json: &str) -> Result<CompilationJob, ProcessOutcome> {
    let job_ref = serde_json::from_str::<CompilationJobRef>(json)
        .map_err(|_| ProcessOutcome::Permanent("invalid_compilation_payload"))?;

    let remote_url = match job_ref {
        CompilationJobRef::Inline(job) => return Ok(job),
        CompilationJobRef::Remote { remote_url } => remote_url,
    };

    tracing::info!("fetching staged compilation job");
    let body = fetch_bounded(&remote_url, MAX_REMOTE_PAYLOAD_BYTES)
        .await
        .map_err(|_| ProcessOutcome::Retryable("compilation_payload_fetch_failed"))?;

    serde_json::from_slice::<CompilationJob>(&body)
        .map_err(|_| ProcessOutcome::Permanent("invalid_compilation_payload"))
}

/// Versioned wrapper around the dispatch payload.
///
/// `job_id` replaces the broker message ID as the signed identity the worker
/// compares against the resolved payload.
#[derive(Debug, Deserialize)]
struct DispatchEnvelope {
    v: u32,
    job_id: String,
    payload: serde_json::Value,
}

/// A Pub/Sub push delivery.
///
/// Pub/Sub emits the camelCase spellings; the snake_case aliases exist because
/// the proto3 JSON mapping permits both and some emulator and gateway paths use
/// the original field names.
#[derive(Debug, Deserialize)]
struct PushDelivery {
    message: PubsubMessage,
    #[serde(default)]
    subscription: String,
    /// Present **only** when the subscription carries a `dead_letter_policy`.
    #[serde(rename = "deliveryAttempt", alias = "delivery_attempt", default)]
    delivery_attempt: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct PubsubMessage {
    #[serde(default)]
    data: String,
    #[serde(default)]
    attributes: BTreeMap<String, String>,
    #[serde(rename = "messageId", alias = "message_id", default)]
    message_id: String,
    #[serde(rename = "publishTime", alias = "publish_time", default)]
    #[allow(dead_code)]
    publish_time: String,
}

impl PubsubMessage {
    fn message_id(&self) -> &str {
        self.message_id.trim()
    }

    /// Pub/Sub encodes `data` as base64. The standard alphabet is what the
    /// service emits; the URL-safe alphabet is accepted as a fallback because the
    /// proto3 JSON mapping permits it and a message that cannot be decoded is
    /// indistinguishable from a message that was never readable.
    fn decoded_data(&self) -> Option<Vec<u8>> {
        if self.data.is_empty() {
            return Some(Vec::new());
        }
        BASE64_STANDARD
            .decode(&self.data)
            .or_else(|_| BASE64_URL_SAFE.decode(&self.data))
            .ok()
    }
}

/// Pub/Sub reads exactly one thing from this worker: the HTTP status. A 2xx acks
/// and removes the message; anything else nacks and schedules a redelivery. Every
/// value here is therefore a durability decision rather than a diagnostic.
#[derive(Debug)]
enum Settlement {
    Ack(&'static str),
    /// Nack. The message is dead and should be routed to the dead-letter topic.
    DeadLetter(&'static str),
    /// Nack. Try again.
    Retry(&'static str),
}

impl Settlement {
    fn code(&self) -> &'static str {
        match self {
            Self::Ack(code) | Self::DeadLetter(code) | Self::Retry(code) => code,
        }
    }
}

impl IntoResponse for Settlement {
    fn into_response(self) -> Response {
        match self {
            // 204 is the cheapest ack and carries no body a future proxy could
            // try to buffer.
            Self::Ack(_) => StatusCode::NO_CONTENT.into_response(),
            // Both of the following are nacks and Pub/Sub cannot tell them
            // apart. They differ in the response-code class Cloud Run reports,
            // which is what separates "this message is dead" from "come back
            // later" in the dashboards and alert policies.
            Self::DeadLetter(code) => (StatusCode::INTERNAL_SERVER_ERROR, code).into_response(),
            Self::Retry(code) => (StatusCode::SERVICE_UNAVAILABLE, code).into_response(),
        }
    }
}

#[derive(Debug)]
enum ProcessOutcome {
    Success,
    Permanent(&'static str),
    Retryable(&'static str),
    Interrupted,
    /// The claim was taken over mid-flight; this delivery must not settle.
    ClaimLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    Execution,
    Compilation,
    FileTracking,
    MediaTransformation,
}

impl Workload {
    fn invalid_payload_code(self) -> &'static str {
        match self {
            Self::Execution => "invalid_execution_payload",
            Self::Compilation => "invalid_compilation_payload",
            Self::FileTracking | Self::MediaTransformation => "invalid_object_event",
        }
    }

    /// Fed by Cloud Storage notifications instead of the API's dispatch
    /// envelope; see `prepare`.
    fn is_object_event_workload(self) -> bool {
        matches!(self, Self::FileTracking | Self::MediaTransformation)
    }
}

impl Display for Workload {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Execution => "execution",
            Self::Compilation => "compilation",
            Self::FileTracking => "file-tracking",
            Self::MediaTransformation => "media-transformation",
        })
    }
}

/// Clients a workload owns for its lifetime. Execution and compilation hold
/// none: they reach the API over HTTPS with the JWT inside the signed payload.
/// The object-event workloads are the ones that need data-plane clients of their
/// own, always through the instance's own service account.
enum WorkloadContext {
    Dispatch,
    FileTracking(FileTrackingContext),
    MediaTransformation(MediaTransformationContext),
}

impl WorkloadContext {
    async fn build(
        config: &Config,
        firestore: &FirestoreClient,
    ) -> Result<(Self, Option<postgres::DatabaseTokenLifecycle>), WorkerError> {
        match config.workload {
            Workload::Execution | Workload::Compilation => Ok((Self::Dispatch, None)),
            Workload::FileTracking => {
                let content_bucket = config.require_content_bucket()?;
                let postgres_config = IamPostgresConfig::from_env()
                    .map_err(|error| WorkerError::new("postgres_config", error))?;
                let managed = postgres::connect_as(&postgres_config, "flow-like-gcp-file-tracker")
                    .await
                    .map_err(|error| WorkerError::new("postgres_connect", error))?;
                tracing::info!(
                    token_lifetime_seconds = managed.lifecycle.remaining_seconds(),
                    files_collection = %config.files_collection,
                    "file tracker connected to Firestore and Cloud SQL with its workload identity"
                );
                Ok((
                    Self::FileTracking(FileTrackingContext {
                        firestore: firestore.clone(),
                        files_collection: config.files_collection.clone(),
                        content_bucket,
                        database: managed.connection,
                    }),
                    Some(managed.lifecycle),
                ))
            }
            Workload::MediaTransformation => {
                let content_bucket = config.require_content_bucket()?;
                // `new()` rather than `from_env()`: `from_env` reads the
                // service-account key variables, and those are rejected at
                // startup precisely so that no key path exists. With none of them
                // set, `build()` falls through to the instance credential
                // provider — the metadata server — which is the only credential
                // this worker is allowed to hold.
                let store = GoogleCloudStorageBuilder::new()
                    .with_bucket_name(&content_bucket)
                    .build()
                    .map_err(|error| WorkerError::new("object_store_init", error))?;
                Ok((
                    Self::MediaTransformation(MediaTransformationContext {
                        store: Arc::new(store),
                        content_bucket,
                    }),
                    None,
                ))
            }
        }
    }
}

/// Where the expected `aud` of a push token comes from.
///
/// The value Pub/Sub mints into `aud` is the subscription's
/// `oidc_token.audience`, which the root sets to the worker's own Cloud Run URL
/// — the same URL it uses as the push endpoint. A service cannot be handed its
/// own URL as an environment variable without the root referencing the
/// service's own output (a cycle), so the root deliberately does not set
/// `PUBSUB_PUSH_AUDIENCE`, and the worker derives the audience from the request
/// it is serving: `https://` + the `Host` Pub/Sub connected to. That is exactly
/// the URL the token was minted for whenever the request is a genuine push, and
/// it is *not* a weakening: the token still has to be Google-signed for the
/// pinned pusher service account, and Cloud Run's own invoker check has already
/// verified `aud` against the service's URL before the container saw the
/// request, so a token minted for another service never arrives here. The
/// invariant both sides rely on: **the subscription's `oidc_token.audience` must
/// equal its `push_endpoint`, with no path, query, fragment or trailing slash.**
/// A pinned value remains available for the case where they legitimately
/// differ (a custom audience), and then it must be the subscription's audience
/// verbatim.
#[derive(Clone, Debug)]
enum PushAudience {
    Pinned(String),
    RequestHost,
}

impl PushAudience {
    fn resolve<'a>(&'a self, headers: &HeaderMap, uri: &Uri) -> Result<Cow<'a, str>, &'static str> {
        match self {
            Self::Pinned(audience) => Ok(Cow::Borrowed(audience.as_str())),
            Self::RequestHost => {
                // `Host` on HTTP/1.1; `:authority` lands in the URI on HTTP/2.
                let host = headers
                    .get(HOST)
                    .and_then(|value| value.to_str().ok())
                    .or_else(|| uri.authority().map(|authority| authority.as_str()))
                    .ok_or("push delivery carried no Host")?
                    .trim();
                let audience = format!("https://{host}");
                if host.is_empty()
                    || host.contains(['/', '@', '\\'])
                    || validate_push_audience(&audience).is_err()
                {
                    return Err("push delivery Host does not form an https:// origin");
                }
                Ok(Cow::Owned(audience))
            }
        }
    }
}

impl Display for PushAudience {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pinned(audience) => formatter.write_str(audience),
            Self::RequestHost => formatter.write_str("<derived from request Host>"),
        }
    }
}

#[derive(Clone, Debug)]
struct Config {
    workload: Workload,
    project_id: String,
    subscription: String,
    push_audience: PushAudience,
    push_service_account: String,
    port: u16,
    process_timeout: Duration,
    request_timeout: Duration,
    /// The subscription's `ack_deadline_seconds`. Informational: it decides
    /// whether a run can outlive its delivery and be redelivered while in
    /// flight, which is a cost the operator should see at boot.
    ack_deadline: Duration,
    claim_heartbeat: Duration,
    claim_stale_after: Duration,
    claim_retention: Duration,
    /// How long a redelivery waits on a claim another delivery holds before
    /// nacking. Bounded by the process deadline so the parked request cannot
    /// itself outlive what Cloud Run allows it.
    in_flight_park_budget: Duration,
    /// How often a parked redelivery re-reads the claim.
    in_flight_poll: Duration,
    claims_collection: String,
    files_collection: String,
    content_bucket: Option<String>,
    max_delivery_attempts: i64,
}

impl Config {
    fn from_env() -> Result<Self, WorkerError> {
        reject_forbidden_settings()?;

        let workload = match required_env("GCP_QUEUE_WORKLOAD")?
            .to_ascii_lowercase()
            .as_str()
        {
            "execution" => Workload::Execution,
            "compilation" => Workload::Compilation,
            "file-tracking" => Workload::FileTracking,
            "media-transformation" => Workload::MediaTransformation,
            _ => {
                return Err(WorkerError::message(
                    "invalid_workload",
                    "GCP_QUEUE_WORKLOAD must be 'execution', 'compilation', 'file-tracking' or 'media-transformation'",
                ));
            }
        };

        let project_id = required_env("GCP_PROJECT_ID")?;
        validate_project_id(&project_id)?;
        let subscription =
            canonical_subscription(&required_env("PUBSUB_SUBSCRIPTION")?, &project_id)?;

        // The audience is the service URL Pub/Sub was told to mint tokens for.
        // Getting it wrong does not weaken anything — every delivery is simply
        // rejected — but getting it *broad* would, so a pinned value's shape is
        // checked. Absent, the audience is derived from each request's Host;
        // see `PushAudience` for why that is the default the root relies on.
        let push_audience = match optional_env("PUBSUB_PUSH_AUDIENCE") {
            Some(audience) => {
                validate_push_audience(&audience)?;
                PushAudience::Pinned(audience)
            }
            None => PushAudience::RequestHost,
        };
        let push_service_account = required_env("PUBSUB_PUSH_SERVICE_ACCOUNT")?;
        validate_service_account_email(&push_service_account)?;

        let port = parse_u64("PORT", 8080, 1, 65_535)? as u16;

        // Push subscriptions cap the ack deadline at 600, and the root sets
        // this to the value it gave the subscription. Nothing is enforced
        // against it — the claim exists precisely so nothing has to be — but
        // whether the process deadline exceeds it is worth a line at boot.
        let ack_deadline_secs = parse_u64("GCP_QUEUE_ACK_DEADLINE_SECS", 600, 10, 600)?;

        // Cloud Run terminates the request at its own `timeout_seconds`, which
        // caps at 3600. The worker has to know that number so its own deadline
        // can fire first and settle the claim, instead of being killed with an
        // `in_progress` claim that then blocks every redelivery until it ages out.
        let request_timeout_secs = parse_u64("GCP_QUEUE_REQUEST_TIMEOUT_SECS", 3_600, 60, 3_600)?;
        // The process deadline wraps store setup, event flushing and the terminal
        // ack around the executor's own run timeout, so it must fire strictly
        // after that timeout or a long run is killed mid-flight, re-run, and
        // dead-lettered without a terminal status.
        let executor_timeout_secs = std::env::var("EXECUTOR_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(3_600);
        let process_timeout_ceiling = request_timeout_secs.saturating_sub(SETTLE_MARGIN_SECS);
        let process_timeout_default = executor_timeout_secs
            .saturating_add(600)
            .min(process_timeout_ceiling);
        let process_timeout_secs = parse_u64(
            "GCP_QUEUE_PROCESS_TIMEOUT_SECS",
            process_timeout_default,
            30,
            process_timeout_ceiling,
        )?;
        // The two workloads that run a bounded inner job carry that job's own
        // timeout inside the process deadline, and the inner timeout must fire
        // first: it is what reports the failure through the callback, whereas
        // the process deadline interrupts and nacks with no terminal status.
        // Execution wraps EXECUTOR_TIMEOUT_SECS; compilation wraps
        // COMPILER_TIMEOUT_SECS (the compiler crate's own default is 600, above
        // the 570 s ceiling a 600 s request allows, so an unset variable is a
        // boot error here rather than an interrupted compile later). It is also
        // the one place where GCP is genuinely tighter than Azure: Container
        // Apps let the deadline sit at `EXECUTOR_TIMEOUT_SECS + 600` because
        // nothing above it capped the request, whereas Cloud Run stops at 3600
        // no matter what. Saying so at boot beats discovering it when a long
        // job is killed.
        let inner_timeout = match workload {
            Workload::Execution => Some(("EXECUTOR_TIMEOUT_SECS", executor_timeout_secs)),
            Workload::Compilation => Some((
                "COMPILER_TIMEOUT_SECS",
                std::env::var("COMPILER_TIMEOUT_SECS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(600),
            )),
            Workload::FileTracking | Workload::MediaTransformation => None,
        };
        if let Some((name, inner_timeout_secs)) = inner_timeout
            && process_timeout_secs <= inner_timeout_secs
        {
            return Err(WorkerError::message(
                "invalid_process_timeout",
                format!(
                    "GCP_QUEUE_PROCESS_TIMEOUT_SECS ({process_timeout_secs}) must exceed the \
                     inner job timeout ({name} = {inner_timeout_secs}) and still leave \
                     {SETTLE_MARGIN_SECS}s inside GCP_QUEUE_REQUEST_TIMEOUT_SECS \
                     ({request_timeout_secs}); lower {name}"
                ),
            ));
        }

        let claim_heartbeat_secs = parse_u64("GCP_QUEUE_CLAIM_HEARTBEAT_SECS", 30, 5, 600)?;
        let claim_stale_after_secs = parse_u64("GCP_QUEUE_CLAIM_STALE_AFTER_SECS", 180, 30, 3_600)?;
        // Three intervals, so two consecutive Firestore hiccups cannot hand a
        // live job to a second replica.
        if claim_stale_after_secs < claim_heartbeat_secs * 3 {
            return Err(WorkerError::message(
                "invalid_claim_stale_after",
                "GCP_QUEUE_CLAIM_STALE_AFTER_SECS must be at least three times \
                 GCP_QUEUE_CLAIM_HEARTBEAT_SECS",
            ));
        }
        let claim_retention_secs =
            parse_u64("GCP_QUEUE_CLAIM_RETENTION_SECS", 86_400, 3_600, 604_800)?;
        // The claim is what recognises a redelivery, so it has to outlive both
        // the longest possible run and the subscription's message retention. A
        // claim collected too early turns a duplicate back into fresh work.
        if claim_retention_secs <= process_timeout_secs.saturating_mul(2) {
            return Err(WorkerError::message(
                "invalid_claim_retention",
                "GCP_QUEUE_CLAIM_RETENTION_SECS must be more than twice \
                 GCP_QUEUE_PROCESS_TIMEOUT_SECS",
            ));
        }

        // A redelivery that finds a live claim parks for at most the staleness
        // window: a dead owner is detected within it by construction, and a
        // live owner is not one to outwait. The process deadline is the hard
        // ceiling because the parked request is itself a Cloud Run request. The
        // poll interval is short enough that a finished owner is noticed
        // promptly and long enough that a parked slot costs a handful of reads.
        let in_flight_park_budget =
            Duration::from_secs(claim_stale_after_secs.min(process_timeout_secs));
        let in_flight_poll = Duration::from_secs(claim_heartbeat_secs.min(15));

        let claims_collection = optional_env("FIRESTORE_CLAIMS_COLLECTION")
            .unwrap_or_else(|| claim::DEFAULT_CLAIMS_COLLECTION.to_string());
        let files_collection = optional_env("FIRESTORE_FILES_COLLECTION")
            .unwrap_or_else(|| file_tracking::DEFAULT_FILES_COLLECTION.to_string());

        // `GCP_CONTENT_BUCKET` is the provider-scoped name; `CONTENT_BUCKET` is
        // the generic fallback the rest of the codebase also reads.
        let content_bucket =
            optional_env("GCP_CONTENT_BUCKET").or_else(|| optional_env("CONTENT_BUCKET"));
        if let Some(bucket) = &content_bucket {
            validate_bucket_name(bucket)?;
        }

        Ok(Self {
            workload,
            project_id,
            subscription,
            push_audience,
            push_service_account,
            port,
            process_timeout: Duration::from_secs(process_timeout_secs),
            request_timeout: Duration::from_secs(request_timeout_secs),
            ack_deadline: Duration::from_secs(ack_deadline_secs),
            claim_heartbeat: Duration::from_secs(claim_heartbeat_secs),
            claim_stale_after: Duration::from_secs(claim_stale_after_secs),
            claim_retention: Duration::from_secs(claim_retention_secs),
            in_flight_park_budget,
            in_flight_poll,
            claims_collection,
            files_collection,
            content_bucket,
            // Pub/Sub's own dead-letter minimum is five attempts; this is the
            // AWS `maxReceiveCount` and Azure dequeue cap carried over so the
            // worker stops doing work at the same point on all three clouds even
            // when the broker would keep trying.
            max_delivery_attempts: parse_u64("GCP_QUEUE_MAX_DELIVERY_ATTEMPTS", 3, 1, 100)? as i64,
        })
    }

    fn require_content_bucket(&self) -> Result<String, WorkerError> {
        self.content_bucket.clone().ok_or_else(|| {
            WorkerError::message(
                "missing_configuration",
                "GCP_CONTENT_BUCKET (or CONTENT_BUCKET) is required for object-event workloads",
            )
        })
    }
}

fn reject_forbidden_settings() -> Result<(), WorkerError> {
    ensure_no_forbidden_credential_env()
        .map_err(|error| WorkerError::new("forbidden_credential_setting", error))?;
    for variable in FORBIDDEN_ENDPOINT_SETTINGS {
        if std::env::var_os(variable).is_some() {
            return Err(WorkerError::message(
                "forbidden_endpoint_setting",
                format!(
                    "{variable} is set; this worker only talks to Google's published endpoints \
                     with its own workload identity"
                ),
            ));
        }
    }
    Ok(())
}

fn required_env(name: &'static str) -> Result<String, WorkerError> {
    let value = std::env::var(name).map_err(|_| {
        WorkerError::message("missing_configuration", format!("{name} is required"))
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(WorkerError::message(
            "empty_configuration",
            format!("{name} must not be empty"),
        ));
    }
    Ok(value.to_string())
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_u64(
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, WorkerError> {
    let value = match std::env::var(name) {
        Ok(value) => value.trim().parse::<u64>().map_err(|_| {
            WorkerError::message("invalid_number", format!("{name} must be an integer"))
        })?,
        Err(_) => default,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(WorkerError::message(
            "number_out_of_range",
            format!("{name} must be between {minimum} and {maximum}"),
        ));
    }
    Ok(value)
}

fn validate_project_id(value: &str) -> Result<(), WorkerError> {
    let valid = (6..=30).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value.starts_with(|character: char| character.is_ascii_lowercase());
    if !valid {
        return Err(WorkerError::message(
            "invalid_project_id",
            "GCP_PROJECT_ID must be a 6-30 character lowercase project ID",
        ));
    }
    Ok(())
}

/// The subscription the worker serves, as the full resource name
/// `projects/<GCP_PROJECT_ID>/subscriptions/<name>` — which is what Pub/Sub
/// puts in every push envelope's `subscription` field and therefore what the
/// push handler compares against.
///
/// Both spellings are accepted from the environment: the bare id, which is what
/// the Terraform root hands over (`local.pubsub_subscription_names[stream]`),
/// and the qualified name. A bare id is qualified with `GCP_PROJECT_ID`; a
/// qualified name must already carry `GCP_PROJECT_ID`, so a subscription in a
/// foreign project can never satisfy the handler's equality test — the project
/// pinning is the point of comparing the full name at all.
fn canonical_subscription(value: &str, project_id: &str) -> Result<String, WorkerError> {
    let prefix = format!("projects/{project_id}/subscriptions/");
    let name = match value.strip_prefix(&prefix) {
        Some(name) => name,
        None if value.starts_with("projects/") => {
            return Err(WorkerError::message(
                "invalid_subscription",
                "PUBSUB_SUBSCRIPTION names a project other than GCP_PROJECT_ID; it must be \
                 <name> or projects/<GCP_PROJECT_ID>/subscriptions/<name>",
            ));
        }
        None => value,
    };
    let valid = (3..=255).contains(&name.len())
        && name.starts_with(|character: char| character.is_ascii_alphabetic())
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'+' | b'%')
        })
        // Pub/Sub reserves the `goog` prefix for its own resources.
        && !name.to_ascii_lowercase().starts_with("goog");
    if !valid {
        return Err(WorkerError::message(
            "invalid_subscription",
            "PUBSUB_SUBSCRIPTION carries an invalid subscription name",
        ));
    }
    Ok(format!("{prefix}{name}"))
}

/// The audience must be one absolute HTTPS origin — the service URL. A value
/// with a query string or a fragment would silently never match the `aud` claim
/// Pub/Sub mints, so it is refused rather than left to fail at 3am.
fn validate_push_audience(value: &str) -> Result<(), WorkerError> {
    let valid = value.starts_with("https://")
        && value.len() > "https://".len()
        && !value.contains(['?', '#', ' ', '\t'])
        && !value.ends_with('/');
    if !valid {
        return Err(WorkerError::message(
            "invalid_push_audience",
            "PUBSUB_PUSH_AUDIENCE must be an https:// URL with no query, fragment or trailing slash",
        ));
    }
    Ok(())
}

fn validate_service_account_email(value: &str) -> Result<(), WorkerError> {
    let valid = value.ends_with(".gserviceaccount.com")
        && value.matches('@').count() == 1
        && value.split('@').all(|part| !part.is_empty())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '@')
        });
    if !valid {
        return Err(WorkerError::message(
            "invalid_push_service_account",
            "PUBSUB_PUSH_SERVICE_ACCOUNT must be a <name>@<project>.iam.gserviceaccount.com address",
        ));
    }
    Ok(())
}

fn validate_bucket_name(value: &str) -> Result<(), WorkerError> {
    let valid = (3..=63).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
        && value.starts_with(|character: char| character.is_ascii_alphanumeric())
        && value.ends_with(|character: char| character.is_ascii_alphanumeric())
        && !value.contains("..")
        // Cloud Storage reserves the `goog` prefix for its own buckets.
        && !value.starts_with("goog");
    if !valid {
        return Err(WorkerError::message(
            "invalid_bucket_name",
            "the content bucket name is not a valid Cloud Storage bucket name",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct WorkerError {
    code: &'static str,
    detail: String,
}

impl WorkerError {
    fn new(code: &'static str, error: impl Display) -> Self {
        Self {
            code,
            detail: error.to_string(),
        }
    }

    fn message(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl Display for WorkerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for WorkerError {}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let terminate = signal(SignalKind::terminate());
    match terminate {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch_message(json: &str) -> PubsubMessage {
        PubsubMessage {
            data: BASE64_STANDARD.encode(json),
            ..Default::default()
        }
    }

    #[test]
    fn validates_the_resource_shaped_settings() {
        assert!(validate_project_id("flow-like-gcp-dev").is_ok());
        assert!(validate_project_id("Flow-Like").is_err());
        assert!(validate_push_audience("https://worker-abc.a.run.app").is_ok());
        assert!(validate_push_audience("http://worker-abc.a.run.app").is_err());
        assert!(validate_push_audience("https://worker-abc.a.run.app/").is_err());
        assert!(
            validate_service_account_email("flow-like-executor@p.iam.gserviceaccount.com").is_ok()
        );
        assert!(validate_service_account_email("someone@example.com").is_err());
        assert!(validate_bucket_name("flow-like-gcp-dev-content").is_ok());
        assert!(validate_bucket_name("Flow-Like-Content").is_err());
    }

    /// The root hands over the bare id; the push envelope carries the full
    /// name. Both must resolve to the same string, and only the same project.
    #[test]
    fn subscription_is_canonicalised_to_the_full_name_in_this_project() {
        let full = "projects/flow-like-gcp-dev/subscriptions/flow-like-execution-sub";
        assert_eq!(
            canonical_subscription("flow-like-execution-sub", "flow-like-gcp-dev").unwrap(),
            full
        );
        assert_eq!(
            canonical_subscription(full, "flow-like-gcp-dev").unwrap(),
            full
        );
        assert!(
            canonical_subscription(
                "projects/other-project/subscriptions/flow-like-execution-sub",
                "flow-like-gcp-dev"
            )
            .is_err()
        );
        assert!(canonical_subscription("goog-reserved", "flow-like-gcp-dev").is_err());
        assert!(canonical_subscription("x", "flow-like-gcp-dev").is_err());
    }

    /// With no pinned value the audience is the origin Pub/Sub connected to,
    /// which is the URL the subscription's oidc_token.audience names.
    #[test]
    fn push_audience_is_derived_from_the_request_host_when_not_pinned() {
        let derived = PushAudience::RequestHost;
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "worker-abc-uc.a.run.app".parse().unwrap());
        assert_eq!(
            derived.resolve(&headers, &Uri::from_static("/")).unwrap(),
            "https://worker-abc-uc.a.run.app"
        );
        // HTTP/2 puts the authority in the URI rather than in a Host header.
        assert_eq!(
            derived
                .resolve(
                    &HeaderMap::new(),
                    &Uri::from_static("https://worker-abc-uc.a.run.app/")
                )
                .unwrap(),
            "https://worker-abc-uc.a.run.app"
        );
        assert!(
            derived
                .resolve(&HeaderMap::new(), &Uri::from_static("/"))
                .is_err()
        );
        let mut bad = HeaderMap::new();
        bad.insert(HOST, "worker-abc-uc.a.run.app/x?y".parse().unwrap());
        assert!(derived.resolve(&bad, &Uri::from_static("/")).is_err());

        let pinned = PushAudience::Pinned("https://pinned.example".to_string());
        assert_eq!(
            pinned.resolve(&headers, &Uri::from_static("/")).unwrap(),
            "https://pinned.example"
        );
    }

    #[test]
    fn unwraps_versioned_dispatch_envelopes() {
        let json =
            r#"{"v":1,"job_id":"job-1","payload":{"remote_url":"https://example.invalid/a"}}"#;
        let PreparedMessage::Dispatch { job_id, payload } =
            prepare(&dispatch_message(json), Workload::Execution).expect("envelope")
        else {
            panic!("execution messages carry a dispatch envelope");
        };
        assert_eq!(job_id, "job-1");
        assert_eq!(payload, r#"{"remote_url":"https://example.invalid/a"}"#);
    }

    #[test]
    fn rejects_unsupported_envelope_version() {
        let json = r#"{"v":2,"job_id":"job-1","payload":{}}"#;
        assert_eq!(
            prepare(&dispatch_message(json), Workload::Execution).unwrap_err(),
            "envelope_version_unsupported"
        );
    }

    #[test]
    fn object_event_workloads_read_the_attributes_not_the_envelope() {
        let message = PubsubMessage {
            attributes: [
                ("eventType", "OBJECT_FINALIZE"),
                ("bucketId", "flow-like-content"),
                ("objectId", "apps/a1/x.bin"),
                ("objectGeneration", "17"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
            ..Default::default()
        };
        let PreparedMessage::ObjectEvent(event) =
            prepare(&message, Workload::FileTracking).expect("object event")
        else {
            panic!("object-event workloads never see a dispatch envelope");
        };
        assert_eq!(event.object, "apps/a1/x.bin");
        assert_eq!(event.generation, Some(17));
    }

    /// The two thresholds have to line up: below the broker's dead-letter
    /// minimum the worker keeps nacking so the `dead_letter_policy` can take the
    /// message, and above it the worker acks so a message nobody will collect
    /// stops circulating.
    #[test]
    fn the_give_up_point_never_starves_the_dead_letter_policy() {
        for attempts in 1..=PUBSUB_DEAD_LETTER_MIN_ATTEMPTS {
            assert!(!give_up_and_ack(attempts, 3));
        }
        assert!(give_up_and_ack(PUBSUB_DEAD_LETTER_MIN_ATTEMPTS + 1, 3));
        // A ceiling above the broker's minimum governs on its own.
        assert!(!give_up_and_ack(8, 10));
        assert!(give_up_and_ack(11, 10));
    }

    #[test]
    fn decodes_both_base64_alphabets_and_an_absent_body() {
        let raw = br#"{"v":1}"#;
        let standard = PubsubMessage {
            data: BASE64_STANDARD.encode(raw),
            ..Default::default()
        };
        let url_safe = PubsubMessage {
            data: BASE64_URL_SAFE.encode(raw),
            ..Default::default()
        };
        assert_eq!(standard.decoded_data().as_deref(), Some(&raw[..]));
        assert_eq!(url_safe.decoded_data().as_deref(), Some(&raw[..]));
        assert_eq!(PubsubMessage::default().decoded_data(), Some(Vec::new()));
    }
}
