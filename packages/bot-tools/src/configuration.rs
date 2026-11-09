//! A bot may inspect its settings and propose a change to its owner. Hosts bind the backend to
//! the current bot; the tool arguments cannot select another bot or change its ownership.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::{Result, bot::BotCapability};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    ToolEnv, ToolError, ToolOutput, ToolResult,
    spec::{Approval, ToolSpec, schema_of},
};

pub const BOT_CONFIGURATION: &str = "bot_configuration";
pub const BOT_MODELS: &str = "bot_models";
pub const BOT_UPDATE_CONFIGURATION: &str = "bot_update_configuration";

const TEXT_FIELDS: &[(&str, usize)] = &[
    ("name", 64),
    ("description", 2_000),
    ("instructions", 8_000),
    ("embedding_model", 256),
    ("stt_model", 256),
    ("tts_model", 256),
    ("image_model", 256),
    ("vision_model", 256),
    ("subtask_model", 256),
];

pub fn is_configuration_tool(name: &str) -> bool {
    matches!(
        name,
        BOT_CONFIGURATION | BOT_MODELS | BOT_UPDATE_CONFIGURATION
    )
}

/// Auto mode and standing tool grants never approve a settings change: one to this bot's own
/// settings, a new bot, or another bot's settings.
pub fn requires_approval(name: &str) -> bool {
    name == BOT_UPDATE_CONFIGURATION || crate::crew::requires_approval(name)
}

/// Tools bound to the owner's bots rather than to an app or the workspace: this bot's own
/// settings, and Flux's management of the owner's other bots. They need a bot turn and take no
/// app scope.
pub fn is_settings_tool(name: &str) -> bool {
    is_configuration_tool(name) || crate::crew::is_crew_tool(name)
}

/// The proposed settings an approval card shows for a settings change; `None` for every other
/// tool.
pub fn proposed_changes(name: &str, args: &Value) -> Option<Value> {
    if name == BOT_UPDATE_CONFIGURATION {
        return args.get("changes").cloned();
    }
    crate::crew::proposed_changes(name, args)
}

#[async_trait]
pub trait ConfigurationBackend: Send + Sync {
    async fn inspect(&self) -> Result<Value>;
    async fn models(&self) -> Result<Value>;
    /// The host must obtain approval for these exact changes before calling this method.
    async fn update(&self, changes: Value) -> Result<Value>;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    pub changes: Value,
}

/// The JSON Schema properties of every settings field [`validate_changes`] accepts.
pub(crate) fn settings_properties(capabilities: &[&str]) -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    for (name, max) in TEXT_FIELDS {
        properties.insert(
            (*name).into(),
            json!({"type": ["string", "null"], "maxLength": max}),
        );
    }
    properties.insert("capabilities".into(), json!({
        "type": "array", "uniqueItems": true,
        "description": "The complete desired capability list. Read the current settings first and preserve capabilities the owner wants to keep. Host and built-in restrictions still apply.",
        "items": {"type":"string", "enum": capabilities},
    }));
    properties.insert("model".into(), json!({
        "type": "object", "additionalProperties": false, "minProperties": 1,
        "description": "Partial chat model policy. Omitted fields keep their current values; null clears primary or decision.",
        "properties": {
            "primary": {"type": ["string", "null"], "maxLength": 256},
            "decision": {"type": ["string", "null"], "maxLength": 256},
            "fallbacks": {"type": "array", "maxItems": 3, "uniqueItems": true, "items": {"type": "string", "minLength": 1, "maxLength": 256}}
        }
    }));
    properties
}

