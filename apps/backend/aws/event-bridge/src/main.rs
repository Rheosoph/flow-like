#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_types::tokio;
use lambda_runtime::{Error, LambdaEvent, run, service_fn, tracing};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::OnceLock;
use std::time::Duration;

// Defaults chosen so a slow API doesn't blow past the Lambda timeout and cause
// EventBridge Scheduler to retry, producing duplicate executions.
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 120;

static SINK_JWT: OnceLock<String> = OnceLock::new();
static API_BASE_URL: OnceLock<String> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn is_retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .expect("Failed to build reqwest client")
    })
}

fn get_sink_jwt() -> Result<&'static str, Error> {
    if let Some(value) = SINK_JWT.get() {
        return Ok(value.as_str());
    }

    let value = env::var("SINK_JWT").map_err(|_| Error::from("SINK_JWT not set"))?;
    let _ = SINK_JWT.set(value);
    Ok(SINK_JWT
        .get()
        .expect("SINK_JWT value must be initialized")
        .as_str())
}

fn get_api_base_url() -> Result<&'static str, Error> {
    if let Some(value) = API_BASE_URL.get() {
        return Ok(value.as_str());
    }

    let value = env::var("API_BASE_URL").map_err(|_| Error::from("API_BASE_URL not set"))?;
    let _ = API_BASE_URL.set(value);
    Ok(API_BASE_URL
        .get()
        .expect("API_BASE_URL value must be initialized")
        .as_str())
}

/// Payload delivered by EventBridge Scheduler. Scheduler passes the `Input`
/// string verbatim as the Lambda event body (unlike EventBridge *Bus* rules,
/// which wrap the payload in a `detail` field).
#[derive(Debug, Deserialize, Serialize)]
struct ScheduledEventPayload {
    event_id: String,
}

#[derive(Debug, Serialize)]
struct TriggerRequest {
    event_id: String,
    sink_type: String,
}

#[derive(Debug, Deserialize)]
struct TriggerResponse {
    success: bool,
    run_id: Option<String>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::init_default_subscriber();
    run(service_fn(event_bridge_handler)).await
}

async fn event_bridge_handler(event: LambdaEvent<ScheduledEventPayload>) -> Result<(), Error> {
    let api_base_url = get_api_base_url()?;
    let sink_jwt = get_sink_jwt()?;

    let payload = event.payload;
    // Lambda's per-invocation request id is the natural idempotency key:
    // retries of the same logical invocation share it, retries originating
    // from EventBridge Scheduler get a fresh one (which is what we want — the
    // API should run once per scheduled firing).
    let idempotency_key = event.context.request_id.clone();

    tracing::info!(
        event_id = %payload.event_id,
        idempotency_key = %idempotency_key,
        "Processing scheduled event"
    );

    let client = get_http_client();
    let trigger_url = format!("{}/api/v1/sink/trigger/async", api_base_url);

    let request_body = TriggerRequest {
        event_id: payload.event_id.clone(),
        sink_type: "cron".to_string(),
    };

    let response = client
        .post(&trigger_url)
        .header("Authorization", format!("Bearer {}", sink_jwt))
        .header("Idempotency-Key", &idempotency_key)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to send trigger request");
            Error::from(format!("HTTP request failed: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("<failed to read response body: {}>", e));

        // Most 4xx responses are hard failures. Keep retrying transient client
        // failures such as 408/429 so EventBridge Scheduler can back off.
        if status.is_client_error() && !is_retryable_status(status) {
            tracing::error!(
                status = %status,
                body = %body,
                event_id = %payload.event_id,
                "API returned client error — not retrying"
            );
            return Ok(());
        }

        tracing::error!(status = %status, body = %body, "API returned retryable error");
        return Err(Error::from(format!("API error: {} - {}", status, body)));
    }

    let trigger_response = response.json::<TriggerResponse>().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to parse trigger response");
        Error::from(format!("Failed to parse trigger response: {}", e))
    })?;

    if !trigger_response.success {
        let error = trigger_response
            .error
            .unwrap_or_else(|| "unknown trigger error".to_string());
        tracing::error!(
            event_id = %payload.event_id,
            run_id = ?trigger_response.run_id,
            error = %error,
            "API accepted request but did not trigger event"
        );
        return Err(Error::from(format!("Trigger failed: {}", error)));
    }

    tracing::info!(
        event_id = %payload.event_id,
        run_id = ?trigger_response.run_id,
        "Successfully triggered event"
    );
    Ok(())
}
