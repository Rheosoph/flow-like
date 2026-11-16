//! Live custom-Bit fixtures restricted to the isolated development application.

use crate::{
    functions::{
        ai::{copilot, native_tool_bridge::NativeRunOptions},
        local_bots::{self, LocalBots},
    },
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    app::App,
    bit::{Bit, BitTypes, LLMParameters, Metadata},
    copilot::CopilotScope,
    flow::{board::Board, copilot::FlowIrCommitToken},
    flow_like_model_provider::provider::ModelProvider,
    profile::{Profile, ProfileCustomBit},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{
    AppHandle, Manager,
    ipc::{Channel, InvokeResponseBody},
};

const SCHEMA: &str = "flowpilot.custom-bit-live-e2e/v1";
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
static FIXTURES: LazyLock<tokio::sync::Mutex<Option<CustomBitSeed>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomBitFixture {
    pub provider: String,
    pub bit_id: String,
    pub bit_name: String,
    pub model_id: String,
    pub model_name: String,
    pub app_id: String,
    pub board_id: String,
    pub bot_id: String,
    pub board: Board,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomBitSeed {
    pub schema: String,
    pub run_id: String,
    pub profile_id: String,
    pub nonce: String,
    pub app_id: String,
    pub flux_bot_id: String,
    pub fixtures: Vec<CustomBitFixture>,
    pub errors: Vec<Value>,
}

// Settings saves itself on drop. Evidence reads must never write an older snapshot back.
#[derive(Deserialize)]
struct PersistedCustomBitSettings {
    profiles: std::collections::HashMap<String, crate::profile::UserProfile>,
    custom_bits: Vec<Bit>,
}

fn require_isolation() -> Result<(), String> {
    if !crate::e2e_runtime::isolated_runtime_active() {
        return Err("Custom Bit live tests require the isolated development application".into());
    }
    Ok(())
}

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn persisted_settings() -> Result<PersistedCustomBitSettings, String> {
    require_isolation()?;
    let root = crate::e2e_isolation::data_root().ok_or("The isolated data root is unavailable")?;
    let bytes =
        std::fs::read(root.join("cache/flow-like/global-settings.json")).map_err(failure)?;
    serde_json::from_slice(&bytes).map_err(failure)
}

#[tauri::command]
pub async fn custom_bit_e2e_seed(app_handle: AppHandle) -> Result<CustomBitSeed, String> {
    require_isolation()?;
    let mut cached = FIXTURES.lock().await;
    if let Some(seed) = cached.as_ref() {
        return Ok(seed.clone());
    }
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(failure)?;
    let profile_id = settings.lock().await.current_profile.clone();
    if profile_id != "flowpilot-e2e-local" {
        return Err("Custom Bit fixtures require the fresh local E2E profile".into());
    }
    let run_id = std::env::var("FLOWPILOT_E2E_CLI_RUN_ID").map_err(failure)?;
    let nonce = format!("CUSTOM_BIT_{}", flow_like_types::create_id());
    let mut app = crate::functions::app::create_app(
        app_handle.clone(),
        None,
        Metadata {
            name: "Custom Bit live test".into(),
            description: format!("The verification nonce is {nonce}"),
            ..Default::default()
        },
        Vec::new(),
        Some(true),
    )
    .await
    .map_err(failure)?;
    let bots = local_bots::local_bots_list(app_handle.state::<LocalBots>())
        .await
        .map_err(failure)?;
    let flux_bot_id = bots
        .iter()
        .find(|bot| bot.system == Some(flow_like_types::bot::BotSystemKind::Flux))
        .map(|bot| bot.id.clone())
        .ok_or("The isolated profile has no Flux bot")?;
    let mut seed = CustomBitSeed {
        schema: SCHEMA.into(),
        run_id,
        profile_id,
        nonce,
        app_id: app.id.clone(),
        flux_bot_id,
        fixtures: Vec::new(),
        errors: Vec::new(),
    };
    for provider in ["claude-code", "codex"] {
        match seed_provider(&app_handle, &mut app, &seed.profile_id, provider).await {
            Ok(fixture) => seed.fixtures.push(fixture),
            Err(message) => seed
                .errors
                .push(json!({"provider": provider, "message": message})),
        }
    }
    {
        let mut settings = settings.lock().await;
        let profile = settings
            .profiles
            .get_mut(&seed.profile_id)
            .ok_or("The isolated profile disappeared")?;
        profile.hub_profile.bits = seed
            .fixtures
            .iter()
            .map(|fixture| fixture.bit_id.clone())
            .collect();
        settings.try_serialize().map_err(failure)?;
    }
    *cached = Some(seed.clone());
    Ok(seed)
}

async fn seed_provider(
    app_handle: &AppHandle,
    app: &mut App,
    profile_id: &str,
    provider: &str,
) -> Result<CustomBitFixture, String> {
    let status =
        copilot::flowpilot_agent_backend_status(app_handle.clone(), provider.into()).await?;
    if !status.available {
        return Err(format!("{provider} is not installed on this host"));
    }
    let auth =
        copilot::flowpilot_agent_backend_get_auth_status(app_handle.clone(), provider.into())
            .await?;
    if !auth.authenticated {
        return Err(format!("{provider} is not signed in on this host"));
    }
    let models =
        copilot::flowpilot_agent_backend_list_models(app_handle.clone(), provider.into()).await?;
    let concrete = |model: &&copilot::CopilotModelInfo| {
        !model.id.trim().is_empty() && !model.id.eq_ignore_ascii_case("default")
    };
    let model = models
        .iter()
        .filter(concrete)
        .find(|model| model.is_default)
        .or_else(|| models.iter().filter(concrete).next())
        .ok_or_else(|| format!("{provider} returned no specific model"))?;
    let bit_id = flow_like_types::create_id();
    let bit_name = format!("E2E {} ({provider})", model.name);
    let mut bit = Bit {
        id: bit_id.clone(),
        bit_type: BitTypes::Llm,
        parameters: serde_json::to_value(LLMParameters {
            context_length: model
                .context_window
                .and_then(|limit| u32::try_from(limit).ok())
                .unwrap_or(128000),
            model_classification: Default::default(),
            data_classification: None,
            provider: ModelProvider {
                provider_name: format!("custom:{provider}"),
                model_id: Some(model.id.clone()),
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([(
                    "model_id".into(),
                    json!(model.id),
                )])),
            },
        })
        .map_err(failure)?,
        ..Default::default()
    };
    bit.meta.insert(
        "en".into(),
        Metadata {
            name: bit_name.clone(),
            ..Default::default()
        },
    );
    crate::functions::settings::profiles::upsert_custom_bit(app_handle.clone(), bit)
        .await
        .map_err(failure)?;
    app.bits.push(bit_id.clone());
    let created = app.create_board(None, None).await.map_err(failure)?;
    app.save().await.map_err(failure)?;
    let board = app
        .open_board(created.board_id.clone(), Some(false), None)
        .await
        .map_err(failure)?;
    {
        let mut writer = board.write().await;
        writer.name = format!("{provider} live Board Copilot test");
        writer.save(None).await.map_err(failure)?;
    }
    let board = (*board.snapshot()).clone();
    let request = serde_json::from_value(json!({
        "name": format!("E2E {provider}"), "appearance": {"kind": "pip"},
        "instructions": "Follow the test request exactly. Use the permitted read tool when requested.",
        "model": {"primary": bit_id, "fallbacks": []},
        "capabilities": ["selected_apps"], "profile_id": profile_id,
    })).map_err(failure)?;
    let bot = local_bots::local_bots_create(app_handle.state::<LocalBots>(), request)
        .await
        .map_err(failure)?;
    local_bots::local_bots_attach_app(
        app_handle.clone(),
        app_handle.state::<LocalBots>(),
        bot.id.clone(),
        app.id.clone(),
    )
    .await
    .map_err(failure)?;
    Ok(CustomBitFixture {
        provider: provider.into(),
        bit_id,
        bit_name,
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        app_id: app.id.clone(),
        board_id: created.board_id,
        bot_id: bot.id,
        board,
    })
}

async fn fixture(
    app: &AppHandle,
    provider: &str,
) -> Result<(CustomBitSeed, CustomBitFixture), String> {
    require_isolation()?;
    let seed = FIXTURES
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or("Seed the isolated fixtures first")?;
    let fixture = seed
        .fixtures
        .iter()
        .find(|fixture| fixture.provider == provider)
        .cloned()
        .ok_or("This provider has no owned test fixture")?;
    let settings = TauriSettingsState::construct(app).await.map_err(failure)?;
    let settings = settings.lock().await;
    if settings.current_profile != seed.profile_id {
        return Err("The test profile changed".into());
    }
    let persisted = persisted_settings()?;
    let bit = persisted
        .custom_bits
        .iter()
        .find(|bit| bit.id == fixture.bit_id)
        .ok_or("The persisted custom Bit disappeared")?;
    let parameters = bit
        .try_to_llm()
        .ok_or("The fixture is not a language model")?;
    if parameters.provider.provider_name != format!("custom:{provider}")
        || parameters.provider.model_id.as_deref() != Some(fixture.model_id.as_str())
        || parameters
            .provider
            .params
            .as_ref()
            .and_then(|params| params.get("model_id"))
            .and_then(Value::as_str)
            != Some(fixture.model_id.as_str())
    {
        return Err("The persisted custom provider or model identity changed".into());
    }
    Ok((seed, fixture))
}

fn stream_payload(chunk: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(chunk) {
        return Some(value);
    }
    let tag = flow_like::flow::copilot::stream::frame_tag(chunk)?;
    let body = chunk
        .strip_prefix(&format!("<{tag}>"))?
        .strip_suffix(&format!("</{tag}>"))?;
    let mut value: Value = serde_json::from_str(body).ok()?;
    value["type"] = json!(tag);
    Some(value)
}

#[tauri::command]
pub async fn custom_bit_e2e_run_bot(
    app_handle: AppHandle,
    provider: String,
) -> Result<Value, String> {
    let (seed, fixture) = fixture(&app_handle, &provider).await?;
    let bot = local_bots::local_bots_get(app_handle.state::<LocalBots>(), fixture.bot_id.clone())
        .await
        .map_err(failure)?
        .ok_or("The fixture bot disappeared")?;
    if bot.model.primary.as_deref() != Some(fixture.bit_id.as_str())
        || !bot.model.fallbacks.is_empty()
    {
        return Err("The test bot's exact primary model policy changed".into());
    }
    let frames = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = frames.clone();
    let channel = Channel::<String>::new(move |body: InvokeResponseBody| {
        let text = match body {
            InvokeResponseBody::Json(body) => serde_json::from_str::<String>(&body).ok(),
            InvokeResponseBody::Raw(body) => String::from_utf8(body).ok(),
        };
        if let Some(value) = text.as_deref().and_then(stream_payload) {
            let mut frames = captured.lock().unwrap_or_else(|error| error.into_inner());
            if frames.len() < 512 {
                frames.push(value);
            }
        }
        Ok(())
    });
    let run_id = format!("custom-bit-bot-{}", flow_like_types::create_id());
    let prompt = format!(
        "Use get_app_detail to read app {}. Return only the exact verification nonce from its description. Do not modify anything.",
        fixture.app_id
    );
    let response = tokio::time::timeout(
        RUN_TIMEOUT,
        copilot::global_chat_native(
            app_handle.clone(),
            None,
            app_handle.state::<TauriFlowLikeState>(),
            CopilotScope::Both,
            prompt,
            None,
            Some(Vec::new()),
            Some(fixture.bit_id.clone()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(run_id.clone()),
            channel,
            NativeRunOptions {
                profile_id: Some(seed.profile_id.clone()),
                auto_approve: true,
                ..Default::default()
            },
            Some(fixture.bot_id.clone()),
        ),
    )
    .await;
    let response = match response {
        Ok(response) => response?,
        Err(_) => {
            let _ = copilot::cancel_copilot_chat(run_id.clone());
            return Err("The live bot test timed out".into());
        }
    };
    let frames = frames
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let models: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "native_run_model")
        .filter_map(|frame| frame["model_id"].as_str())
        .collect();
    let model_matches = !models.is_empty() && models.iter().all(|model| *model == fixture.bit_id);
    let tool_called = frames
        .iter()
        .any(|frame| frame["type"] == "tool_start" && frame["tool"] == "get_app_detail");
    let tool_returned = frames.iter().any(|frame| {
        frame["type"] == "tool_end"
            && frame["tool"] == "get_app_detail"
            && frame["status"] == "done"
    });
    let nonce_matches = response.message.contains(&seed.nonce);
    let completion_models: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "usage_stat")
        .flat_map(|frame| frame["stats"]["calls"].as_array().into_iter().flatten())
        .filter_map(|call| call["model"].as_str())
        .collect();
    let usage_observed = !completion_models.is_empty();
    let output_tokens = frames
        .iter()
        .filter(|frame| frame["type"] == "usage_stat")
        .filter_map(|frame| frame["stats"]["usage"]["completion_tokens"].as_u64())
        .max()
        .unwrap_or(0);
    let completion_models_match = usage_observed
        && completion_models
            .iter()
            .all(|model| *model == fixture.model_id);
    Ok(json!({
        "schema": SCHEMA, "provider": provider, "runId": run_id, "bitId": fixture.bit_id,
        "modelId": fixture.model_id, "providerName": format!("custom:{}", fixture.provider),
        "passed": model_matches && completion_models_match && output_tokens > 0 && tool_called && tool_returned && nonce_matches,
        "modelMatches": model_matches, "toolCalled": tool_called, "toolReturned": tool_returned,
        "nonceMatches": nonce_matches, "usageObserved": usage_observed,
        "outputTokens": output_tokens,
        "completionModelsMatch": completion_models_match, "completionModels": completion_models,
        "message": response.message, "frames": frames,
    }))
}

