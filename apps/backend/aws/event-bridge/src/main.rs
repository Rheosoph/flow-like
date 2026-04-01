#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use flow_like_types::tokio;
use lambda_runtime::{Error, LambdaEvent, run, service_fn, tracing};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::OnceLock;

static SINK_JWT: OnceLock<String> = OnceLock::new();
static API_BASE_URL: OnceLock<String> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(reqwest::Client::new)
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

#[derive(Debug, Deserialize, Serialize)]
struct EventDetail {
    event_id: String,
}

#[derive(Debug, Serialize)]
struct TriggerRequest {
    event_id: String,
    sink_type: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::init_default_subscriber();
    run(service_fn(event_bridge_handler)).await
}

async fn event_bridge_handler(event: LambdaEvent<EventDetail>) -> Result<(), Error> {
    let api_base_url = get_api_base_url()?;
    let sink_jwt = get_sink_jwt()?;

    let detail = event.payload;

    tracing::info!(event_id = %detail.event_id, "Processing scheduled event");

    let client = get_http_client();
    let trigger_url = format!("{}/api/v1/sink/trigger/async", api_base_url);

    let request_body = TriggerRequest {
        event_id: detail.event_id.clone(),
        sink_type: "cron".to_string(),
    };

    let response = client
        .post(&trigger_url)
        .header("Authorization", format!("Bearer {}", sink_jwt))
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
        tracing::error!(status = %status, body = %body, "API returned error");
        return Err(Error::from(format!("API error: {} - {}", status, body)));
    }

    tracing::info!(event_id = %detail.event_id, "Successfully triggered event");
    Ok(())
}
