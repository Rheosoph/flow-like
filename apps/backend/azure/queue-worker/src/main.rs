#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod blob_events;
mod file_tracking;
mod media_transformation;

use azure_core_queue::base64;
use azure_core_queue::http::headers::HeaderName;
use azure_core_queue::http::{RequestContent, Url, XmlFormat};
use azure_core_queue::time::{OffsetDateTime, to_rfc3339};
use azure_identity_queue::{
    ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId,
};
use azure_storage_queue::models::{
    QueueClientReceiveMessagesOptions, QueueClientSendMessageOptions, QueueMessage, ReceivedMessage,
};
use azure_storage_queue::{QueueClient, QueueServiceClient};
use blob_events::{BlobEvent, BlobEventError};
use file_tracking::{FileTrackingContext, FileTrackingError};
use flow_like_azure_data::cosmos::CosmosClient;
use flow_like_azure_data::postgres::{self, ManagedIdentityPostgresConfig};
use flow_like_compiler::{CompilerConfig, CompilerError, compile};
use flow_like_executor::{
    ExecutionRequest, ExecutorConfig, ExecutorError, MAX_REMOTE_PAYLOAD_BYTES, ResolveError,
    fetch_bounded, report_queue_failure, resolve_payload_from_str,
};
use flow_like_storage::object_store::azure::MicrosoftAzureBuilder;
use flow_like_types::dispatch::{CompilationJob, CompilationJobRef};
use media_transformation::{MediaTransformationContext, MediaTransformationError};
use serde::Deserialize;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Every `Update Message` response carries a fresh receipt in this header and
/// invalidates the previous one.
const POP_RECEIPT_HEADER: HeaderName = HeaderName::from_static("x-ms-popreceipt");
/// Queue Storage caps an encoded message at 64 KiB and base64 expands 3:4, so
/// this is the exact largest body that can ever have been sent.
const MAX_MESSAGE_BODY_BYTES: usize = 49_152;
/// Poison records embed the original text as a JSON string. Worst-case escaping
/// doubles it, so 16 KiB keeps the re-encoded record inside the wire limit.
const MAX_POISON_BODY_BYTES: usize = 16_384;
/// Dead-lettered messages must not expire; operators drain the poison queues.
const POISON_MESSAGE_TTL_SECS: i32 = -1;
const ENVELOPE_VERSION: u32 = 1;
const FORBIDDEN_CREDENTIAL_SETTINGS: &[&str] = &[
    "AZURE_STORAGE_CONNECTION_STRING",
    "AZURE_STORAGE_ACCOUNT_KEY",
    "AZURE_STORAGE_KEY",
    "AZURE_STORAGE_SAS_TOKEN",
    "AZURE_STORAGE_ACCESS_KEY",
    "AZURE_CLIENT_SECRET",
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

    tracing::info!(
        workload = %config.workload,
        account = %config.account_name,
        queue = %config.queue_name,
        poison_queue = %config.poison_queue_name,
        max_dequeue_count = config.max_dequeue_count,
        "starting passwordless Azure Queue Storage worker"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_shutdown = shutdown_tx.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        let _ = signal_shutdown.send(true);
    });

    let (context, database_lifecycle) = WorkloadContext::build(&config).await?;

    // The PostgreSQL pool holds one managed-identity token that cannot be
    // refreshed in place (see flow_like_azure_data::postgres). The worker
    // therefore stops taking messages once the token enters its safety window
    // and exits before expiry; KEDA/Container Apps start a fresh replica.
    let hard_stop = async {
        match database_lifecycle {
            Some(lifecycle) => {
                let drain_shutdown = shutdown_tx.clone();
                let drain_lifecycle = lifecycle.clone();
                tokio::spawn(async move {
                    drain_lifecycle.wait_until_drain().await;
                    tracing::warn!(
                        token_lifetime_seconds = drain_lifecycle.remaining_seconds(),
                        "database access token entered its safety window; draining"
                    );
                    let _ = drain_shutdown.send(true);
                });
                lifecycle.wait_until_hard_stop().await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(hard_stop);

    let worker = run(config, context, shutdown_rx);
    tokio::pin!(worker);

    tokio::select! {
        result = &mut worker => result?,
        _ = &mut hard_stop => {
            tracing::error!(
                graceful_shutdown_seconds = postgres::GRACEFUL_SHUTDOWN_SECONDS,
                "forcing worker rotation before the database access token expires"
            );
        }
    }
    Ok(())
}

async fn run(
    config: Config,
    context: WorkloadContext,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), WorkerError> {
    let mut retry_delay = Duration::from_secs(1);

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }

        match run_connection(&config, &context, &mut shutdown).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                // No connection strings or SAS credentials exist in this process,
                // so the SDK error cannot contain either of those secret classes.
                tracing::error!(error = %error, "Queue Storage poll loop failed");
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(retry_delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
        }
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

async fn run_connection(
    config: &Config,
    context: &WorkloadContext,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), WorkerError> {
    let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
        user_assigned_id: Some(UserAssignedId::ClientId(config.client_id.clone())),
        ..Default::default()
    }))
    .map_err(|error| WorkerError::new("managed_identity_init", error))?;

    let service_url = Url::parse(&format!(
        "https://{}.queue.core.windows.net/",
        config.account_name
    ))
    .map_err(|error| WorkerError::new("invalid_queue_endpoint", error))?;
    let service = QueueServiceClient::new(service_url, Some(credential), None)
        .map_err(|error| WorkerError::new("queue_client_open", error))?;
    let source = service
        .queue_client(&config.queue_name)
        .map_err(|error| WorkerError::new("queue_client_open", error))?;
    let poison = service
        .queue_client(&config.poison_queue_name)
        .map_err(|error| WorkerError::new("poison_queue_client_open", error))?;

    // Queue Storage has no long poll: `Get Messages` always returns
    // immediately. Poll tightly while work flows and back off to the ceiling
    // once the queue drains; KEDA scales the replica to zero when it stays idle.
    let mut poll_delay = config.poll_min_interval;

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }

        let response = source
            .receive_messages(Some(QueueClientReceiveMessagesOptions {
                number_of_messages: Some(config.batch_size),
                visibility_timeout: Some(config.visibility_timeout_secs),
                ..Default::default()
            }))
            .await
            .map_err(|error| WorkerError::new("queue_receive", error))?;
        let messages = response
            .into_model()
            .map_err(|error| WorkerError::new("queue_receive_decode", error))?
            .items
            .unwrap_or_default();

        if messages.is_empty() {
            tokio::select! {
                _ = tokio::time::sleep(poll_delay) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
            poll_delay = (poll_delay * 2).min(config.poll_max_interval);
            continue;
        }

        poll_delay = config.poll_min_interval;
        for message in messages {
            match process_and_settle(&source, &poison, message, config, context, shutdown).await? {
                ProcessDirective::Continue => {}
                ProcessDirective::Shutdown => return Ok(()),
            }
        }
    }
}