#[tauri::command]
pub async fn custom_bit_e2e_run_workflows(
    app_handle: AppHandle,
    provider: String,
) -> Result<Value, String> {
    let (seed, fixture) = fixture(&app_handle, &provider).await?;
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(failure)?;
    let previous = {
        let mut settings = settings.lock().await;
        let profile = settings
            .profiles
            .get_mut(&seed.profile_id)
            .ok_or("The isolated profile disappeared")?;
        let previous =
            std::mem::replace(&mut profile.hub_profile.bits, vec![fixture.bit_id.clone()]);
        settings.try_serialize().map_err(failure)?;
        previous
    };
    let result = async {
        let persisted = persisted_settings()?;
        let mut profile: Profile = persisted
            .profiles
            .get(&seed.profile_id)
            .ok_or("The persisted isolated profile disappeared")?
            .hub_profile
            .clone();
        profile.custom_bits = persisted
            .custom_bits
            .iter()
            .cloned()
            .map(ProfileCustomBit)
            .collect();
        let state = TauriFlowLikeState::construct(&app_handle)
            .await
            .map_err(failure)?;
        crate::custom_bit_e2e_workflows::execute_workflows(
            state,
            Arc::new(profile),
            &fixture.bit_id,
            &seed.nonce,
        )
        .await
    }
    .await;
    {
        let mut settings = settings.lock().await;
        if let Some(profile) = settings.profiles.get_mut(&seed.profile_id) {
            profile.hub_profile.bits = previous;
        }
        settings.try_serialize().map_err(failure)?;
    }
    result
}

