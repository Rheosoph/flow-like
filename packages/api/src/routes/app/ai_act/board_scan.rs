//! Static board scan that derives EU AI Act [`Signals`] for an app without
//! executing anything. Inspects each board's nodes to detect model-bearing
//! nodes, capability categories (chatbot/GenAI/biometric/scraping) and reads
//! cached board scores for the security/governance posture. See
//! todo/EU-AI.md §1.4 and §2.1.

use super::questionnaire::{Classification, classify};
use super::signals::{CapabilitySignals, DetectedModel, Signals};
use crate::entity::app_board_score;
use crate::state::AppState;
use flow_like::app::{App, AppCategory};
use flow_like::flow::board::Board;
use flow_like::flow::node::Node;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;
use std::collections::BTreeMap;

/// Node `name` for the dynamic "Find Model" selector.
const FIND_MODEL_NODE: &str = "ai_generative_find_model";

/// Map [`AppCategory`] to the uppercase string the classifier expects.
fn category_label(category: &AppCategory) -> &'static str {
    match category {
        AppCategory::Health => "HEALTH",
        AppCategory::Finance => "FINANCE",
        AppCategory::Education => "EDUCATION",
        AppCategory::Business => "BUSINESS",
        AppCategory::Social => "SOCIAL",
        AppCategory::Communication => "COMMUNICATION",
        _ => "OTHER",
    }
}

/// Classify a single node's contribution to the capability signals by its
/// `name`/`category`. Matching is substring-based against the stable node-name
/// identifiers (e.g. `ai_generative_invoke`, `agent_invoke`).
fn apply_node_capabilities(node: &Node, caps: &mut CapabilitySignals) {
    let name = node.name.as_str();
    let category = node.category.to_lowercase();

    let is_agent = name.starts_with("agent_");
    let is_generative = name.starts_with("ai_generative_")
        || name.starts_with("ai_image_")
        || name.starts_with("ai_video_")
        || name.starts_with("ai_audio_")
        || name == "ai_llm_summarize";

    if is_agent {
        caps.has_chatbot = true;
        caps.has_genai = true;
    }
    if name.contains("invoke") && (name.starts_with("ai_generative_") || is_agent) {
        caps.has_chatbot = true;
    }
    if is_generative {
        caps.has_genai = true;
    }

    if name.contains("emotion") || name.contains("sentiment") {
        caps.has_emotion_biometric = true;
    }
    if name.contains("biometric")
        || name.contains("face")
        || category.contains("biometric")
        || category.contains("vision")
    {
        caps.has_biometric_id = true;
    }
    if name.contains("scrap") || name.contains("crawl") || category.contains("scraping") {
        caps.has_web_scraping = true;
    }
}

/// Recursively walk a serde_json value looking for an embedded `ModelProvider`
/// shape (`{ "provider_name": "...", "model_id": "..." }`) and collect matches.
fn collect_models_from_value(value: &Value, out: &mut Vec<DetectedModel>) {
    match value {
        Value::Object(map) => {
            if let Some(provider) = map.get("provider_name").and_then(Value::as_str) {
                let model_id = map
                    .get("model_id")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                if let Some(model_id) = model_id {
                    out.push(DetectedModel {
                        model_id,
                        provider: Some(provider.to_string()),
                        dynamic_selector: false,
                    });
                }
            }
            for v in map.values() {
                collect_models_from_value(v, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_models_from_value(v, out);
            }
        }
        _ => {}
    }
}

/// Extract any statically-configured models from a node's pin default values.
fn extract_node_models(node: &Node, out: &mut Vec<DetectedModel>) {
    if node.name == FIND_MODEL_NODE {
        // Dynamic selector: the concrete model is resolved at runtime.
        out.push(DetectedModel {
            model_id: "dynamic".to_string(),
            provider: None,
            dynamic_selector: true,
        });
    }

    for pin in node.pins.values() {
        let Some(bytes) = pin.default_value.as_ref() else {
            continue;
        };
        let Ok(value) = flow_like_types::json::from_slice::<Value>(bytes) else {
            continue;
        };
        collect_models_from_value(&value, out);
    }
}

/// Scan a single board into the running [`ScanAccumulator`].
fn scan_board(board: &Board, acc: &mut ScanAccumulator) {
    for node in board.nodes.values() {
        if node.name == "reroute" {
            continue;
        }
        apply_node_capabilities(node, &mut acc.capabilities);
        extract_node_models(node, &mut acc.models);
    }
}

#[derive(Default)]
struct ScanAccumulator {
    models: Vec<DetectedModel>,
    capabilities: CapabilitySignals,
}

/// Deduplicate detected models by provider/model_id, preserving a single
/// dynamic-selector marker.
fn dedup_models(models: Vec<DetectedModel>) -> Vec<DetectedModel> {
    let mut map: BTreeMap<String, DetectedModel> = BTreeMap::new();
    for m in models {
        let key = if m.dynamic_selector {
            "::dynamic::".to_string()
        } else {
            format!(
                "{}/{}",
                m.provider.as_deref().unwrap_or("unknown"),
                m.model_id
            )
        };
        map.entry(key).or_insert(m);
    }
    map.into_values().collect()
}

/// Run a full static scan of an app's boards and combine with cached board
/// scores to produce [`Signals`]. Loads each board via the master credentials
/// (caller already authorised). Best-effort: a board that fails to load is
/// skipped rather than aborting the scan.
pub async fn scan_app_signals(
    state: &AppState,
    sub: &str,
    app_id: &str,
    app: &App,
) -> flow_like_types::Result<Signals> {
    let mut acc = ScanAccumulator::default();

    for board_id in &app.boards {
        let board = match state.master_board(sub, app_id, board_id, state, None).await {
            Ok(board) => board,
            Err(err) => {
                tracing::warn!(board_id, error = %err, "ai-act scan: skipping unloadable board");
                continue;
            }
        };
        scan_board(&board, &mut acc);
    }

    // Board scores -> MIN security / governance across boards.
    let scores = app_board_score::Entity::find()
        .filter(app_board_score::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .unwrap_or_default();

    let min_security = scores.iter().map(|s| s.security).min();
    let min_governance = scores.iter().map(|s| s.governance).min();

    let primary_category = app
        .primary_category
        .as_ref()
        .map(|c| category_label(c).to_string());
    let secondary_category = app
        .secondary_category
        .as_ref()
        .map(|c| category_label(c).to_string());

    Ok(Signals {
        models: dedup_models(acc.models),
        capabilities: acc.capabilities,
        primary_category,
        secondary_category,
        min_security,
        min_governance,
        download_count: app.download_count as i64,
        interaction_count: app.interactions_count as i64,
        trains_own_model: false,
    })
}

/// Convenience: scan and immediately classify with a given answer set.
pub async fn scan_and_classify(
    state: &AppState,
    sub: &str,
    app_id: &str,
    app: &App,
    answers: &Value,
) -> flow_like_types::Result<(Signals, Classification)> {
    let signals = scan_app_signals(state, sub, app_id, app).await?;
    let classification = classify(answers, &signals);
    Ok((signals, classification))
}
