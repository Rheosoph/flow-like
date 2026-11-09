//! Bots: assistants that act on behalf of their owner. These routes accept people only (personal
//! access tokens), not API keys or running flows.

use crate::{Client, Method, RequestOptions, Result, Value, inbox::wire_enum, json};
use serde::{Deserialize, Serialize};
use std::fmt;

wire_enum!(
    /// Something a bot may do. Every capability is off until the owner switches it on.
    BotCapability {
        Internet => "internet",
        /// Every app in one profile; needs `profile_id`.
        ProfileApps => "profile_apps",
        /// The apps attached with [`Client::attach_bot_app`].
        SelectedApps => "selected_apps",
        CodeExecution => "code_execution",
        /// Desktop or device bots only.
        ComputerUse => "computer_use",
        /// Desktop or device bots only.
        BrowserUse => "browser_use",
        Email => "email",
        PushNotifications => "push_notifications",
        Scheduling => "scheduling",
        Storage => "storage",
        DataStudio => "data_studio",
        /// Desktop or device bots only.
        CommandLine => "command_line",
        Encryption => "encryption",
        /// Requests current context from a consenting conversation participant.
        UserContext => "user_context",
        Webhook => "webhook",
    }
);

wire_enum!(
    /// Where a bot's host runs.
    BotLocation {
        Server => "server",
        Desktop => "desktop",
        /// A managed device; needs `device_id`.
        Device => "device",
    }
);

wire_enum!(
    /// The built-in bots every person has and can't delete.
    BotSystemKind {
        /// Assistant.
        General => "general",
        Flux => "flux",
    }
);

wire_enum!(
    /// How sensitive data is, from least to most sensitive.
    DataClassification {
        Public => "public",
        Internal => "internal",
        Confidential => "confidential",
        Restricted => "restricted",
    }
);

wire_enum!(
    /// How a secret is stored.
    BotSecretEncryption {
        /// Encrypted by the server under a key only this bot's secrets use.
        Server => "server",
        /// Sealed to the bot's own key; only the bot's host can open it.
        Sealed => "sealed",
    }
);

/// The model a bot prefers; hosts try `fallbacks` in order where `primary` is unavailable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotModelPolicy {
    /// Bit id of the preferred model; `None` lets the host choose.
    pub primary: Option<String>,
    /// At most 3 bit ids.
    pub fallbacks: Vec<String>,
    /// System One Bit for response decisions; absent disables decision routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

/// How sensitive the data a bot handles is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BotClassification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<DataClassification>,
    pub pii: bool,
}

/// A bot as its owner sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    /// Also the bot's sub: records of what the bot did store it.
    pub id: String,
    pub owner_id: String,
    /// `None` for bots you created.
    pub system: Option<BotSystemKind>,
    pub preset_id: Option<String>,
    /// `None` on a built-in bot that was never renamed: show its localized default name.
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    /// The look from the bot builder: an object with a string `kind`.
    #[serde(default)]
    pub appearance: Value,
    #[serde(default)]
    pub model: BotModelPolicy,
    #[serde(default)]
    pub classification: BotClassification,
    #[serde(default)]
    pub capabilities: Vec<BotCapability>,
    /// The profile whose apps `ProfileApps` covers.
    pub profile_id: Option<String>,
    pub location: BotLocation,
    /// The managed device a device bot runs on.
    pub device_id: Option<String>,
    /// The local id a bot moved from a device was created with.
    pub import_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A new bot. It starts with `capabilities` and nothing else; build it from [`CreateBot::new`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CreateBot {
    /// 1 to 64 characters.
    pub name: String,
    /// The look from the bot builder; [`CreateBot::new`] uses `{"kind": "pip"}`, Assistant's look.
    pub appearance: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<BotModelPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<BotClassification>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<BotCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// `Server` when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<BotLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

impl CreateBot {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            appearance: json!({ "kind": "pip" }),
            preset_id: None,
            description: None,
            instructions: None,
            model: None,
            classification: None,
            capabilities: Vec::new(),
            profile_id: None,
            location: None,
            device_id: None,
        }
    }
}

/// Changes to a bot: `None` leaves a field as it is, `Some(None)` clears a nullable one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UpdateBot {
    /// `Some(None)` resets a built-in bot to its localized default name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Value>,
    /// Replaces the whole policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<BotModelPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<BotClassification>,
    /// Replaces the whole set. Flux's capabilities can't be changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<BotCapability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<BotLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<Option<String>>,
}

/// An app the bot may use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotApp {
    pub app_id: String,
    pub added_at: String,
    /// Your current role in the app, which bounds the bot there; `None` once you left the app.
    pub owner_role: Option<String>,
}

/// A secret's metadata. No response ever carries its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotSecret {
    pub name: String,
    pub encryption: BotSecretEncryption,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// When the bot last read the secret.
    pub last_used_at: Option<String>,
}