async fn close_fixture_board(app_handle: &AppHandle, board_id: &str) -> Result<(), String> {
    let state = TauriFlowLikeState::construct(app_handle)
        .await
        .map_err(failure)?;
    if state.board_registry.contains_key(board_id) {
        crate::functions::flow::board::close_board(app_handle.clone(), board_id.into())
            .await
            .map_err(failure)?;
    }
    if state.board_registry.contains_key(board_id) {
        return Err("The fixture board still has an in-flight writer; stop the editor before testing lazy reopen".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn custom_bit_e2e_apply_review(
    app_handle: AppHandle,
    provider: String,
    token: FlowIrCommitToken,
) -> Result<Value, String> {
    let (seed, fixture) = fixture(&app_handle, &provider).await?;
    if token.board_id != fixture.board_id {
        return Err("The compiled review is not for this fixture board".into());
    }
    close_fixture_board(&app_handle, &fixture.board_id).await?;
    let job = copilot::flowpilot_create_board_edit_job(
        app_handle.clone(),
        fixture.app_id.clone(),
        None,
        token.clone(),
    )
    .await?;
    let state = TauriFlowLikeState::construct(&app_handle)
        .await
        .map_err(failure)?;
    let create_reopened = state.board_registry.contains_key(&fixture.board_id);
    close_fixture_board(&app_handle, &fixture.board_id).await?;
    let resolution = copilot::flowpilot_resolve_board_edit_job(
        app_handle.clone(),
        job.job_id.clone(),
        true,
        Some(true),
        None,
        None,
        None,
    )
    .await?;
    let apply_reopened = state.board_registry.contains_key(&fixture.board_id);
    let replay =
        copilot::flowpilot_apply_flow_ir_commit(app_handle.clone(), fixture.app_id.clone(), token)
            .await;
    let readback = copilot::flowpilot_read_flow_ir_commit_board(
        app_handle.clone(),
        fixture.app_id.clone(),
        fixture.board_id.clone(),
    )
    .await?;
    let app = App::load(fixture.app_id.clone(), state)
        .await
        .map_err(failure)?;
    let board = app
        .open_board(fixture.board_id.clone(), Some(false), None)
        .await
        .map_err(failure)?;
    let fingerprint = copilot::persisted_board_graph_fingerprint(&board.snapshot())?;
    let persisted_matches = resolution.job.persisted_board_fingerprint.as_deref()
        == Some(readback.graph_fingerprint.as_str())
        && fingerprint == readback.graph_fingerprint;
    let nonce_persisted = readback.flowscript.contains(&seed.nonce);
    let applied = resolution
        .job
        .result
        .as_ref()
        .is_some_and(|result| result.status == "applied");
    Ok(json!({
        "schema": SCHEMA, "provider": provider, "appId": fixture.app_id, "boardId": fixture.board_id,
        "passed": create_reopened && apply_reopened && persisted_matches && nonce_persisted && applied && replay.replayed,
        "closedBeforeCreate": true, "createReopened": create_reopened, "closedBeforeApply": true,
        "applyReopened": apply_reopened, "persistedMatches": persisted_matches,
        "noncePersisted": nonce_persisted,
        "idempotentReplay": replay.replayed, "job": resolution.job, "board": board.snapshot(), "readback": readback,
    }))
}
