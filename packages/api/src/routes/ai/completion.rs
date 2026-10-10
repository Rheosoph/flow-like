use std::time::{Duration, Instant};

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::{MethodRouter, post},
};
use flow_like::{
    bit::BitModelPreference,
    flow_like_model_provider::{
        history::{Content, History, HistoryMessage, HistoryThinking, MessageContent, Role},
        provider::is_hosted_provider_name,
        response::Response,
    },
    models::llm::ModelUsageContext,
    state::FlowLikeState,
};
use serde::Deserialize;

use super::{
    copilot::{master_flow_like_state, user_access_token},
    global_chat::{
        ensure_metered_or_customer_provider, load_user_profile_access,
        reserve_assistant_usage_with_max_runtime, settle_assistant_usage,
    },
};
use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};

const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_MESSAGES: usize = 16;
const MAX_TEXT_CHARS: usize = 40_000;
const MAX_COMPLETION_TOKENS: u32 = 256;
const MAX_RUNTIME: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionRequest {
    profile_id: String,
    app_id: Option<String>,
    messages: Vec<HistoryMessage>,
}

pub fn routes() -> Router<AppState> {
    completion_routes(post(complete))
}

fn completion_routes<S: Clone + Send + Sync + 'static>(handler: MethodRouter<S>) -> Router<S> {
    Router::new()
        .route("/completion", handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .route_layer(axum::middleware::from_fn(require_interactive_session))
}

async fn require_interactive_session(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match request.extensions().get::<AppUser>() {
        Some(AppUser::OpenID(_)) => next.run(request).await,
        None | Some(AppUser::Unauthorized) => ApiError::UNAUTHORIZED.into_response(),
        Some(_) => ApiError::bad_request("Inline completion requires an interactive session")
            .into_response(),
    }
}

fn validate(payload: &CompletionRequest) -> Result<(), ApiError> {
    if payload.profile_id.trim().is_empty() || payload.profile_id.len() > 256 {
        return Err(ApiError::bad_request("An active profile is required"));
    }
    if payload
        .app_id
        .as_ref()
        .is_some_and(|id| id.trim().is_empty() || id.len() > 256)
    {
        return Err(ApiError::bad_request("Invalid completion app ID"));
    }
    if payload.messages.is_empty() || payload.messages.len() > MAX_MESSAGES {
        return Err(ApiError::bad_request("Invalid completion message count"));
    }
    let mut text_chars = 0;
    for message in &payload.messages {
        if !matches!(message.role, Role::System | Role::User | Role::Assistant)
            || message.tool_calls.is_some()
            || message.tool_call_id.is_some()
        {
            return Err(ApiError::bad_request(
                "Inline completion accepts text messages only",
            ));
        }
        match &message.content {
            MessageContent::String(text) => text_chars += text.chars().count(),
            MessageContent::Contents(parts) => {
                for part in parts {
                    match part {
                        Content::Text { text, .. } => text_chars += text.chars().count(),
                        _ => {
                            return Err(ApiError::bad_request(
                                "Inline completion accepts text messages only",
                            ));
                        }
                    }
                }
            }
        }
        if text_chars > MAX_TEXT_CHARS {
            return Err(ApiError::bad_request("Completion context is too long"));
        }
    }
    Ok(())
}

fn completion_preferences() -> BitModelPreference {
    BitModelPreference {
        cost_weight: Some(1.0),
        ..Default::default()
    }
}