async fn process_and_settle(
    source: &QueueClient,
    poison: &QueueClient,
    message: ReceivedMessage,
    config: &Config,
    context: &WorkloadContext,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<ProcessDirective, WorkerError> {
    let (Some(message_id), Some(initial_receipt)) =
        (message.message_id.clone(), message.pop_receipt.clone())
    else {
        // Neither settlement verb can be issued without both values, so the
        // only safe action is to let the visibility timeout expire.
        tracing::error!("queue message is missing its ID or pop receipt");
        return Ok(ProcessDirective::Continue);
    };
    let dequeue_count = message.dequeue_count.unwrap_or(1);

    // The receipt rotates on every successful `Update Message` and the previous
    // value dies with it. It is therefore a single mutable local that the
    // renewal arm reassigns in place, and every later `Delete Message` or
    // poison move reads at call time. Renewal deliberately runs inside the same
    // `select!` as the work instead of a spawned task: a receipt owned by two
    // tasks is exactly the race that silently redelivers finished jobs.
    let mut pop_receipt = initial_receipt;

    if dequeue_count > config.max_dequeue_count {
        return poison_message(
            source,
            poison,
            &message,
            &message_id,
            &pop_receipt,
            dequeue_count,
            "max_dequeue_exceeded",
            config,
        )
        .await;
    }

    let prepared = match prepare(&message, config.workload) {
        Ok(prepared) => prepared,
        Err(code) => {
            return poison_message(
                source,
                poison,
                &message,
                &message_id,
                &pop_receipt,
                dequeue_count,
                code,
                config,
            )
            .await;
        }
    };

    let outcome = {
        let processing =
            process_message(context, config.workload, prepared.payload, prepared.job_id);
        tokio::pin!(processing);

        let deadline = tokio::time::sleep(config.process_timeout);
        tokio::pin!(deadline);
        let mut renewal = tokio::time::interval(config.renewal_interval);
        renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Consume the immediate first tick; renew only after a full interval.
        renewal.tick().await;

        loop {
            tokio::select! {
                outcome = &mut processing => break outcome,
                _ = renewal.tick() => {
                    match extend_visibility(source, &message_id, &pop_receipt, config.visibility_timeout_secs).await {
                        Ok(renewed) => pop_receipt = renewed,
                        Err(error) => {
                            // The lease is gone: the receipt no longer proves
                            // ownership, so settling would either fail or steal
                            // another replica's message. Drop the work and let
                            // the message become visible on its own.
                            tracing::warn!(
                                error = %error,
                                message_id = %message_id,
                                dequeue_count,
                                "queue_lease_lost"
                            );
                            return Ok(ProcessDirective::Continue);
                        }
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
            source
                .delete_message(&message_id, &pop_receipt, None)
                .await
                .map_err(|error| WorkerError::new("queue_delete", error))?;
            tracing::info!(
                message_id = %message_id,
                dequeue_count,
                "queue workload completed"
            );
            Ok(ProcessDirective::Continue)
        }
        ProcessOutcome::Permanent(code) => {
            poison_message(
                source,
                poison,
                &message,
                &message_id,
                &pop_receipt,
                dequeue_count,
                code,
                config,
            )
            .await
        }
        ProcessOutcome::Retryable(code) if dequeue_count >= config.max_dequeue_count => {
            poison_message(
                source,
                poison,
                &message,
                &message_id,
                &pop_receipt,
                dequeue_count,
                code,
                config,
            )
            .await
        }
        ProcessOutcome::Retryable(code) => {
            release(source, &message_id, &pop_receipt).await;
            tracing::warn!(
                message_id = %message_id,
                dequeue_count,
                failure_code = code,
                "queue workload failed and was released for retry"
            );
            Ok(ProcessDirective::Continue)
        }
        ProcessOutcome::Interrupted => {
            release(source, &message_id, &pop_receipt).await;
            Ok(ProcessDirective::Shutdown)
        }
    }
}

/// Extend the lease and return the receipt that replaces the one just spent.
///
/// The caller must overwrite its receipt with this value; the receipt passed in
/// is invalid the instant the service accepts the update.
async fn extend_visibility(
    client: &QueueClient,
    message_id: &str,
    pop_receipt: &str,
    visibility_timeout_secs: i32,
) -> Result<String, WorkerError> {
    let response = client
        .update_message(message_id, pop_receipt, visibility_timeout_secs, None)
        .await
        .map_err(|error| WorkerError::new("queue_update_message", error))?;
    response
        .headers()
        .get_optional_str(&POP_RECEIPT_HEADER)
        .map(str::to_string)
        .ok_or_else(|| {
            WorkerError::message(
                "missing_pop_receipt",
                "Update Message returned no x-ms-popreceipt header",
            )
        })
}

/// Make a message immediately visible again. This is the Queue Storage
/// equivalent of abandoning a peek-locked message; a failure is ignored because
/// lease expiry produces the same redelivery.
async fn release(client: &QueueClient, message_id: &str, pop_receipt: &str) {
    if let Err(error) = client
        .update_message(message_id, pop_receipt, 0, None)
        .await
    {
        tracing::warn!(error = %error, message_id = %message_id, "queue message release failed");
    }
}

/// Move a message into its poison sibling and delete the original.
///
/// Put-then-delete is deliberate: Queue Storage offers no transaction, and a
/// duplicated forensic record is harmless where a lost one is not. A failed put
/// leaves the original untouched so the next delivery retries the move.
#[allow(clippy::too_many_arguments)]
async fn poison_message(
    source: &QueueClient,
    poison: &QueueClient,
    message: &ReceivedMessage,
    message_id: &str,
    pop_receipt: &str,
    dequeue_count: i64,
    reason: &str,
    config: &Config,
) -> Result<ProcessDirective, WorkerError> {
    // Dead-lettering alone would leave the run record non-terminal until it
    // expires; pollers would watch a forever-Running execution.
    fail_poisoned_execution_run(message, reason, config).await;

    let original = message.message_text.clone().unwrap_or_default();
    let truncated = truncate_on_char_boundary(&original, MAX_POISON_BODY_BYTES);
    // Queue Storage messages carry no user-settable properties, so the failure
    // reason and the broker metadata have to travel inside the body.
    let record = serde_json::json!({
        "v": ENVELOPE_VERSION,
        "poisoned_at": to_rfc3339(&OffsetDateTime::now_utc()),
        "source_queue": config.queue_name,
        "reason": reason,
        "dequeue_count": dequeue_count,
        "source_message_id": message_id,
        "insertion_time": message.insertion_time.as_ref().map(to_rfc3339),
        "body_truncated": truncated.len() < original.len(),
        "body_original_len": original.len(),
        "body": truncated,
    });
    let body = serde_json::to_vec(&record)
        .map_err(|error| WorkerError::new("poison_record_encode", error))?;

    let content: RequestContent<QueueMessage, XmlFormat> = QueueMessage {
        message_text: Some(base64::encode(body)),
    }
    .try_into()
    .map_err(|error| WorkerError::new("poison_record_encode", error))?;

    let put = poison
        .send_message(
            content,
            Some(QueueClientSendMessageOptions {
                message_time_to_live: Some(POISON_MESSAGE_TTL_SECS),
                ..Default::default()
            }),
        )
        .await;

    if let Err(error) = put {
        tracing::error!(
            error = %error,
            message_id = %message_id,
            source_queue = %config.queue_name,
            poison_queue = %config.poison_queue_name,
            reason,
            "queue poison move failed; leaving the message for redelivery"
        );
        return Ok(ProcessDirective::Continue);
    }

    source
        .delete_message(message_id, pop_receipt, None)
        .await
        .map_err(|error| WorkerError::new("queue_delete_after_poison", error))?;

    tracing::error!(
        source_queue = %config.queue_name,
        poison_queue = %config.poison_queue_name,
        source_message_id = %message_id,
        dequeue_count,
        reason,
        "queue_message_poisoned"
    );
    Ok(ProcessDirective::Continue)
}

/// Best-effort terminal `Failed` for the run carried by an execution message
/// that is about to be dead-lettered.
///
/// The identity material (executor JWT, signed broker job ID) is re-derived
/// from the message body, so this covers every poison path whose payload is
/// still readable; unreadable or unfetchable payloads are skipped. A failure
/// here must never block the poison move.
async fn fail_poisoned_execution_run(message: &ReceivedMessage, reason: &str, config: &Config) {
    if config.workload != Workload::Execution {
        return;
    }
    let Ok(prepared) = prepare(message, Workload::Execution) else {
        return;
    };
    let payload = match resolve_payload_from_str(&prepared.payload).await {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "cannot resolve poisoned execution payload; run record stays non-terminal"
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
            "failed to persist terminal Failed status for poisoned execution"
        );
    }
}

fn truncate_on_char_boundary(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[derive(Debug)]
struct PreparedMessage {
    job_id: String,
    payload: String,
}

/// Decode the wire text and unwrap the dispatch envelope.
///
/// Producers base64-encode the JSON body so that no XML-hostile byte can make a
/// message permanently unreadable. Raw text is still accepted: a JSON document
/// starts with `{` or `[`, neither of which is in the base64 alphabet, so the
/// two encodings can never be confused.
fn prepare(message: &ReceivedMessage, workload: Workload) -> Result<PreparedMessage, &'static str> {
    let text = message.message_text.as_deref().unwrap_or_default();
    let decoded = base64::decode(text).unwrap_or_else(|_| text.as_bytes().to_vec());
    if decoded.len() > MAX_MESSAGE_BODY_BYTES {
        return Err("message_too_large");
    }

    // Blob-event workloads receive Event Grid deliveries (CloudEvents or Event
    // Grid schema), never the dispatch envelope. The event id stands in for
    // the job id; the payload is the raw event so the handler re-parses it.
    if workload.is_blob_event_workload() {
        let event = BlobEvent::parse(&decoded).map_err(|error| match error {
            BlobEventError::Malformed => workload.invalid_payload_code(),
            BlobEventError::UnsupportedType => "unsupported_blob_event_type",
        })?;
        let payload = String::from_utf8(decoded).map_err(|_| workload.invalid_payload_code())?;
        return Ok(PreparedMessage {
            job_id: event.id,
            payload,
        });
    }

    let envelope = serde_json::from_slice::<DispatchEnvelope>(&decoded)
        .map_err(|_| workload.invalid_payload_code())?;
    if envelope.v != ENVELOPE_VERSION {
        return Err("envelope_version_unsupported");
    }
    let payload =
        serde_json::to_string(&envelope.payload).map_err(|_| workload.invalid_payload_code())?;

    Ok(PreparedMessage {
        job_id: envelope.job_id,
        payload,
    })
}

async fn process_message(
    context: &WorkloadContext,
    workload: Workload,
    payload: String,
    job_id: String,
) -> ProcessOutcome {
    match workload {
        Workload::FileTracking | Workload::MediaTransformation => {
            let event = match BlobEvent::parse(payload.as_bytes()) {
                Ok(event) => event,
                Err(_) => return ProcessOutcome::Permanent(workload.invalid_payload_code()),
            };
            if event.id != job_id {
                return ProcessOutcome::Permanent("blob_event_id_mismatch");
            }
            match context {
                WorkloadContext::FileTracking(ctx) => {
                    match file_tracking::process(ctx, &event).await {
                        Ok(()) => ProcessOutcome::Success,
                        Err(FileTrackingError::Permanent(code)) => ProcessOutcome::Permanent(code),
                        Err(FileTrackingError::Retryable(code)) => ProcessOutcome::Retryable(code),
                    }
                }
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
            }
        }
        Workload::Execution => {
            // The payload is either inline or a claim-check reference to a
            // staged blob, so the job ID is only known after resolution.
            let payload = match resolve_payload_from_str(&payload).await {
                Ok(payload) => payload,
                Err(ResolveError::Fetch(_)) => {
                    return ProcessOutcome::Retryable("execution_payload_fetch_failed");
                }
                Err(ResolveError::Deserialize(_)) => {
                    return ProcessOutcome::Permanent("invalid_execution_payload");
                }
            };
            // Queue Storage mints its own MessageId, so the identity the
            // producer signed travels in the envelope instead of broker
            // metadata. The check itself is unchanged.
            if payload.job_id != job_id {
                return ProcessOutcome::Permanent("execution_message_id_mismatch");
            }
            let request = match ExecutionRequest::try_from(payload) {
                Ok(request) => request,
                Err(_) => return ProcessOutcome::Permanent("invalid_execution_request"),
            };

            let executor_config = ExecutorConfig::from_env().with_required_terminal_status_ack();
            match flow_like_executor::execute(request, executor_config).await {
                // Strict queue mode only returns success after the API has
                // durably acknowledged a terminal run status. User-code
                // failure is therefore a completed delivery, while callback
                // failure releases the message for retry.
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
    }
}

/// Resolve a queue body into a concrete [`CompilationJob`].
///
/// Mirrors `flow_like_executor::resolve_payload_from_str` for compilation,
/// which has no shared resolver. A failed fetch of the staged blob is a
/// transient condition and stays retryable; anything that cannot be parsed
/// is permanently invalid.
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
/// `job_id` replaces the Service Bus broker message ID as the signed identity
/// the worker compares against the resolved payload.
#[derive(Debug, Deserialize)]
struct DispatchEnvelope {
    v: u32,
    job_id: String,
    payload: serde_json::Value,
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
            Self::FileTracking | Self::MediaTransformation => "invalid_blob_event",
        }
    }

    /// Fed by Event Grid blob subscriptions instead of the API's dispatch
    /// envelope; see `prepare`.
    fn is_blob_event_workload(self) -> bool {
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
/// none: they reach the API over HTTPS with the JWT inside the signed
/// payload. The blob-event workloads are the first that need data-plane
/// clients of their own, always through the assigned managed identity.
enum WorkloadContext {
    Dispatch,
    FileTracking(FileTrackingContext),
    MediaTransformation(MediaTransformationContext),
}

impl WorkloadContext {
    async fn build(
        config: &Config,
    ) -> Result<(Self, Option<postgres::DatabaseTokenLifecycle>), WorkerError> {
        match config.workload {
            Workload::Execution | Workload::Compilation => Ok((Self::Dispatch, None)),
            Workload::FileTracking => {
                let cosmos = CosmosClient::from_env()
                    .map_err(|error| WorkerError::new("cosmos_client_init", error))?;
                let files_container = std::env::var("COSMOS_FILES_CONTAINER")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| file_tracking::DEFAULT_FILES_CONTAINER.to_string());
                let content_container = required_env("AZURE_CONTENT_CONTAINER")?;
                let postgres_config =
                    ManagedIdentityPostgresConfig::from_env(Some(&config.client_id))
                        .map_err(|error| WorkerError::new("postgres_config", error))?;
                let managed =
                    postgres::connect_as(&postgres_config, "flow-like-azure-file-tracker")
                        .await
                        .map_err(|error| WorkerError::new("postgres_connect", error))?;
                tracing::info!(
                    token_lifetime_seconds = managed.lifecycle.remaining_seconds(),
                    files_container = %files_container,
                    "file tracker connected to Cosmos and PostgreSQL with managed identity"
                );
                Ok((
                    Self::FileTracking(FileTrackingContext {
                        cosmos,
                        files_container,
                        content_container,
                        database: managed.connection,
                    }),
                    Some(managed.lifecycle),
                ))
            }
            Workload::MediaTransformation => {
                let account = required_env("AZURE_STORAGE_ACCOUNT_NAME")?;
                validate_account_name(&account)?;
                let content_container = required_env("AZURE_CONTENT_CONTAINER")?;
                // from_env picks up the Container Apps identity endpoint and
                // AZURE_CLIENT_ID; no key or SAS setting is present (rejected
                // at startup).
                let store = MicrosoftAzureBuilder::from_env()
                    .with_account(&account)
                    .with_container_name(&content_container)
                    .build()
                    .map_err(|error| WorkerError::new("blob_store_init", error))?;
                Ok((
                    Self::MediaTransformation(MediaTransformationContext {
                        store: Arc::new(store),
                        content_container,
                    }),
                    None,
                ))
            }
        }
    }
}

#[derive(Debug)]
enum ProcessOutcome {
    Success,
    Permanent(&'static str),
    Retryable(&'static str),
    Interrupted,
}

#[derive(Debug)]
enum ProcessDirective {
    Continue,
    Shutdown,
}

#[derive(Clone, Debug)]
struct Config {
    account_name: String,
    queue_name: String,
    poison_queue_name: String,
    client_id: String,
    workload: Workload,
    visibility_timeout_secs: i32,
    renewal_interval: Duration,
    process_timeout: Duration,
    max_dequeue_count: i64,
    batch_size: i32,
    poll_min_interval: Duration,
    poll_max_interval: Duration,
}

impl Config {
    fn from_env() -> Result<Self, WorkerError> {
        reject_secret_credentials()?;

        let workload = match required_env("AZURE_QUEUE_WORKLOAD")?
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
                    "AZURE_QUEUE_WORKLOAD must be 'execution', 'compilation', 'file-tracking' or 'media-transformation'",
                ));
            }
        };
        let account_name = required_env("AZURE_QUEUE_STORAGE_ACCOUNT_NAME")?;
        validate_account_name(&account_name)?;
        let queue_name = required_env("AZURE_QUEUE_NAME")?;
        validate_queue_name(&queue_name)?;
        let poison_queue_name = required_env("AZURE_QUEUE_POISON_NAME")?;
        validate_queue_name(&poison_queue_name)?;
        if poison_queue_name != format!("{queue_name}-poison") {
            return Err(WorkerError::message(
                "invalid_poison_queue_name",
                "AZURE_QUEUE_POISON_NAME must be AZURE_QUEUE_NAME suffixed with '-poison'",
            ));
        }

        let client_id = required_env("AZURE_CLIENT_ID")?;
        uuid::Uuid::parse_str(&client_id).map_err(|_| {
            WorkerError::message("invalid_client_id", "AZURE_CLIENT_ID must be a UUID")
        })?;

        let visibility_timeout_secs =
            parse_u64("AZURE_QUEUE_VISIBILITY_TIMEOUT_SECS", 300, 60, 604_800)?;
        let renewal_interval_secs = parse_u64("AZURE_QUEUE_RENEWAL_INTERVAL_SECS", 60, 10, 3_600)?;
        // Renewal has to land well inside the lease, or a single slow request
        // lets the message reappear while this replica is still working on it.
        if renewal_interval_secs * 2 >= visibility_timeout_secs {
            return Err(WorkerError::message(
                "invalid_renewal_interval",
                "AZURE_QUEUE_RENEWAL_INTERVAL_SECS must be less than half of AZURE_QUEUE_VISIBILITY_TIMEOUT_SECS",
            ));
        }
        // The process deadline wraps lease acquisition, store setup, event
        // flushing and the terminal ack around the executor's own run timeout,
        // so it must fire strictly after that timeout or a long run is killed
        // mid-flight, re-run, and poisoned without a terminal status.
        let executor_timeout_secs = std::env::var("EXECUTOR_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(3_600);
        let process_timeout_default = executor_timeout_secs.saturating_add(600);
        let process_timeout_secs = parse_u64(
            "AZURE_QUEUE_PROCESS_TIMEOUT_SECS",
            process_timeout_default,
            30,
            process_timeout_default.max(7_200),
        )?;
        if process_timeout_secs <= executor_timeout_secs {
            return Err(WorkerError::message(
                "invalid_process_timeout",
                "AZURE_QUEUE_PROCESS_TIMEOUT_SECS must exceed the executor run timeout (EXECUTOR_TIMEOUT_SECS, default 3600)",
            ));
        }
        let process_timeout = Duration::from_secs(process_timeout_secs);

        let poll_min_interval_secs = parse_u64("AZURE_QUEUE_POLL_MIN_INTERVAL_SECS", 1, 1, 60)?;
        let poll_max_interval_secs = parse_u64("AZURE_QUEUE_POLL_MAX_INTERVAL_SECS", 30, 1, 300)?;
        if poll_max_interval_secs < poll_min_interval_secs {
            return Err(WorkerError::message(
                "invalid_poll_interval",
                "AZURE_QUEUE_POLL_MAX_INTERVAL_SECS must not be below AZURE_QUEUE_POLL_MIN_INTERVAL_SECS",
            ));
        }

        Ok(Self {
            account_name,
            queue_name,
            poison_queue_name,
            client_id,
            workload,
            visibility_timeout_secs: visibility_timeout_secs as i32,
            renewal_interval: Duration::from_secs(renewal_interval_secs),
            process_timeout,
            // Queue Storage has no broker delivery cap, so the limit is
            // enforced here from DequeueCount before any work starts.
            max_dequeue_count: parse_u64("AZURE_QUEUE_MAX_DEQUEUE_COUNT", 3, 1, 100)? as i64,
            // A batch greater than 1 would leave waiting messages unrenewed on
            // their initial visibility: they redeliver elsewhere and their
            // stale receipts later tear down the connection loop. The knob
            // refuses unsafe values until waiting-message renewal exists.
            batch_size: parse_u64("AZURE_QUEUE_BATCH_SIZE", 1, 1, 1)? as i32,
            poll_min_interval: Duration::from_secs(poll_min_interval_secs),
            poll_max_interval: Duration::from_secs(poll_max_interval_secs),
        })
    }
}

fn reject_secret_credentials() -> Result<(), WorkerError> {
    for variable in FORBIDDEN_CREDENTIAL_SETTINGS {
        if std::env::var(variable)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(WorkerError::message(
                "forbidden_secret_credential",
                format!("{variable} is forbidden; use the assigned managed identity"),
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

fn parse_u64(
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, WorkerError> {
    let value = match std::env::var(name) {
        Ok(value) => value.parse::<u64>().map_err(|_| {
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

fn validate_account_name(value: &str) -> Result<(), WorkerError> {
    let valid = (3..=24).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !valid {
        return Err(WorkerError::message(
            "invalid_account_name",
            "AZURE_QUEUE_STORAGE_ACCOUNT_NAME must be 3-24 lowercase alphanumeric characters",
        ));
    }
    Ok(())
}

fn validate_queue_name(value: &str) -> Result<(), WorkerError> {
    let valid = (3..=63).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--");
    if !valid {
        return Err(WorkerError::message(
            "invalid_queue_name",
            "Queue Storage queue name is invalid",
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

    fn message(text: &str) -> ReceivedMessage {
        let mut message = ReceivedMessage::default();
        message.message_text = Some(text.to_string());
        message
    }

    #[test]
    fn validates_account_and_queue_shapes() {
        assert!(validate_account_name("flowlikedev01").is_ok());
        assert!(validate_account_name("flowlike-dev.queue.core.windows.net").is_err());
        assert!(validate_queue_name("execution-poison").is_ok());
        assert!(validate_queue_name("execution/jobs").is_err());
    }

    #[test]
    fn accepts_base64_and_raw_envelopes() {
        let json =
            r#"{"v":1,"job_id":"job-1","payload":{"remote_url":"https://example.invalid/a"}}"#;

        let raw = prepare(&message(json), Workload::Execution).expect("raw envelope");
        assert_eq!(raw.job_id, "job-1");
        assert_eq!(raw.payload, r#"{"remote_url":"https://example.invalid/a"}"#);

        let encoded =
            prepare(&message(&base64::encode(json)), Workload::Execution).expect("base64 envelope");
        assert_eq!(encoded.job_id, "job-1");
        assert_eq!(encoded.payload, raw.payload);
    }

    #[test]
    fn rejects_unsupported_envelope_version() {
        let json = r#"{"v":2,"job_id":"job-1","payload":{}}"#;
        assert_eq!(
            prepare(&message(json), Workload::Execution).unwrap_err(),
            "envelope_version_unsupported"
        );
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate_on_char_boundary("äöü", 3), "ä");
        assert_eq!(truncate_on_char_boundary("abc", 8), "abc");
    }
}
