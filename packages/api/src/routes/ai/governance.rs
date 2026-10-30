//! Bridge between the API and the core [`GovernanceCopilot`]. Loads an app's
//! boards, renders them to FlowScript and runs the read-only governance agent
//! to propose EU AI Act questionnaire answers. Shared by the owner
//! (`/apps/{app_id}/ai-act/assessment/suggest`) and admin
//! (`/admin/ai-act/assist`) surfaces.

use std::{convert::Infallible, future::Future, sync::Arc, time::Duration};

use axum::{
    Json,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};

use flow_like::bit::Bit;
use flow_like::copilot::governance::GovernanceCopilot;
use flow_like::models::llm::ModelUsageContext;
use flow_like::profile::{Profile, ProfileCustomBit};
use flow_like::state::FlowLikeState;

use crate::{
    error::ApiError,
    middleware::error_reporting::StreamErrorReporter,
    routes::{
        app::ai_act::{SuggestResponse, board_scan, questionnaire::questionnaire_schema},
        execution::models::{provider_of, scrub, secret_values},
    },
    state::AppState,
};

/// Send headers and heartbeats while the agent runs so a gateway's idle timeout
/// does not cut off slow model responses. Callers authorize before opening the stream.
pub async fn governance_response<F>(
    streaming: bool,
    reporter: Option<StreamErrorReporter>,
    run: F,
) -> Result<Response, ApiError>
where
    F: Future<Output = Result<SuggestResponse, ApiError>> + Send + 'static,
{
    if !streaming {
        return Ok(Json(run.await?).into_response());
    }

    let stream = async_stream::stream! {
        yield Ok::<Event, Infallible>(Event::default().comment("connected"));
        // Keep the work in the body future so disconnects and the route deadline cancel it.
        let result = run.await.and_then(|response| {
            Event::default()
                .event("completed")
                .json_data(response)
                .map_err(|error| ApiError::internal(error.to_string()))
        });
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                if let Some(reporter) = &reporter {
                    reporter.report(&error).await;
                }
                let message = error.public_message().unwrap_or_else(|| {
                    error.status().canonical_reason().unwrap_or("Governance suggestion failed")
                });
                Event::default().event("error").data(serde_json::json!({
                    "error": message,
                    "code": error.public_code(),
                }).to_string())
            }
        };
        yield Ok(event);
    };

    Ok((
        [("x-accel-buffering", "no")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .text("keep-alive")
                .interval(Duration::from_secs(15)),
        ),
    )
        .into_response())
}

/// Resolve (or build and cache) the master `FlowLikeState`.
async fn master_flow_like_state(state: &AppState) -> Result<Arc<FlowLikeState>, ApiError> {
    if let Some(flow_like_state) = state.state_cache.get("master") {
        return Ok(flow_like_state);
    }
    let credentials = state.master_credentials().await?;
    let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
    state
        .state_cache
        .insert("master".to_string(), flow_like_state.clone());
    Ok(flow_like_state)
}

/// Run the governance agent for an app and return its suggestion, the signals
/// it was grounded on, and the resolved model name that produced it.
///
/// `profile` is the caller's profile used to resolve which model bits to use; when the request omits
/// it we load the caller's synced profile from the DB (like the global chat) so hosted Bits — and
/// their metering — still resolve. `token` is the caller's OpenID access token: hosted Bit models
/// post to this server's metered `/chat/completions` with it, so without it those models 401. With
/// neither a profile nor a token the copilot falls back to an unmetered platform default that may
/// fail if that provider is not configured.
pub async fn run_governance_agent(
    state: &AppState,
    sub: &str,
    app_id: &str,
    model_id: Option<String>,
    profile: Option<Profile>,
    token: Option<String>,
    usage_context: Option<ModelUsageContext>,
) -> Result<SuggestResponse, ApiError> {
    let app = state.master_app(sub, app_id, state).await?;

    // Auto-derived signals for grounding/context.
    let signals = board_scan::scan_app_signals(state, sub, app_id, &app)
        .await
        .unwrap_or_default();

    // Load and depict the boards as FlowScript.
    let mut boards = Vec::new();
    for board_id in &app.boards {
        match state.master_board(sub, app_id, board_id, state, None).await {
            Ok(board) => boards.push(board),
            Err(err) => {
                tracing::warn!(board_id, error = %err, "governance agent: skipping unloadable board");
            }
        }
    }
    let depictions = GovernanceCopilot::depict_boards(&boards);

    // Context = signals + questionnaire schema so the model has canonical keys.
    let context = serde_json::json!({
        "signals": signals,
        "questionnaire": questionnaire_schema(),
    });
    let context_json = serde_json::to_string(&context).unwrap_or_default();

    let flow_like_state = master_flow_like_state(state).await?;

    // Prefer the request's profile; otherwise fall back to the caller's synced DB profile so hosted
    // Bit models (and their metering) still resolve — matching the global chat endpoint.
    let profile = match profile {
        Some(profile) => Some(Arc::new(with_private_bits(
            profile,
            caller_private_bits(state, sub).await,
        ))),
        None => crate::routes::ai::global_chat::load_user_profile_opt(state, sub, None).await?,
    };
    let secrets = private_bit_secrets(profile.as_deref());
    let copilot = GovernanceCopilot::new(flow_like_state, profile, usage_context);

    let suggestion = copilot
        .assist(&depictions, &context_json, model_id, token)
        .await
        .map_err(|error| ApiError::internal(agent_failure(&error, &secrets)))?;

    let (suggestion, model) = suggestion;

    Ok(SuggestResponse {
        suggestion: serde_json::to_value(suggestion)?,
        signals: serde_json::to_value(signals)?,
        model,
    })
}