async fn complete(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<CompletionRequest>,
) -> Result<Json<Response>, ApiError> {
    validate(&payload)?;
    let sub = user.sub()?;
    let token = user_access_token(&user).ok_or(ApiError::UNAUTHORIZED)?;
    if let Some(app_id) = payload.app_id.as_deref() {
        user.execution_app_permission(app_id, &state).await?;
    }
    let (profile, access) = load_user_profile_access(&state, &sub, Some(&payload.profile_id))
        .await?
        .ok_or_else(|| ApiError::bad_request("Active profile was not found"))?;
    if let Some(rejection) = access.rejection(None) {
        return Err(rejection);
    }
    let flow_like_state = master_flow_like_state(&state).await?;
    let selected = profile
        .resolve_completion_model(
            None,
            &completion_preferences(),
            false,
            FlowLikeState::completion_model_capabilities(&flow_like_state).await,
            None,
            flow_like_state.http_client.clone(),
        )
        .await?;
    let provider = selected
        .try_to_provider()
        .ok_or_else(|| ApiError::bad_request("Selected Bit is not a completion provider"))?;
    ensure_metered_or_customer_provider(&provider.provider_name)?;
    let funding = if is_hosted_provider_name(&provider.provider_name) {
        "hosted"
    } else {
        "byok"
    };
    let usage = reserve_assistant_usage_with_max_runtime(
        &state,
        &sub,
        &selected.id,
        &provider.provider_name,
        funding,
        payload.app_id.as_deref(),
        MAX_RUNTIME.as_millis() as i64,
        false,
    )
    .await?;
    if !crate::quota::mark_started(&state, &usage.operation_id).await? {
        return Err(ApiError::conflict(
            "Completion operation has already started",
        ));
    }

    // Keep quota settlement in the model task when the request future is dropped.
    flow_like_types::tokio::spawn(async move {
        let started = Instant::now();
        let remaining = (usage.deadline - chrono::Utc::now())
            .num_milliseconds()
            .max(1) as u64;
        let result = flow_like_types::tokio::time::timeout(
            MAX_RUNTIME.min(Duration::from_millis(remaining)),
            async {
                let model = flow_like_state
                    .model_factory
                    .build(
                        &selected,
                        flow_like_state.clone(),
                        Some(token),
                        Some(ModelUsageContext {
                            app_id: payload.app_id,
                            ..Default::default()
                        }),
                    )
                    .await?;
                let mut history = History::new(selected.id, payload.messages);
                history.set_stream(false);
                history.max_completion_tokens = Some(MAX_COMPLETION_TOKENS);
                history.thinking = Some(HistoryThinking::Off);
                model.invoke(&history, None).await
            },
        )
        .await
        .map_err(|_| ApiError::service_unavailable("Inline completion timed out"))
        .and_then(|result| result.map_err(ApiError::from));
        settle_assistant_usage(
            &state,
            &usage,
            started.elapsed().as_millis() as i64,
            result.is_ok(),
        )
        .await?;
        result.map(Json)
    })
    .await
    .map_err(|error| ApiError::internal(format!("Inline completion task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use flow_like::{
        bit::{Bit, BitTypes},
        profile::{Profile, ProfileCustomBit},
        state::CompletionModelCapabilities,
        utils::http::HTTPClient,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn request() -> CompletionRequest {
        serde_json::from_value(json!({
            "profile_id": "active-profile",
            "messages": [{"role": "user", "content": "Please continue"}]
        }))
        .unwrap()
    }

    #[test]
    fn requires_profile_and_does_not_accept_model_overrides() {
        assert!(
            serde_json::from_value::<CompletionRequest>(json!({
                "messages": [{"role": "user", "content": "Continue"}]
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CompletionRequest>(json!({
                "profile_id": "active-profile", "messages": [], "model_id": "expensive"
            }))
            .is_err()
        );
        let mut payload = request();
        payload.profile_id = " ".into();
        assert!(validate(&payload).is_err());
    }

    #[test]
    fn accepts_editor_text_and_rejects_excessive_context_or_tool_messages() {
        let mut payload = request();
        assert!(validate(&payload).is_ok());
        payload.messages[0] = HistoryMessage::from_string(Role::User, "A text part");
        assert!(validate(&payload).is_ok());
        payload.messages[0].role = Role::Tool;
        assert!(validate(&payload).is_err());
        payload.messages[0] =
            HistoryMessage::from_string(Role::User, &"x".repeat(MAX_TEXT_CHARS + 1));
        assert!(validate(&payload).is_err());
        payload.messages = vec![HistoryMessage::from_string(Role::User, "x"); MAX_MESSAGES + 1];
        assert!(validate(&payload).is_err());
    }

    #[tokio::test]
    async fn authentication_precedes_completion_body_parsing() {
        async fn handler(Json(_): Json<CompletionRequest>) -> StatusCode {
            StatusCode::NO_CONTENT
        }
        let response = completion_routes(post(handler))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/completion")
                    .header("content-type", "application/json")
                    .body(Body::from("invalid JSON"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    fn bit(id: &str, provider: &str, cost: f32) -> ProfileCustomBit {
        ProfileCustomBit(Bit {
            id: id.into(),
            bit_type: BitTypes::Llm,
            parameters: json!({
                "context_length": 32_000,
                "model_classification": {
                    "cost": cost, "speed": 0.5, "reasoning": 0.5, "creativity": 0.5,
                    "factuality": 0.5, "function_calling": 0.5, "safety": 0.5,
                    "openness": 0.5, "multilinguality": 0.5, "coding": 0.5
                },
                "provider": {"provider_name": provider, "model_id": id}
            }),
            ..Bit::default()
        })
    }

    #[tokio::test]
    async fn selects_cheapest_executable_bit_only_from_the_active_profile() {
        for provider in ["hosted:openai", "custom:openai"] {
            let profile = Profile {
                bits: vec!["expensive".into(), "cheap".into(), "local".into()],
                custom_bits: vec![
                    bit("expensive", "hosted:openai", 0.2),
                    bit("cheap", provider, 0.8),
                    bit("inactive", "hosted:openai", 1.0),
                    bit("local", "Local", 1.0),
                ],
                ..Profile::default()
            };
            let selected = profile
                .resolve_completion_model(
                    None,
                    &completion_preferences(),
                    false,
                    CompletionModelCapabilities::default(),
                    None,
                    Arc::new(HTTPClient::new_without_refetch()),
                )
                .await
                .unwrap();
            assert_eq!(selected.id, "cheap");
        }
    }
}