fn changes_schema() -> Value {
    let properties = settings_properties(&BotCapability::ALL.map(BotCapability::as_str));
    json!({"type": "object", "additionalProperties": false, "minProperties": 1, "properties": properties})
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: BOT_CONFIGURATION,
            description: "Read your current name, description, instructions (system prompt), capabilities and model mappings before proposing changes. This tool only inspects your own bot and does not change settings.",
            capability: None,
            module: "configuration",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ReadArgs>(),
        },
        ToolSpec {
            name: BOT_MODELS,
            description: "List the models in your owner's active profile and their supported roles. Use this list to recommend a compatible model for a missing mapping, using its exact id; never invent model ids. This tool makes no changes.",
            capability: None,
            module: "configuration",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ReadArgs>(),
        },
        ToolSpec {
            name: BOT_UPDATE_CONFIGURATION,
            description: "Propose changes to your own name, description, instructions (system prompt), capabilities or model mappings. Read bot_configuration first and bot_models before choosing a model. Every call requires the owner's explicit approval, including in auto mode. Send only the fields to change; null clears a nullable field. Capabilities replace the full list; model fields are patched individually. Changes take effect on the next turn. Never claim a new capability is available in this turn.",
            capability: None,
            module: "configuration",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: json!({"type":"object", "additionalProperties":false, "required":["changes"], "properties":{"changes": changes_schema()}}),
        },
    ]
}

fn nullable_text(field: &str, value: &Value, max: usize) -> ToolResult<()> {
    if value.is_null() {
        return Ok(());
    }
    let text = value
        .as_str()
        .ok_or_else(|| ToolError::invalid(format!("{field} must be text or null")))?;
    if text.chars().count() > max || text.contains('\0') {
        return Err(ToolError::invalid(format!(
            "{field} must contain at most {max} characters and no null characters"
        )));
    }
    if field == "name" && text.trim().is_empty() {
        return Err(ToolError::invalid("name must not be blank"));
    }
    Ok(())
}