/// The caller's private Bits, with their keys, from the database, as global chat loads them.
async fn caller_private_bits(state: &AppState, sub: &str) -> Vec<Bit> {
    crate::routes::user::bits::load_custom_bits_for_user(state, sub, true)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                sub,
                "Failed to load custom bits for the governance agent: {error:?}"
            );
            vec![]
        })
}

/// A client-supplied profile keeps its public Bits and hubs, but its private Bits always come
/// from the caller's own library. A hub the profile names can still serve a `custom:*` Bit that
/// copies another user's id and revision, as it can through a synced profile; the runtime's
/// model cache keeps that Bit apart by keying custom clients on their parameters and egress too.
fn with_private_bits(mut profile: Profile, library: Vec<Bit>) -> Profile {
    profile.custom_bits = library.into_iter().map(ProfileCustomBit).collect();
    profile
}

/// The decrypted credentials of the profile's private Bits, longest first so that no value
/// survives inside another.
fn private_bit_secrets(profile: Option<&Profile>) -> Vec<String> {
    let mut secrets: Vec<String> = profile
        .into_iter()
        .flat_map(|profile| &profile.custom_bits)
        .filter_map(|bit| provider_of(&bit.0.parameters))
        .flat_map(|provider| secret_values(provider.params.as_ref()))
        .collect();
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    secrets
}

