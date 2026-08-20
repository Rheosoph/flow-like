//! Auto-derived signals that feed the EU AI Act classifier and questionnaire
//! prefill. Everything here is observed from data already on the platform
//! (boards, board scores, category, monitoring) so the owner confirms rather
//! than types. See todo/EU-AI.md §2.1.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A model discovered on a board (static scan) or from monitoring.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedModel {
    pub model_id: String,
    pub provider: Option<String>,
    /// True for dynamic selectors (e.g. the `Find Model` node).
    pub dynamic_selector: bool,
}

/// Capabilities inferred from the node types / categories present in an app's
/// boards. Native catalog nodes are inferred from type/category; external
/// (WASM) nodes additionally contribute permission-derived signals.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySignals {
    /// Chatbot / agent presence -> Art. 50 chatbot disclosure prefill.
    pub has_chatbot: bool,
    /// LLM invoke / generative nodes -> GenAI labelling prefill.
    pub has_genai: bool,
    /// Emotion-recognition / biometric-categorisation nodes.
    pub has_emotion_biometric: bool,
    /// Vision / face / biometric ID nodes (Annex III prior).
    pub has_biometric_id: bool,
    /// Web scraping / crawling nodes.
    pub has_web_scraping: bool,
    /// External (WASM) nodes that requested network egress.
    pub external_network: bool,
    /// External (WASM) nodes that requested storage writes.
    pub external_storage_write: bool,
}

/// The complete signal snapshot for an app.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Signals {
    /// Models discovered via the static board scan (§1.4.1, source = board_scan).
    pub models: Vec<DetectedModel>,
    pub capabilities: CapabilitySignals,
    /// App primary/secondary category (HEALTH, FINANCE, EDUCATION, ...).
    pub primary_category: Option<String>,
    pub secondary_category: Option<String>,
    /// MIN security score across boards (0-10), if scored.
    pub min_security: Option<i32>,
    /// MIN governance score across boards (0-10), if scored.
    pub min_governance: Option<i32>,
    /// Exposure scale.
    pub download_count: i64,
    pub interaction_count: i64,
    /// True when the app trains/fine-tunes its own model (rare; inferred conservatively).
    pub trains_own_model: bool,
}

impl Signals {
    /// Category names that are priors for Annex III high-risk domains.
    #[allow(dead_code)]
    pub fn category_is_annex_iii_prior(&self) -> bool {
        const ANNEX_III: &[&str] = &["HEALTH", "FINANCE", "EDUCATION"];
        let in_set = |c: &Option<String>| {
            c.as_deref()
                .map(|s| ANNEX_III.contains(&s.to_uppercase().as_str()))
                .unwrap_or(false)
        };
        in_set(&self.primary_category) || in_set(&self.secondary_category)
    }

    /// Distinct provider/model pairs as a stable set of "provider/model" keys.
    #[allow(dead_code)]
    pub fn model_keys(&self) -> BTreeSet<String> {
        self.models
            .iter()
            .map(|m| {
                format!(
                    "{}/{}",
                    m.provider.as_deref().unwrap_or("unknown"),
                    m.model_id
                )
            })
            .collect()
    }
}
