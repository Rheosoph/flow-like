//! Governance copilot — a dedicated, read-only assistant that inspects an
//! app's boards in their FlowScript representation and proposes EU AI Act
//! questionnaire answers. This is a separate agent route from the board-edit
//! Copilot: it never mutates a board, has no editing tools, and returns a
//! structured suggestion the owner (or admin) confirms.
//!
//! See todo/EU-AI.md §2.3.1 — the governance agent reasons over the textual
//! FlowScript state so it can explain *why* an app may be high-risk in terms
//! of the actual graph the user authored.

use std::sync::Arc;

use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::Completion,
    message::UserContent,
    streaming::StreamedAssistantContent,
};
use serde::{Deserialize, Serialize};

use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters};
use crate::flow::ast::{RenderOptions, board_to_flowscript};
use crate::flow::board::Board;
use crate::profile::Profile;
use crate::state::FlowLikeState;
use flow_like_model_provider::provider::ModelProvider;

/// One board rendered to FlowScript for the governance agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoardDepiction {
    pub board_id: String,
    pub name: String,
    pub flowscript: String,
}

/// A single suggested questionnaire answer with a short justification grounded
/// in the board contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedAnswer {
    /// Questionnaire question key (see api questionnaire `keys`).
    pub key: String,
    /// Suggested value. For multi-select questions this is a JSON array
    /// encoded as a string list; the API normalises it.
    pub value: serde_json::Value,
    /// One-sentence rationale referencing the board (e.g. node names).
    pub rationale: String,
    /// Model confidence 0.0-1.0; low confidence flags for human review.
    #[serde(default)]
    pub confidence: f32,
}

/// Structured result returned by the governance agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernanceSuggestion {
    /// One-sentence description of what the app does, derived from the boards.
    pub purpose: String,
    pub suggested_answers: Vec<SuggestedAnswer>,
    /// Free-form notes for the reviewer (caveats, ambiguities).
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Read-only governance assistant.
pub struct GovernanceCopilot {
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
}

impl GovernanceCopilot {
    pub fn new(state: Arc<FlowLikeState>, profile: Option<Arc<Profile>>) -> Self {
        Self { state, profile }
    }

    /// Render boards to FlowScript with stable anchors so the agent reasons
    /// over the same textual surface the editor copilot uses.
    pub fn depict_boards(boards: &[Board]) -> Vec<BoardDepiction> {
        boards
            .iter()
            .map(|board| BoardDepiction {
                board_id: board.id.clone(),
                name: board.name.clone(),
                flowscript: board_to_flowscript(
                    board,
                    &RenderOptions {
                        anchors: true,
                        ..Default::default()
                    },
                ),
            })
            .collect()
    }

    /// Resolve the completion model, reusing the user's configured copilot
    /// model when a profile is present, otherwise falling back to gpt-4o.
    /// Mirrors `flow::copilot::Copilot::get_model`.
    async fn resolve_model(
        &self,
        model_id: Option<String>,
        token: Option<String>,
    ) -> Result<(
        String,
        Box<dyn rig::client::completion::CompletionClientDyn + Send + Sync>,
    )> {
        let bit = if let Some(profile) = &self.profile {
            if let Some(id) = model_id {
                profile
                    .find_bit(&id, self.state.http_client.clone())
                    .await?
            } else {
                let preference = BitModelPreference {
                    reasoning_weight: Some(1.0),
                    ..Default::default()
                };
                profile
                    .get_best_model(&preference, false, true, self.state.http_client.clone())
                    .await?
            }
        } else {
            Bit {
                id: "gpt-4o".to_string(),
                bit_type: BitTypes::Llm,
                parameters: serde_json::to_value(LLMParameters {
                    context_length: 128000,
                    provider: ModelProvider {
                        provider_name: "openai".to_string(),
                        model_id: None,
                        version: None,
                        params: None,
                    },
                    model_classification: Default::default(),
                })
                .unwrap_or_default(),
                ..Default::default()
            }
        };

        let model_factory = self.state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(&bit, self.state.clone(), token, None)
            .await?;
        let default_model = model.default_model().await.unwrap_or("gpt-4o".to_string());
        let provider = model.provider().await?;
        let completion = provider.into_client();
        Ok((default_model, completion))
    }