/// A secret to store: a value the server encrypts, or an envelope sealed to the bot's key.
/// Without [`SetBotSecret::description`] or [`SetBotSecret::clear_description`] the current
/// description stays.
#[derive(Clone, PartialEq, Serialize)]
pub struct SetBotSecret {
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Option<String>>,
}

impl SetBotSecret {
    /// At most 16 KiB; encrypted before it is stored and never returned.
    pub fn value(value: impl Into<String>) -> Self {
        Self {
            value: Some(value.into()),
            sealed: None,
            description: None,
        }
    }

    /// An envelope sealed to the bot's key: an object with a numeric `v` and an `enc`.
    pub fn sealed(envelope: Value) -> Self {
        Self {
            value: None,
            sealed: Some(envelope),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(Some(description.into()));
        self
    }

    pub fn clear_description(mut self) -> Self {
        self.description = Some(None);
        self
    }
}

impl fmt::Debug for SetBotSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetBotSecret")
            .field("value", &self.value.as_ref().map(|_| "[redacted]"))
            .field("sealed", &self.sealed.is_some())
            .field("description", &self.description)
            .finish()
    }
}

#[derive(Deserialize)]
struct Bots {
    bots: Vec<Bot>,
}

#[derive(Deserialize)]
struct BotApps {
    apps: Vec<BotApp>,
}

#[derive(Deserialize)]
struct BotSecrets {
    secrets: Vec<BotSecret>,
}

impl Client {
    /// Assistant and Flux first (set up on the first call), then your bots, oldest first.
    pub async fn list_bots(&self) -> Result<Vec<Bot>> {
        let list: Bots = self
            .request(Method::GET, &["bots"], &RequestOptions::new())
            .await?;
        Ok(list.bots)
    }

    /// One of your bots; someone else's bot reads as not found.
    pub async fn get_bot(&self, bot_id: &str) -> Result<Bot> {
        self.request(Method::GET, &["bots", bot_id], &RequestOptions::new())
            .await
    }

    pub async fn create_bot(&self, request: &CreateBot) -> Result<Bot> {
        self.request(
            Method::POST,
            &["bots"],
            &RequestOptions::new().json(serde_json::to_value(request)?),
        )
        .await
    }

    pub async fn update_bot(&self, bot_id: &str, changes: &UpdateBot) -> Result<Bot> {
        self.request(
            Method::PATCH,
            &["bots", bot_id],
            &RequestOptions::new().json(serde_json::to_value(changes)?),
        )
        .await
    }

    /// Deletes a bot you created; Assistant and Flux answer 409 `BOT_SYSTEM_UNDELETABLE`.
    pub async fn delete_bot(&self, bot_id: &str) -> Result<()> {
        self.request(Method::DELETE, &["bots", bot_id], &RequestOptions::new())
            .await
    }

    /// The apps attached to the bot, oldest first, each with your current role there.
    pub async fn list_bot_apps(&self, bot_id: &str) -> Result<Vec<BotApp>> {
        let list: BotApps = self
            .request(
                Method::GET,
                &["bots", bot_id, "apps"],
                &RequestOptions::new(),
            )
            .await?;
        Ok(list.apps)
    }

    /// Lets the bot use an app you are a member of, with your role there.
    pub async fn attach_bot_app(&self, bot_id: &str, app_id: &str) -> Result<()> {
        self.request(
            Method::PUT,
            &["bots", bot_id, "apps", app_id],
            &RequestOptions::new(),
        )
        .await
    }

    pub async fn detach_bot_app(&self, bot_id: &str, app_id: &str) -> Result<()> {
        self.request(
            Method::DELETE,
            &["bots", bot_id, "apps", app_id],
            &RequestOptions::new(),
        )
        .await
    }

    /// Names and metadata of the bot's secrets, sorted by name; never their values.
    pub async fn list_bot_secrets(&self, bot_id: &str) -> Result<Vec<BotSecret>> {
        let list: BotSecrets = self
            .request(
                Method::GET,
                &["bots", bot_id, "secrets"],
                &RequestOptions::new(),
            )
            .await?;
        Ok(list.secrets)
    }

    /// Stores or replaces a secret named like an environment variable (`^[A-Z][A-Z0-9_]{0,63}$`).
    pub async fn set_bot_secret(
        &self,
        bot_id: &str,
        name: &str,
        secret: &SetBotSecret,
    ) -> Result<BotSecret> {
        self.request(
            Method::PUT,
            &["bots", bot_id, "secrets", name],
            &RequestOptions::new().json(serde_json::to_value(secret)?),
        )
        .await
    }

    pub async fn delete_bot_secret(&self, bot_id: &str, name: &str) -> Result<()> {
        self.request(
            Method::DELETE,
            &["bots", bot_id, "secrets", name],
            &RequestOptions::new(),
        )
        .await
    }
}
