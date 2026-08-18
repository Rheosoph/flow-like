#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_types::tokio;
use lambda_runtime::{Error, LambdaEvent, run, service_fn, tracing};
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::OnceLock;
use std::time::Duration;

// Defaults chosen so a slow API doesn't blow past the Lambda timeout and cause
// EventBridge Scheduler to retry, producing duplicate executions.
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HTTP_TIMEOUT_SECS: u64 = 120;
const SCHEDULER_CONTEXT_PLACEHOLDER_PREFIX: &str = "<aws.scheduler.";

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

fn get_http_client() -> Result<&'static reqwest::Client, Error> {
    if let Some(client) = HTTP_CLIENT.get() {
        return Ok(client);
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to build HTTP client");
            Error::from(format!("Failed to build HTTP client: {}", e))
        })?;

    let _ = HTTP_CLIENT.set(client);
    HTTP_CLIENT
        .get()
        .ok_or_else(|| Error::from("HTTP client failed to initialize"))
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
    #[serde(default)]
    scheduled_time: Option<String>,
    #[serde(default)]
    schedule_arn: Option<String>,
    #[serde(default)]
    execution_id: Option<String>,
    #[serde(default)]
    attempt_number: Option<String>,
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

fn scheduler_context_value(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !value.starts_with(SCHEDULER_CONTEXT_PLACEHOLDER_PREFIX))
}

fn build_idempotency_key(payload: &ScheduledEventPayload, request_id: &str) -> String {
    if let Some(scheduled_time) = scheduler_context_value(&payload.scheduled_time) {
        let schedule_id =
            scheduler_context_value(&payload.schedule_arn).unwrap_or(payload.event_id.as_str());
        return format!("aws-scheduler:{}:{}", schedule_id, scheduled_time);
    }

    // Backward compatible fallback for schedules created before the target
    // input included Scheduler context fields.
    format!("lambda-request:{}", request_id)
}

/// Split an EventBridge Scheduler ARN
/// (`arn:aws:scheduler:<region>:<account>:schedule/<group>/<name>`) into its
/// group and schedule name.
fn parse_schedule_arn(arn: &str) -> Option<(&str, &str)> {
    let resource = arn.splitn(6, ':').nth(5)?;
    let mut parts = resource.split('/');

    if parts.next()? != "schedule" {
        return None;
    }

    let group_name = parts.next().filter(|value| !value.is_empty())?;
    let schedule_name = parts.next().filter(|value| !value.is_empty())?;

    if parts.next().is_some() {
        return None;
    }

    Some((group_name, schedule_name))
}

/// Recognize the API's own "this event has no active sink" answer.
///
/// A bare 404 is not enough to act on: a misrouted or misconfigured
/// `API_BASE_URL` also answers 404, and reaping on that would delete every
/// schedule in the account. Only the API's structured NOT_FOUND naming this
/// event counts.
fn is_missing_sink_response(body: &str, event_id: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let Some(error) = parsed.get("error") else {
        return false;
    };

    if error.get("code").and_then(|code| code.as_str()) != Some("NOT_FOUND") {
        return false;
    }

    error
        .get("message")
        .and_then(|message| message.as_str())
        .is_some_and(|message| message.contains("No active sink") && message.contains(event_id))
}