/// Rejects unsupported fields before a host deserializes a patch into its wider settings API.
pub fn validate_changes(changes: &Value) -> ToolResult<()> {
    let fields = changes
        .as_object()
        .filter(|fields| !fields.is_empty())
        .ok_or_else(|| ToolError::invalid("changes must be a non-empty object"))?;
    for (field, value) in fields {
        if let Some((_, max)) = TEXT_FIELDS.iter().find(|(name, _)| *name == field) {
            nullable_text(field, value, *max)?;
        } else if field == "capabilities" {
            let capabilities = value
                .as_array()
                .ok_or_else(|| ToolError::invalid("capabilities must be a list"))?;
            let mut seen = std::collections::BTreeSet::new();
            for capability in capabilities {
                let id = capability.as_str().ok_or_else(|| {
                    ToolError::invalid("Each capability must be an id from bot_configuration")
                })?;
                if !BotCapability::ALL.iter().any(|known| known.as_str() == id) || !seen.insert(id)
                {
                    return Err(ToolError::invalid(format!(
                        "Unknown or repeated capability: {id}"
                    )));
                }
            }
        } else if field == "model" {
            let policy = value
                .as_object()
                .filter(|policy| !policy.is_empty())
                .ok_or_else(|| ToolError::invalid("model must be a non-empty object"))?;
            for (key, value) in policy {
                match key.as_str() {
                    "primary" | "decision" => nullable_text(key, value, 256)?,
                    "fallbacks" => {
                        let ids =
                            value
                                .as_array()
                                .filter(|ids| ids.len() <= 3)
                                .ok_or_else(|| {
                                    ToolError::invalid(
                                        "model.fallbacks must list at most three model ids",
                                    )
                                })?;
                        let mut seen = std::collections::BTreeSet::new();
                        for id in ids {
                            let id = id.as_str().filter(|id| !id.trim().is_empty()).ok_or_else(
                                || ToolError::invalid("Each fallback must be a model id"),
                            )?;
                            nullable_text("model.fallbacks", &json!(id), 256)?;
                            if !seen.insert(id) {
                                return Err(ToolError::invalid(
                                    "Fallback model ids must not repeat",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(ToolError::invalid(format!(
                            "Cannot change model.{key} with this tool"
                        )));
                    }
                }
            }
        } else {
            return Err(ToolError::invalid(format!(
                "Cannot change {field} with this tool"
            )));
        }
    }
    Ok(())
}

/// The settings safe to show the bot; never returns credentials or arbitrary stored metadata.
pub fn snapshot(bot: &Value) -> Value {
    let names = [
        "id",
        "name",
        "description",
        "instructions",
        "capabilities",
        "model",
        "embedding_model",
        "stt_model",
        "tts_model",
        "image_model",
        "vision_model",
        "subtask_model",
        "system",
        "location",
        "profile_id",
        "updated_at",
    ];
    Value::Object(
        names
            .into_iter()
            .filter_map(|name| bot.get(name).map(|value| (name.to_string(), value.clone())))
            .collect(),
    )
}

/// Expands a partial model policy against the current policy before the settings API reads it.
pub fn merge_model_changes(changes: &mut Value, current_model: &Value) -> ToolResult<()> {
    validate_changes(changes)?;
    let Some(patch) = changes.get_mut("model") else {
        return Ok(());
    };
    let mut merged = current_model.as_object().cloned().unwrap_or_default();
    for (field, value) in patch.as_object().expect("validated model patch") {
        merged.insert(field.clone(), value.clone());
    }
    *patch = Value::Object(merged);
    Ok(())
}

fn backend(env: &dyn ToolEnv) -> ToolResult<Arc<dyn ConfigurationBackend>> {
    env.configuration().ok_or_else(|| {
        ToolError::new(
            crate::error::code::NOT_YET,
            "This host cannot inspect or change this bot's configuration.",
        )
    })
}

pub async fn inspect(env: &dyn ToolEnv, _args: ReadArgs) -> ToolResult<ToolOutput> {
    Ok(ToolOutput::new(backend(env)?.inspect().await?))
}

pub async fn models(env: &dyn ToolEnv, _args: ReadArgs) -> ToolResult<ToolOutput> {
    Ok(ToolOutput::new(backend(env)?.models().await?))
}

pub async fn update(env: &dyn ToolEnv, args: UpdateArgs) -> ToolResult<ToolOutput> {
    validate_changes(&args.changes)?;
    let configuration = backend(env)?.update(args.changes).await?;
    Ok(ToolOutput::new(
        json!({"configuration": configuration, "effective": "next_turn",
        "message": "The approved changes were saved. They take effect on your next turn; this turn keeps its existing capabilities, models and instructions."}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_changes_are_allowlisted_and_typed() {
        for changes in [
            json!({}),
            json!({"bot_id":"other"}),
            json!({"owner_id":"other"}),
            json!({"location":"server"}),
            json!({"capabilities":["made_up"]}),
            json!({"capabilities":["internet","internet"]}),
            json!({"model":{"provider":"evil"}}),
            json!({"name":" "}),
            json!({"instructions":"a".repeat(8_001)}),
            json!({"image_model":42}),
            json!({"model":{"fallbacks":[null]}}),
        ] {
            assert!(validate_changes(&changes).is_err(), "{changes}");
        }
        validate_changes(&json!({"name":"Scout", "description":null, "instructions":"Be concise.", "capabilities":["internet"], "model":{"primary":"chat-a"}, "image_model":"image-a"})).unwrap();
        validate_changes(&json!({"model":{"decision":null,"fallbacks":[]}})).unwrap();
        assert!(
            serde_json::from_value::<UpdateArgs>(json!({"changes":{"name":"A"},"bot_id":"other"}))
                .is_err()
        );
    }

    #[test]
    fn settings_snapshot_does_not_expose_other_stored_fields() {
        assert_eq!(
            snapshot(
                &json!({"name":"A","secrets":{"TOKEN":"secret"},"appearance":{},"owner_id":"owner"})
            ),
            json!({"name":"A"})
        );
    }

    #[test]
    fn partial_model_changes_preserve_other_pins_and_allow_clearing() {
        let mut changes = json!({"model":{"decision":null}});
        merge_model_changes(
            &mut changes,
            &json!({"primary":"chat-a","fallbacks":["chat-b"],"decision":"fast"}),
        )
        .unwrap();
        assert_eq!(
            changes,
            json!({"model":{"primary":"chat-a","fallbacks":["chat-b"],"decision":null}})
        );
    }

    #[test]
    fn only_configuration_updates_ask_and_no_tool_needs_a_capability() {
        for spec in specs() {
            assert_eq!(spec.capability, None);
            assert_eq!(spec.approval == Approval::Ask, requires_approval(spec.name));
            assert_eq!(spec.mutating, requires_approval(spec.name));
            assert!(is_configuration_tool(spec.name));
        }
    }
}
