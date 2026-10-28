//! Bridge between the API and the core [`GovernanceCopilot`]. Loads an app's
//! boards, renders them to FlowScript and runs the read-only governance agent
//! to propose EU AI Act questionnaire answers. Shared by the owner
//! (`/apps/{app_id}/ai-act/assessment/suggest`) and admin
//! (`/admin/ai-act/assist`) surfaces.

use std::sync::Arc;

use flow_like::bit::Bit;
use flow_like::copilot::governance::{GovernanceCopilot, GovernanceSuggestion};
use flow_like::models::llm::ModelUsageContext;
use flow_like::profile::{Profile, ProfileCustomBit};
use flow_like::state::FlowLikeState;

use crate::{
    error::ApiError,
    routes::{
        app::ai_act::{board_scan, questionnaire::questionnaire_schema, signals::Signals},
        execution::models::{provider_of, scrub, secret_values},
    },
    state::AppState,
};

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
) -> Result<(GovernanceSuggestion, Signals, String), ApiError> {
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

    Ok((suggestion, signals, model))
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
    use serde_json::json;

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
