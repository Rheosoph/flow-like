use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Value;
use crate::sync::Mutex;

/// Configuration for a single choice option
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChoiceOption {
    /// Unique identifier for this option
    pub id: String,
    /// Display label
    pub label: String,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option allows freeform text input
    #[serde(default)]
    pub freeform: bool,
}

/// Configuration for a form field
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FormField {
    /// Unique identifier for this field
    pub id: String,
    /// Display label
    pub label: String,
    /// Optional description/help text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Field type (text, number, boolean, select)
    pub field_type: FormFieldType,
    /// Whether the field is required
    #[serde(default)]
    pub required: bool,
    /// Default value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    /// Options for select fields
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
}

/// Type of form field
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldType {
    Text,
    Number,
    Boolean,
    Select,
}

/// Type of interaction
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionType {
    SingleChoice {
        options: Vec<ChoiceOption>,
        #[serde(default)]
        allow_freeform: bool,
    },
    MultipleChoice {
        options: Vec<ChoiceOption>,
        #[serde(default)]
        min_selections: usize,
        #[serde(default = "default_max_selections")]
        max_selections: usize,
    },
    Form {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FormField>,
    },
}

fn default_max_selections() -> usize {
    usize::MAX
}

/// Status of an interaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    Pending,
    Responded,
    Expired,
    Cancelled,
}

/// A human-in-the-loop interaction request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionRequest {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Description/prompt shown to the user
    pub description: String,
    /// The type and configuration of this interaction
    pub interaction_type: InteractionType,
    /// Current status
    pub status: InteractionStatus,
    /// TTL in seconds from creation
    pub ttl_seconds: u64,
    /// Unix timestamp when this expires
    pub expires_at: u64,
    /// Run ID that created this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// App ID context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// JWT for responding to this interaction (present in remote execution)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responder_jwt: Option<String>,
}

/// Response to an interaction
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InteractionResponse {
    /// The interaction ID this responds to
    pub interaction_id: String,
    /// The response value (interpretation depends on interaction type)
    pub value: Value,
}

/// Result of polling for an interaction response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InteractionPollResult {
    Pending,
    Responded { value: Value },
    Expired,
    Cancelled,
}

static INTERACTION_REQUESTS: LazyLock<Arc<Mutex<HashMap<String, InteractionRequest>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

static INTERACTION_RESPONSES: LazyLock<Arc<Mutex<HashMap<String, Value>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

pub async fn register_interaction(request: InteractionRequest) {
    let mut store = INTERACTION_REQUESTS.lock().await;
    store.insert(request.id.clone(), request);
}

pub async fn submit_interaction_response(interaction_id: &str, value: Value) -> bool {
    let mut responses = INTERACTION_RESPONSES.lock().await;
    if responses.contains_key(interaction_id) {
        return false;
    }
    responses.insert(interaction_id.to_string(), value);
    drop(responses);

    let mut requests = INTERACTION_REQUESTS.lock().await;
    if let Some(req) = requests.get_mut(interaction_id) {
        req.status = InteractionStatus::Responded;
    }
    true
}

pub async fn poll_interaction_response(interaction_id: &str) -> InteractionPollResult {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let responses = INTERACTION_RESPONSES.lock().await;
    if let Some(value) = responses.get(interaction_id) {
        return InteractionPollResult::Responded {
            value: value.clone(),
        };
    }
    drop(responses);

    let requests = INTERACTION_REQUESTS.lock().await;
    if let Some(req) = requests.get(interaction_id) {
        if now >= req.expires_at {
            return InteractionPollResult::Expired;
        }
        if req.status == InteractionStatus::Cancelled {
            return InteractionPollResult::Cancelled;
        }
        return InteractionPollResult::Pending;
    }

    InteractionPollResult::Expired
}

