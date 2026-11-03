use crate::{
    entity::{bot, json_types::StringList},
    routes::profile::deserialize_nullable,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    Value,
    bot::{BotCapability, BotLocation, BotModelPolicy, BotSystemKind},
    data_classification::{DataClassification, DataSensitivity},
};
use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A bot as its owner sees it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Bot {
    /// The bot's id, which is also its sub: records of what the bot did store it.
    pub id: String,
    /// The person the bot acts for.
    pub owner_id: String,
    /// `general` (Assistant) or `flux` for the built-in bots; null for bots you created.
    #[schema(value_type = Option<String>, example = "general")]
    pub system: Option<BotSystemKind>,
    /// The preset the bot was created from.
    pub preset_id: Option<String>,
    /// Null on a built-in bot that was never renamed: show its localized default name.
    pub name: Option<String>,
    pub description: Option<String>,
    /// How the bot should behave, in the owner's words.
    pub instructions: Option<String>,
    /// The look from the bot builder: an object with a string `kind`.
    #[schema(value_type = Object, example = json!({ "kind": "pip" }))]
    pub appearance: Value,
    /// The model the bot prefers and the fallbacks hosts try in order.
    #[schema(value_type = Object, example = json!({ "primary": null, "fallbacks": [] }))]
    pub model: BotModelPolicy,
    /// Bit id of the embedding model the bot's memory uses; null uses the chat's.
    pub embedding_model: Option<String>,
    /// How sensitive the data the bot handles is.
    #[schema(value_type = Object, example = json!({ "classification": "internal", "pii": false }))]
    pub classification: DataSensitivity,
    /// What the bot may do; everything else is off.
    #[schema(value_type = Vec<String>, example = json!(["internet"]))]
    pub capabilities: Vec<BotCapability>,
    /// The profile whose apps `profile_apps` covers.
    pub profile_id: Option<String>,
    /// Where the bot runs: `server`, `desktop` or `device`.
    #[schema(value_type = String, example = "server")]
    pub location: BotLocation,
    /// The managed device a `device` bot runs on.
    pub device_id: Option<String>,
    /// The `local_id` the bot was moved from a device with (`POST /bots/import`); null for bots
    /// made in the account.
    #[schema(example = "bot_tz4a98xxat96iws9zmbrgj3a")]
    pub import_key: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotList {
    /// Assistant, Flux, then the bots you created, oldest first.
    pub bots: Vec<Bot>,
}

/// A new bot. It starts with the capabilities listed here and nothing else.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBotRequest {
    /// Shown wherever the bot acts. 1 to 64 characters.
    pub name: String,
    /// The preset the bot was created from. At most 128 characters.
    pub preset_id: Option<String>,
    /// At most 2,000 characters.
    pub description: Option<String>,
    /// How the bot should behave. At most 8,000 characters.
    pub instructions: Option<String>,
    /// The look from the bot builder: an object with a string `kind`, at most 16 KiB.
    #[schema(value_type = Object, example = json!({ "kind": "pip" }))]
    pub appearance: Value,
    /// Model ids of at most 256 characters; at most 3 fallbacks.
    #[schema(value_type = Option<Object>, example = json!({ "primary": null, "fallbacks": [] }))]
    pub model: Option<BotModelPolicy>,
    /// Bit id of the embedding model the bot's memory uses, at most 256 characters; omitted uses
    /// the chat's.
    pub embedding_model: Option<String>,
    #[schema(value_type = Option<Object>, example = json!({ "classification": "internal", "pii": false }))]
    pub classification: Option<DataSensitivity>,
    /// Capability ids. `computer_use`, `browser_use` and `command_line` need a desktop or
    /// device location; `profile_apps` needs `profile_id`; `flowpilot` comes only with Flux
    /// and its duplicates.
    #[schema(example = json!(["internet"]))]
    pub capabilities: Option<Vec<String>>,
    /// One of your profiles.
    pub profile_id: Option<String>,
    /// `server` (default), `desktop` or `device`.
    #[schema(value_type = Option<String>, example = "server")]
    pub location: Option<BotLocation>,
    /// One of your active managed devices; required for `device` bots.
    pub device_id: Option<String>,
}

/// Changes to a bot. Omitted fields stay as they are; `null` clears a nullable field.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateBotRequest {
    /// `null` resets a built-in bot to its localized default name; bots you created keep one.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub instructions: Option<Option<String>>,
    #[schema(value_type = Option<Object>)]
    pub appearance: Option<Value>,
    #[schema(value_type = Option<Object>)]
    pub model: Option<BotModelPolicy>,
    /// `null` returns the bot's memory to the chat's embedding model.
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub embedding_model: Option<Option<String>>,
    #[schema(value_type = Option<Object>)]
    pub classification: Option<DataSensitivity>,
    /// Replaces the whole set. The capabilities of Flux and of bots with `flowpilot` can't be
    /// changed, and no other bot gets `flowpilot`.
    pub capabilities: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub profile_id: Option<Option<String>>,
    #[schema(value_type = Option<String>, example = "desktop")]
    pub location: Option<BotLocation>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    #[schema(value_type = Option<String>)]
    pub device_id: Option<Option<String>>,
}