/// Delete the schedule that just fired for an event the API no longer has a
/// sink for.
///
/// `EventSink` rows cascade away with their app or event while the schedule
/// itself lives in EventBridge, so an orphan keeps firing into a 404 forever.
/// The firing is the only signal that the orphan exists, so it is also the only
/// place it can reap itself.
async fn delete_orphaned_schedule(event_id: &str, schedule_arn: &str) {
    let Some((group_name, schedule_name)) = parse_schedule_arn(schedule_arn) else {
        tracing::warn!(
            event_id = %event_id,
            schedule_arn = %schedule_arn,
            "Could not parse schedule ARN; leaving orphaned schedule in place"
        );
        return;
    };

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_scheduler::Client::new(&aws_config);

    match client
        .delete_schedule()
        .group_name(group_name)
        .name(schedule_name)
        // Fresh token per attempt so a retry after a transient failure can
        // actually re-delete rather than replay a cached success.
        .client_token(flow_like_types::create_id())
        .send()
        .await
    {
        Ok(_) => tracing::warn!(
            event_id = %event_id,
            group_name = %group_name,
            schedule_name = %schedule_name,
            "Deleted orphaned schedule; the API has no active sink for this event"
        ),
        Err(e) => tracing::error!(
            event_id = %event_id,
            group_name = %group_name,
            schedule_name = %schedule_name,
            error = %e,
            "Failed to delete orphaned schedule"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with(event_id: &str, scheduled_time: Option<&str>) -> ScheduledEventPayload {
        ScheduledEventPayload {
            event_id: event_id.to_string(),
            scheduled_time: scheduled_time.map(str::to_string),
            schedule_arn: Some(
                "arn:aws:scheduler:eu-west-1:123456789012:schedule/flow-like/test".to_string(),
            ),
            execution_id: None,
            attempt_number: None,
        }
    }

    #[test]
    fn idempotency_key_uses_schedule_and_scheduled_time() {
        let payload = payload_with("event-1", Some("2026-04-27T06:00:00Z"));

        assert_eq!(
            build_idempotency_key(&payload, "request-1"),
            "aws-scheduler:arn:aws:scheduler:eu-west-1:123456789012:schedule/flow-like/test:2026-04-27T06:00:00Z"
        );
    }

    #[test]
    fn idempotency_key_falls_back_for_old_schedule_payloads() {
        let payload = payload_with("event-1", None);

        assert_eq!(
            build_idempotency_key(&payload, "request-1"),
            "lambda-request:request-1"
        );
    }

    #[test]
    fn idempotency_key_ignores_unexpanded_scheduler_placeholders() {
        let payload = payload_with("event-1", Some("<aws.scheduler.scheduled-time>"));

        assert_eq!(
            build_idempotency_key(&payload, "request-1"),
            "lambda-request:request-1"
        );
    }

    #[test]
    fn parses_group_and_name_from_schedule_arn() {
        assert_eq!(
            parse_schedule_arn(
                "arn:aws:scheduler:eu-west-1:725302850608:schedule/flow-like-dev/flow-like-cron-abc"
            ),
            Some(("flow-like-dev", "flow-like-cron-abc"))
        );
    }

    #[test]
    fn rejects_arns_that_are_not_schedules() {
        assert_eq!(
            parse_schedule_arn("arn:aws:lambda:eu-west-1:725302850608:function:some-fn"),
            None
        );
        assert_eq!(
            parse_schedule_arn("arn:aws:scheduler:eu-west-1:725302850608:schedule/only-group"),
            None
        );
        assert_eq!(parse_schedule_arn("not-an-arn"), None);
    }

    #[test]
    fn detects_missing_sink_response() {
        let body =
            r#"{"error":{"code":"NOT_FOUND","message":"No active sink found for event evt-1"}}"#;

        assert!(is_missing_sink_response(body, "evt-1"));
    }

    #[test]
    fn ignores_unrelated_not_found_responses() {
        // A misrouted API_BASE_URL answers 404 too; reaping on that would delete
        // every schedule in the account.
        assert!(!is_missing_sink_response(
            "<html>404 Not Found</html>",
            "evt-1"
        ));
        assert!(!is_missing_sink_response(
            r#"{"error":{"code":"NOT_FOUND","message":"Route not found"}}"#,
            "evt-1"
        ));
        // Right shape, but about a different event.
        assert!(!is_missing_sink_response(
            r#"{"error":{"code":"NOT_FOUND","message":"No active sink found for event evt-2"}}"#,
            "evt-1"
        ));
    }
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
    let idempotency_key = build_idempotency_key(&payload, &event.context.request_id);
    let scheduled_time = scheduler_context_value(&payload.scheduled_time);
    let schedule_arn = scheduler_context_value(&payload.schedule_arn);
    let execution_id = scheduler_context_value(&payload.execution_id);
    let attempt_number = scheduler_context_value(&payload.attempt_number);

    tracing::info!(
        event_id = %payload.event_id,
        idempotency_key = %idempotency_key,
        scheduled_time = ?scheduled_time,
        schedule_arn = ?schedule_arn,
        execution_id = ?execution_id,
        attempt_number = ?attempt_number,
        "Processing scheduled event"
    );

    let client = get_http_client()?;
    let trigger_url = format!(
        "{}/api/v1/sink/trigger/async",
        api_base_url.trim_end_matches('/')
    );

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

        // Most 4xx responses are hard failures. Retry only transient client
        // failures where the same scheduled firing can plausibly succeed later.
        if status.is_client_error() && !is_retryable_status(status) {
            tracing::error!(
                status = %status,
                body = %body,
                event_id = %payload.event_id,
                "API returned client error — not retrying"
            );

            if status == StatusCode::NOT_FOUND && is_missing_sink_response(&body, &payload.event_id)
            {
                match schedule_arn {
                    Some(arn) => delete_orphaned_schedule(&payload.event_id, arn).await,
                    // Schedules created before the target input carried Scheduler
                    // context cannot be identified, so they are left alone.
                    None => tracing::warn!(
                        event_id = %payload.event_id,
                        "Sink is gone but the payload carries no schedule ARN; cannot reap this schedule"
                    ),
                }
            }

            return Ok(());
        }

        tracing::error!(status = %status, body = %body, "API returned retryable error");
        return Err(Error::from(format!("API error: {} - {}", status, body)));
    }

    if response.status() == StatusCode::NO_CONTENT {
        tracing::info!(event_id = %payload.event_id, "API returned 204; treating trigger as successful");
        return Ok(());
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let response_body = response.text().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to read trigger response");
        Error::from(format!("Failed to read trigger response: {}", e))
    })?;
    let trimmed_body = response_body.trim();

    if trimmed_body.is_empty() {
        tracing::info!(event_id = %payload.event_id, "API returned empty success body; treating trigger as successful");
        return Ok(());
    }

    let looks_like_json = content_type
        .as_deref()
        .is_some_and(|value| value.to_ascii_lowercase().contains("json"))
        || trimmed_body.starts_with('{');
    if !looks_like_json {
        tracing::warn!(
            event_id = %payload.event_id,
            content_type = ?content_type,
            body = %trimmed_body,
            "API returned non-JSON success body; treating trigger as successful"
        );
        return Ok(());
    }

    let trigger_response = serde_json::from_str::<TriggerResponse>(trimmed_body).map_err(|e| {
        tracing::error!(error = %e, body = %trimmed_body, "Failed to parse trigger response");
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