/// A failed run, as the API log and the error report keep it: Gemini sends its key in the
/// request URL and reqwest errors quote that URL, so the keys and every URL are removed.
fn agent_failure(error: &flow_like_types::Error, secrets: &[String]) -> String {
    format!(
        "Governance agent failed: {}",
        scrub(&format!("{error:#}"), secrets)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use flow_like_types::tokio::{sync::oneshot, time::timeout};
    use futures::StreamExt;
    use serde_json::json;

    fn suggestion() -> SuggestResponse {
        SuggestResponse {
            suggestion: json!({"purpose": "Review invoices", "suggestedAnswers": []}),
            signals: json!({"models": []}),
            model: "test-model".to_owned(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_suggestions_send_headers_and_heartbeats_before_the_result() {
        let (finish, result) = oneshot::channel();
        let response = governance_response(true, None, async { result.await.unwrap() })
            .await
            .unwrap();
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        let mut body = response.into_body().into_data_stream();
        let first = timeout(Duration::from_secs(1), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first.as_ref(), b": connected\n\n");

        // The model stays pending beyond the common 60 s gateway idle timeout.
        for _ in 0..5 {
            let heartbeat = timeout(Duration::from_secs(16), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(heartbeat.as_ref(), b": keep-alive\n\n");
        }

        finish.send(Ok(suggestion())).unwrap();
        let completed = body.next().await.unwrap().unwrap();
        let completed = std::str::from_utf8(&completed).unwrap();
        assert!(completed.starts_with("event: completed\n"));
        let payload = completed.split_once("data: ").unwrap().1.trim();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(payload).unwrap(),
            serde_json::to_value(suggestion()).unwrap(),
        );
        assert!(body.next().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_response_cancels_the_pending_agent() {
        let (finish, result) = oneshot::channel::<Result<SuggestResponse, ApiError>>();
        let response = governance_response(true, None, async { result.await.unwrap() })
            .await
            .unwrap();
        let mut body = response.into_body().into_data_stream();
        assert!(body.next().await.is_some());
        // Receiving a heartbeat means the agent future has been polled and is pending.
        let heartbeat = body.next().await.unwrap().unwrap();
        assert_eq!(heartbeat.as_ref(), b": keep-alive\n\n");
        drop(body);
        assert!(finish.is_closed());
    }

    #[tokio::test]
    async fn streaming_errors_expose_only_the_public_message() {
        for (error, expected) in [
            (
                ApiError::internal("private provider details"),
                json!({"error": "Internal Server Error", "code": "INTERNAL_ERROR"}),
            ),
            (
                ApiError::bad_request("Model unavailable"),
                json!({"error": "Model unavailable", "code": "BAD_REQUEST"}),
            ),
        ] {
            let response = governance_response(true, None, async { Err(error) })
                .await
                .unwrap();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body = std::str::from_utf8(&bytes).unwrap();
            assert!(body.contains("event: error\n"));
            assert!(!body.contains("private provider details"));
            assert!(!body.contains("event: completed"));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    body.split_once("data: ").unwrap().1.trim()
                )
                .unwrap(),
                expected,
            );
        }
    }

    #[tokio::test]
    async fn json_clients_keep_the_existing_response_and_error_status() {
        let owner: crate::routes::app::ai_act::SuggestBody =
            serde_json::from_value(json!({})).unwrap();
        let admin: crate::routes::admin::ai_act::AssistBody =
            serde_json::from_value(json!({})).unwrap();
        assert!(!owner.stream);
        assert!(!admin.stream);

        let response = governance_response(false, None, async { Ok(suggestion()) })
            .await
            .unwrap();
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::to_value(suggestion()).unwrap(),
        );

        let response = governance_response(false, None, async { Err(ApiError::FORBIDDEN) })
            .await
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    fn private_bit(id: &str, endpoint: &str) -> Bit {
        Bit {
            id: id.to_owned(),
            updated: "2026-10-28T00:00:00Z".to_owned(),
            parameters: json!({"provider": {
                "provider_name": "custom:openai",
                "model_id": "gpt",
                "params": {"endpoint": endpoint, "api_key": "sk-attacker"},
            }}),
            ..Default::default()
        }
    }

    #[test]
    fn a_requests_private_bits_are_replaced_by_the_callers_library() {
        let profile = Profile {
            bits: vec!["hub.example:public-model".to_owned()],
            custom_bits: vec![ProfileCustomBit(private_bit(
                "victim-claude",
                "https://attacker.example/v1",
            ))],
            ..Default::default()
        };
        let own = private_bit("caller-gpt", "https://models.example.com/v1");

        let resolved = with_private_bits(profile, vec![own]);

        let ids: Vec<&str> = resolved
            .custom_bits
            .iter()
            .map(|bit| bit.0.id.as_str())
            .collect();
        assert_eq!(ids, ["caller-gpt"]);
        assert_eq!(resolved.bits, ["hub.example:public-model"]);
        assert!(
            !serde_json::to_string(&resolved)
                .unwrap()
                .contains("attacker.example")
        );
        assert!(with_private_bits(resolved, vec![]).custom_bits.is_empty());
    }

    /// A Gemini failure quotes its request URL with the caller's key in it. Neither the key,
    /// another private Bit's credential nor any URL reaches the log or the error report; the
    /// rest of the failure does.
    #[test]
    fn a_failed_run_reports_no_credential_and_no_url() {
        let key = "AIzaSyD-governance-test-key-0123456789";
        let header = "hk-governance-header-secret";
        let mut gemini = private_bit("caller-gemini", "https://gemini.example.com/v1beta");
        gemini.parameters["provider"]["params"]["api_key"] = json!(key);
        let mut proxy = private_bit("caller-proxy", "https://proxy.example/v1");
        proxy.parameters["provider"]["params"]["headers"] = json!({"x-api-key": header});
        let profile = with_private_bits(Profile::default(), vec![gemini, proxy]);

        let secrets = private_bit_secrets(Some(&profile));
        let failure = flow_like_types::anyhow!(
            "HttpError: error sending request for url (https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent?alt=sse&key={key})"
        )
        .context(format!("provider echoed {key} and {header}"));
        let message = agent_failure(&failure, &secrets);
        for leaked in [key, header, "googleapis", "?key=", "://"] {
            assert!(!message.contains(leaked), "{leaked} survived: {message}");
        }
        assert_eq!(
            message,
            "Governance agent failed: provider echoed [REDACTED] and [REDACTED]: HttpError: error sending request for url ([url])"
        );
        assert!(private_bit_secrets(None).is_empty());
    }
}