/// Everything an owner can change on a bot, validated and normalized.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct BotSettings {
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub appearance: Value,
    pub model: BotModelPolicy,
    pub embedding_model: Option<String>,
    pub classification: DataSensitivity,
    pub capabilities: Vec<BotCapability>,
    pub profile_id: Option<String>,
    pub location: BotLocation,
    pub device_id: Option<String>,
}

impl BotSettings {
    pub fn from_row(row: &bot::Model) -> Self {
        Self {
            name: row.name.clone(),
            description: row.description.clone(),
            instructions: row.instructions.clone(),
            appearance: row.appearance.clone(),
            model: serde_json::from_value(row.model_policy.clone()).unwrap_or_default(),
            embedding_model: row.embedding_model.clone(),
            classification: DataSensitivity {
                classification: row
                    .classification
                    .as_deref()
                    .and_then(DataClassification::parse),
                pii: row.pii,
            },
            capabilities: stored_capabilities(
                &row.capabilities,
                row.system_key.as_deref().and_then(BotSystemKind::parse),
            ),
            profile_id: row.profile_id.clone(),
            location: BotLocation::parse(&row.location).unwrap_or_default(),
            device_id: row.device_id.clone(),
        }
    }

    pub fn has(&self, capability: BotCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Names of the fields `next` changes, for the audit record; never their values.
    pub fn changed_fields(&self, next: &Self) -> Vec<&'static str> {
        [
            ("name", self.name != next.name),
            ("description", self.description != next.description),
            ("instructions", self.instructions != next.instructions),
            ("appearance", self.appearance != next.appearance),
            ("model", self.model != next.model),
            (
                "embedding_model",
                self.embedding_model != next.embedding_model,
            ),
            ("classification", self.classification != next.classification),
            ("capabilities", self.capabilities != next.capabilities),
            ("profile_id", self.profile_id != next.profile_id),
            ("location", self.location != next.location),
            ("device_id", self.device_id != next.device_id),
        ]
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect()
    }

    pub fn write(&self, row: &mut bot::ActiveModel) -> flow_like_types::Result<()> {
        row.name = Set(self.name.clone());
        row.description = Set(self.description.clone());
        row.instructions = Set(self.instructions.clone());
        row.appearance = Set(self.appearance.clone());
        row.model_policy = Set(serde_json::to_value(&self.model)?);
        row.embedding_model = Set(self.embedding_model.clone());
        row.classification = Set(self
            .classification
            .classification
            .map(|level| level.as_str().to_string()));
        row.pii = Set(self.classification.pii);
        row.capabilities = Set(capability_list(&self.capabilities));
        row.profile_id = Set(self.profile_id.clone());
        row.location = Set(location_column(self.location));
        row.device_id = Set(self.device_id.clone());
        Ok(())
    }
}

impl From<bot::Model> for Bot {
    fn from(row: bot::Model) -> Self {
        let settings = BotSettings::from_row(&row);
        Self {
            system: row.system_key.as_deref().and_then(BotSystemKind::parse),
            preset_id: row.preset_id,
            name: settings.name,
            description: settings.description,
            instructions: settings.instructions,
            appearance: settings.appearance,
            model: settings.model,
            embedding_model: settings.embedding_model,
            classification: settings.classification,
            capabilities: settings.capabilities,
            profile_id: settings.profile_id,
            location: settings.location,
            device_id: settings.device_id,
            import_key: row.import_key,
            id: row.id,
            owner_id: row.owner_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Capability ids in their canonical order, without duplicates.
pub(super) fn capability_list(capabilities: &[BotCapability]) -> StringList {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| capabilities.contains(capability))
        .map(|capability| capability.as_str().to_string())
        .collect()
}

/// Ids this server no longer knows are skipped rather than failing the whole bot. A built-in bot
/// also holds what it always holds (Flux: `flowpilot`), whatever its row stores.
fn stored_capabilities(ids: &StringList, system: Option<BotSystemKind>) -> Vec<BotCapability> {
    let fixed = system.map_or(&[][..], BotSystemKind::fixed_capabilities);
    BotCapability::ALL
        .into_iter()
        .filter(|capability| {
            fixed.contains(capability)
                || ids
                    .iter()
                    .any(|id| BotCapability::parse(id) == Some(*capability))
        })
        .collect()
}

pub(super) fn location_column(location: BotLocation) -> String {
    location.as_str().to_ascii_uppercase()
}