pub async fn cleanup_expired_interactions() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut requests = INTERACTION_REQUESTS.lock().await;
    let expired_ids: Vec<String> = requests
        .iter()
        .filter(|(_, req)| now >= req.expires_at)
        .map(|(id, _)| id.clone())
        .collect();

    for id in &expired_ids {
        requests.remove(id);
    }
    drop(requests);

    let mut responses = INTERACTION_RESPONSES.lock().await;
    for id in &expired_ids {
        responses.remove(id);
    }
}

/// SSE event payload for interaction creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseCreatedPayload {
    pub id: String,
    pub responder_jwt: String,
    pub expires_at: i64,
}

/// SSE event payload for interaction response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseRespondedPayload {
    pub value: Value,
}

/// Result of awaiting a remote interaction
#[derive(Debug, Clone)]
pub struct RemoteInteractionResult {
    /// Whether the user responded (vs timeout/expired)
    pub responded: bool,
    /// The response value (empty if not responded)
    pub value: Value,
    /// The responder JWT for the interaction
    pub responder_jwt: String,
}

/// Parameters for creating a remote interaction
#[derive(Debug, Clone)]
pub struct RemoteInteractionParams<'a> {
    pub hub_url: &'a str,
    pub token: &'a str,
    pub app_id: &'a str,
    pub ttl_seconds: u64,
    pub request: InteractionRequest,
}

/// Create an interaction remotely and wait for the response.
/// This version allows streaming the interaction request with responder_jwt before waiting.
///
/// Returns the interaction result or an error with details about what failed.
pub async fn create_remote_interaction_stream<F>(
    params: RemoteInteractionParams<'_>,
    on_created: F,
) -> crate::Result<RemoteInteractionResult>
where
    F: FnOnce(
        InteractionRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
{
    use futures::StreamExt;
    use reqwest_eventsource::{Event, EventSource};

    let base_url = params.hub_url.trim_end_matches('/');
    let url = format!("{}/api/v1/interaction", base_url);

    let client = reqwest::Client::new();
    let request = client
        .post(&url)
        .bearer_auth(params.token)
        .json(&serde_json::json!({
            "app_id": params.app_id,
            "ttl_seconds": params.ttl_seconds
        }));

    let mut es = EventSource::new(request)
        .map_err(|e| crate::anyhow!("Failed to create SSE connection to {}: {}", url, e))?;

    let mut responder_jwt = String::new();
    let mut on_created = Some(on_created);

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {}
            Ok(Event::Message(message)) => match message.event.as_str() {
                "created" => {
                    if let Ok(payload) = serde_json::from_str::<SseCreatedPayload>(&message.data) {
                        responder_jwt = payload.responder_jwt.clone();

                        // Update the request with the responder JWT and stream it
                        if let Some(callback) = on_created.take() {
                            let mut request_with_jwt = params.request.clone();
                            request_with_jwt.id = payload.id;
                            request_with_jwt.responder_jwt = Some(payload.responder_jwt);
                            request_with_jwt.expires_at = payload.expires_at as u64;
                            callback(request_with_jwt).await;
                        }
                    }
                }
                "responded" => {
                    es.close();
                    if let Ok(payload) = serde_json::from_str::<SseRespondedPayload>(&message.data)
                    {
                        return Ok(RemoteInteractionResult {
                            responded: true,
                            value: payload.value,
                            responder_jwt,
                        });
                    }
                    return Ok(RemoteInteractionResult {
                        responded: true,
                        value: Value::Null,
                        responder_jwt,
                    });
                }
                "expired" => {
                    es.close();
                    return Ok(RemoteInteractionResult {
                        responded: false,
                        value: Value::Null,
                        responder_jwt,
                    });
                }
                _ => {}
            },
            Err(e) => {
                es.close();
                return Err(crate::anyhow!("SSE stream error: {}", e));
            }
        }
    }

    Ok(RemoteInteractionResult {
        responded: false,
        value: Value::Null,
        responder_jwt,
    })
}