    /// Run the governance analysis. `context_json` carries the auto-derived
    /// signals and the questionnaire schema (serialised by the caller) so the
    /// model has the canonical question keys and detected capabilities.
    ///
    /// Returns the parsed suggestion together with the resolved model name that
    /// produced it, so callers can attribute the reasoning to a concrete model.
    pub async fn assist(
        &self,
        depictions: &[BoardDepiction],
        context_json: &str,
        model_id: Option<String>,
        token: Option<String>,
    ) -> Result<(GovernanceSuggestion, String)> {
        let (model_name, completion_client) = self.resolve_model(model_id, token).await?;

        let system_prompt = governance_system_prompt(context_json);

        let mut boards_text = String::new();
        for depiction in depictions {
            boards_text.push_str(&format!(
                "\n## Board: {} (id: {})\n```flowscript\n{}\n```\n",
                depiction.name, depiction.board_id, depiction.flowscript
            ));
        }
        if boards_text.is_empty() {
            boards_text.push_str("\n(No boards found for this app.)\n");
        }

        let user_text = format!(
            "Analyse the following boards and produce the governance suggestion JSON.\n{boards_text}"
        );

        let agent = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .build();
        let prompt_message = rig::message::Message::User {
            content: OneOrMany::one(UserContent::Text(rig::message::Text { text: user_text })),
        };

        let request = agent
            .completion(prompt_message, Vec::<rig::message::Message>::new())
            .await
            .map_err(|e| flow_like_types::anyhow!("Governance completion error: {e}"))?;

        let mut stream = request
            .stream()
            .await
            .map_err(|e| flow_like_types::anyhow!("Governance stream error: {e}"))?;

        let mut full_text = String::new();
        while let Some(item) = stream.next().await {
            let content =
                item.map_err(|e| flow_like_types::anyhow!("Governance chunk error: {e}"))?;
            if let StreamedAssistantContent::Text(text) = content {
                full_text.push_str(&text.text);
            }
            // Tool/Reasoning/Final variants are ignored: no tools are attached.
        }

        Ok((parse_suggestion(&full_text), model_name))
    }
}

/// Extract the JSON object from a possibly fenced / chatty model reply and
/// deserialise it into a [`GovernanceSuggestion`]. Lenient: on failure returns
/// a suggestion carrying the raw text as a note so the reviewer still sees it.
fn parse_suggestion(text: &str) -> GovernanceSuggestion {
    let candidate = extract_json_object(text);
    if let Some(candidate) = candidate {
        if let Ok(parsed) = serde_json::from_str::<GovernanceSuggestion>(&candidate) {
            return parsed;
        }
    }
    GovernanceSuggestion {
        purpose: String::new(),
        suggested_answers: Vec::new(),
        notes: vec![format!(
            "Could not parse a structured suggestion from the model. Raw reply: {}",
            text.chars().take(500).collect::<String>()
        )],
    }
}

/// Find the first balanced top-level `{...}` block in a string.
fn extract_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// System prompt for the governance agent. Read-only, returns JSON only.
fn governance_system_prompt(context_json: &str) -> String {
    format!(
        r#"You are an EU AI Act governance analyst embedded in the Flow-Like platform.

Your job: inspect an app's automation boards (given as FlowScript, the textual
representation of the node graph) and the auto-derived signals, then propose
answers to the platform's EU AI Act questionnaire. You DO NOT modify boards and
you have no tools — reason only over the provided FlowScript and signals.

You are given this context (auto-derived signals + the questionnaire schema with
canonical question keys and options):
```json
{context_json}
```

Guidelines:
- Ground every suggestion in concrete evidence from the FlowScript (reference node
  names or capabilities). Never invent capabilities the boards do not show.
- For yes/no questions use "yes" or "no". For multi-select questions return a JSON
  array of option `value` strings. For text questions return a string.
- Be conservative: if the boards are ambiguous about a high-risk or prohibited use,
  prefer "unsure" where the option exists and explain why in the rationale.
- The `purpose` must be a single plain-language sentence describing what the app does.
- Set `confidence` between 0 and 1; use < 0.5 when the boards are unclear.

Respond with ONLY a JSON object, no prose, no code fences, in exactly this shape:
{{
  "purpose": "string",
  "suggestedAnswers": [
    {{ "key": "question_key", "value": <answer>, "rationale": "string", "confidence": 0.0 }}
  ],
  "notes": ["string"]
}}"#
    )
}
