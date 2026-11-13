//! Flux manages its owner's other bots: it lists and reads them, creates new ones, changes
//! their settings, places them on the owner's devices and deletes them. Hosts bind the backend
//! to the owner, so a call can never reach another person's bots. Creating, changing or deleting
//! a bot always needs the owner's explicit approval.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use flow_like_types::{
    Result,
    bot::{BotCapability, BotLocation},
    bot_restrictions::{
        BotFileScope, BotPermission, BotPreset, BotRestrictions, BotScopes, MAX_SCOPE_ENTRIES,
        MAX_SCOPE_ENTRY_LEN,
    },
    data_classification::DataClassification,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    ToolEnv, ToolError, ToolOutput, ToolResult,
    configuration::{merge_model_changes, settings_properties, snapshot, validate_changes},
    error::code,
    spec::{Approval, ToolSpec, schema_of},
};

pub const LIST_BOTS: &str = "list_bots";
pub const GET_BOT: &str = "get_bot";
pub const CREATE_BOT: &str = "create_bot";
pub const UPDATE_BOT: &str = "update_bot";
pub const DELETE_BOT: &str = "delete_bot";
pub const LIST_BOT_DEVICES: &str = "list_bot_devices";

/// The characters every owner has, so a bot Flux creates never needs a pack.
pub const STARTER_LOOKS: [&str; 5] = ["pip", "mochi", "hexa", "pebble", "nimbus"];
const LOCATIONS: [&str; 3] = ["server", "desktop", "device"];
const MAX_APPS: usize = 50;
const MAX_APP_ID_CHARS: usize = 128;
const MAX_DEVICE_ID_CHARS: usize = 256;
/// A device's bot host counts as online this long after its last poll, as the Bots tab shows it.
const BOT_HOST_FRESH_MS: i64 = 3 * 60_000;
/// Fields that are not bot settings: the look, the placement, the data classification and the
/// apps.
const CREW_FIELDS: [&str; 6] = [
    "look",
    "location",
    "device_id",
    "classification",
    "app_ids",
    "app_restrictions",
];

pub fn is_crew_tool(name: &str) -> bool {
    matches!(
        name,
        LIST_BOTS | GET_BOT | CREATE_BOT | UPDATE_BOT | DELETE_BOT | LIST_BOT_DEVICES
    )
}

pub fn requires_approval(name: &str) -> bool {
    matches!(name, CREATE_BOT | UPDATE_BOT | DELETE_BOT)
}

/// An app attached to a bot and what the bot may do there.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct CrewApp {
    pub app_id: String,
    #[serde(default)]
    pub restrictions: BotRestrictions,
}

/// The owner's bots as the API's `Bot`. Every method runs the same server function or local
/// store call as the Bots tab. The host must obtain approval for the exact request before
/// [`CrewBackend::create`], [`CrewBackend::update`], [`CrewBackend::restrict`] or
/// [`CrewBackend::delete`].
#[async_trait]
pub trait CrewBackend: Send + Sync {
    async fn bots(&self) -> Result<Vec<Value>>;
    /// One of the owner's bots with the apps attached to it.
    async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<CrewApp>)>;
    /// The profile `profile_apps` covers on a new bot; `None` where the host knows none.
    fn profile_id(&self) -> Option<String>;
    /// Creates the bot from an API `CreateBotRequest`, then attaches `app_ids`.
    async fn create(&self, request: Value, app_ids: &[String]) -> Result<Value>;
    /// Applies an API `UpdateBotRequest` when there is one, then attaches and detaches apps.
    async fn update(
        &self,
        bot_id: &str,
        request: Option<Value>,
        attach: &[String],
        detach: &[String],
    ) -> Result<Value>;
    /// Replaces what the bot may do in an app attached to it.
    async fn restrict(
        &self,
        bot_id: &str,
        app_id: &str,
        restrictions: &BotRestrictions,
    ) -> Result<()>;
    /// Deletes the bot with its data and retires it from its conversations.
    async fn delete(&self, bot_id: &str) -> Result<()>;
    /// The devices the owner sees, as the API's `DeviceView`; empty without an account.
    async fn devices(&self) -> Result<Vec<Value>>;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetArgs {
    /// The bot's id from list_bots.
    pub bot_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteArgs {
    /// The id from list_bots of the bot to delete.
    pub bot_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    pub bot_id: String,
    pub changes: Value,
}

/// One `app_restrictions` entry: a preset or exact permissions, and the scopes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestrictionEntry {
    app_id: String,
    preset: Option<BotPreset>,
    permissions: Option<Vec<BotPermission>>,
    folders: Option<Vec<String>>,
    #[serde(default)]
    files_read_only: bool,
    tables: Option<Vec<String>>,
    events: Option<Vec<String>>,
    boards: Option<Vec<String>>,
}

fn assignable_capabilities() -> Vec<&'static str> {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| !capability.is_fixed())
        .map(BotCapability::as_str)
        .collect()
}

fn wire_ids<T: Serialize>(values: impl IntoIterator<Item = T>) -> Vec<Value> {
    values
        .into_iter()
        .filter_map(|value| serde_json::to_value(value).ok())
        .collect()
}

fn classification_property() -> Value {
    let mut levels: Vec<Value> = DataClassification::names()
        .into_iter()
        .map(Value::String)
        .collect();
    levels.push(Value::Null);
    json!({"type": "object", "additionalProperties": false, "minProperties": 1,
    "description": "How sensitive the data the bot handles is: classification (null clears it) and pii (it handles personal data). On update_bot an omitted field keeps its value.",
    "properties": {
        "classification": {"type": ["string", "null"], "enum": levels},
        "pii": {"type": "boolean"},
    }})
}

fn restrictions_property() -> Value {
    let list = |what: &str| {
        json!({"type": "array", "maxItems": MAX_SCOPE_ENTRIES, "uniqueItems": true, "description": what,
            "items": {"type": "string", "minLength": 1, "maxLength": MAX_SCOPE_ENTRY_LEN}})
    };
    json!({"type": "array", "maxItems": MAX_APPS,
    "description": "What the bot may do in apps attached to it. Each entry replaces that app's restrictions; apps left out keep theirs. Without preset or permissions the bot keeps the owner's whole role there, narrowed by the scopes given. The owner's own role in the app is always the upper bound.",
    "items": {"type": "object", "additionalProperties": false, "required": ["app_id"], "properties": {
        "app_id": {"type": "string", "minLength": 1, "maxLength": MAX_APP_ID_CHARS},
        "preset": {"type": "string", "enum": wire_ids(BotPreset::ALL), "description": "read_only: looks at files, tables, boards and events and changes nothing; read_and_call_events: also calls the app's events; full: the owner's whole role."},
        "permissions": {"type": "array", "uniqueItems": true, "description": "Instead of a preset: exactly the permissions the bot may hold.",
            "items": {"type": "string", "enum": wire_ids(BotPermission::ALL)}},
        "folders": list("Only these folders of the app's files, such as reports/2026; omit for every folder."),
        "files_read_only": {"type": "boolean", "description": "Never change files, whatever the permissions allow."},
        "tables": list("Only these table names; omit for every table, [] for none."),
        "events": list("Only these event ids; omit for every event, [] for none."),
        "boards": list("Only these board ids; omit for every board, [] for none."),
    }}})
}

fn crew_properties(properties: &mut Map<String, Value>, app_ids: &str, location: &str) {
    properties.insert(
        "look".into(),
        json!({"type": "string", "enum": STARTER_LOOKS, "description": "The character the bot appears as. Omitted on a new bot: one is picked from its name."}),
    );
    properties.insert(
        "app_ids".into(),
        json!({"type": "array", "maxItems": MAX_APPS, "uniqueItems": true, "description": app_ids,
            "items": {"type": "string", "minLength": 1, "maxLength": MAX_APP_ID_CHARS}}),
    );
    properties.insert("classification".into(), classification_property());
    properties.insert(
        "location".into(),
        json!({"type": "string", "enum": LOCATIONS, "description": location}),
    );
    properties.insert(
        "device_id".into(),
        json!({"type": "string", "minLength": 1, "maxLength": MAX_DEVICE_ID_CHARS, "description": "With location device: a device_id from list_bot_devices."}),
    );
}

fn create_schema() -> Value {
    let mut properties = settings_properties(&assignable_capabilities());
    crew_properties(
        &mut properties,
        "Apps from list_apps the bot may use with the owner's role there. Needs the selected_apps capability.",
        "Where the bot runs: server (default when signed in; works while the owner is offline), desktop (this computer, needed for computer_use, browser_use, command_line and encryption unless a device runs it) or device (one of the owner's devices from list_bot_devices, with device_id). Signed out, bots always run on this desktop.",
    );
    properties.insert(
        "name".into(),
        json!({"type": "string", "minLength": 1, "maxLength": 64}),
    );
    json!({"type": "object", "additionalProperties": false, "required": ["name"], "properties": properties})
}

fn update_schema() -> Value {
    let mut properties = settings_properties(&assignable_capabilities());
    crew_properties(
        &mut properties,
        "The complete list of apps the bot may use. Read get_bot first and keep the apps the owner wants to keep; apps left out are detached.",
        "Moves the bot: server, desktop or device (with device_id from list_bot_devices). Send capabilities without the ones its new host can't hold.",
    );
    properties.insert("app_restrictions".into(), restrictions_property());
    json!({"type": "object", "additionalProperties": false, "required": ["bot_id", "changes"], "properties": {
        "bot_id": {"type": "string", "minLength": 1, "description": "The bot's id from list_bots."},
        "changes": {"type": "object", "additionalProperties": false, "minProperties": 1, "properties": properties},
    }})
}

const CREATE_DESCRIPTION: &str = "Create a new bot for the owner. A bot is a persona that chats in its own conversation and in rooms, acts with the owner's rights and holds only the capabilities you switch on. Give it a short name, a one-line description and instructions (its system prompt: role, tone, what to do and what to avoid). Switch on only the capabilities the purpose needs: internet (web research), web_requests (direct API and URL calls), memory, scheduling (one-time and recurring tasks), push_notifications, email, selected_apps (with app_ids) or profile_apps (every app of the current profile), storage, data_studio, documents, barcodes (QR codes and barcodes in images), knowledge, code_execution, image_generation, skills, connectors, channels, delegation, user_context, webhook; computer_use, browser_use, command_line and encryption need location desktop or device. To run it on one of the owner's devices, call list_bot_devices and use location device with that device_id. Set classification when the bot handles confidential or personal data. Use bot_models for exact model ids; omit models to use the owner's defaults. Building apps, workflows and pages stays your work: build the app first, then give the bot that app. Every call asks the owner, including in auto mode.";

const UPDATE_DESCRIPTION: &str = "Change another bot's settings: name, description, instructions (system prompt), capabilities, models, look, data classification, where it runs (location, device_id), apps (app_ids) or what it may do in each app (app_restrictions). Read get_bot first. Send only the fields to change; null clears a nullable field. capabilities and app_ids replace the full list; model and classification fields are patched individually; each app_restrictions entry replaces that app's restrictions. Flux itself can't be moved. Flux's own capabilities are fixed; change your own name, description, instructions or models with bot_update_configuration. Every call asks the owner, including in auto mode. Changes apply from the bot's next turn.";

const DELETE_DESCRIPTION: &str = "Delete one of the owner's bots for good: its settings, app access, secrets and its workspace with files and tables go; its past messages stay and show it as a deleted bot. Only when the owner asked for it. Read list_bots first and name the bot you delete. Assistant, Flux and you yourself can't be deleted. Every call asks the owner, including in auto mode.";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: LIST_BOTS,
            description: "List the owner's bots: id, name, description, built-in kind, capabilities, location, device and look. Use it before creating a bot that may already exist and to find the bot_id for get_bot, update_bot and delete_bot.",
            capability: None,
            module: "crew",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ListArgs>(),
        },
        ToolSpec {
            name: GET_BOT,
            description: "Read one of the owner's bots in full: instructions, capabilities, model mappings, data classification, location and device, look, the ids of the apps it may use and its restrictions in each (an app missing from app_restrictions allows the owner's whole role). Read it before update_bot.",
            capability: None,
            module: "crew",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<GetArgs>(),
        },
        ToolSpec {
            name: CREATE_BOT,
            description: CREATE_DESCRIPTION,
            capability: None,
            module: "crew",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: create_schema(),
        },
        ToolSpec {
            name: UPDATE_BOT,
            description: UPDATE_DESCRIPTION,
            capability: None,
            module: "crew",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: update_schema(),
        },
        ToolSpec {
            name: DELETE_BOT,
            description: DELETE_DESCRIPTION,
            capability: None,
            module: "crew",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<DeleteArgs>(),
        },
        ToolSpec {
            name: LIST_BOT_DEVICES,
            description: "List the owner's devices a bot can run on: device_id, name, whether the device's bot host is online, and the capabilities a bot there can't hold. Call it before placing a bot on a device with create_bot or update_bot (location device). Empty without an account.",
            capability: None,
            module: "crew",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ListArgs>(),
        },
    ]
}

fn look(value: &Value) -> ToolResult<&str> {
    value
        .as_str()
        .filter(|look| STARTER_LOOKS.contains(look))
        .ok_or_else(|| {
            ToolError::invalid(format!("look must be one of {}", STARTER_LOOKS.join(", ")))
        })
}

fn app_id(value: &str) -> ToolResult<String> {
    Some(value.trim())
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_APP_ID_CHARS)
        .map(str::to_string)
        .ok_or_else(|| ToolError::invalid("Each app id must be an id from list_apps"))
}

fn app_ids(value: &Value) -> ToolResult<Vec<String>> {
    let ids = value
        .as_array()
        .filter(|ids| ids.len() <= MAX_APPS)
        .ok_or_else(|| ToolError::invalid(format!("app_ids must list at most {MAX_APPS} apps")))?;
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(|id| {
            let id = app_id(id.as_str().unwrap_or_default())?;
            if !seen.insert(id.clone()) {
                return Err(ToolError::invalid(format!("App '{id}' is listed twice")));
            }
            Ok(id)
        })
        .collect()
}

/// A folder as the Bots tab stores it: no leading or trailing slash.
fn folder(value: String) -> String {
    value.trim().trim_matches('/').to_string()
}

/// The validated `app_restrictions` of an `update_bot` call, by app.
fn restriction_entries(value: &Value) -> ToolResult<Vec<(String, BotRestrictions)>> {
    let entries: Vec<RestrictionEntry> = serde_json::from_value(value.clone())
        .map_err(|error| ToolError::invalid(format!("Bad app_restrictions: {error}")))?;
    if entries.len() > MAX_APPS {
        return Err(ToolError::invalid(format!(
            "app_restrictions may name at most {MAX_APPS} apps"
        )));
    }
    let mut seen = BTreeSet::new();
    entries
        .into_iter()
        .map(|entry| {
            let app_id = app_id(&entry.app_id)?;
            if !seen.insert(app_id.clone()) {
                return Err(ToolError::invalid(format!(
                    "App '{app_id}' is restricted twice"
                )));
            }
            let permissions = match (entry.preset, entry.permissions) {
                (Some(_), Some(_)) => {
                    return Err(ToolError::invalid(format!(
                        "App '{app_id}': give a preset or permissions, not both"
                    )));
                }
                (Some(preset), None) => preset.restrictions().permissions,
                (None, permissions) => permissions,
            };
            let restrictions = BotRestrictions {
                permissions,
                scopes: BotScopes {
                    files: BotFileScope {
                        prefixes: entry
                            .folders
                            .unwrap_or_default()
                            .into_iter()
                            .map(folder)
                            .collect(),
                        read_only: entry.files_read_only,
                    },
                    tables: entry.tables,
                    events: entry.events,
                    boards: entry.boards,
                },
            };
            restrictions
                .validate()
                .map_err(|error| ToolError::invalid(format!("App '{app_id}': {error}")))?;
            Ok((app_id, restrictions))
        })
        .collect()
}

/// One field of a `classification` change as stored: the level (null clears it) or `pii`.
fn classification_field(key: &str, value: &Value) -> ToolResult<Value> {
    match key {
        "classification" if value.is_null() => Ok(Value::Null),
        "classification" => value
            .as_str()
            .and_then(DataClassification::parse)
            .map(|level| json!(level.as_str()))
            .ok_or_else(|| {
                ToolError::invalid(format!(
                    "classification must be one of {} or null",
                    DataClassification::names().join(", ")
                ))
            }),
        "pii" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| ToolError::invalid("pii must be true or false")),
        other => Err(ToolError::invalid(format!(
            "classification has no field '{other}'; use classification and pii"
        ))),
    }
}

/// The data classification a call sets, merged onto the bot's stored value; `current` is null
/// for a new bot.
fn classification(value: &Value, current: &Value) -> ToolResult<Value> {
    let fields = value
        .as_object()
        .filter(|fields| !fields.is_empty())
        .ok_or_else(|| {
            ToolError::invalid("classification must be an object with classification and/or pii")
        })?;
    let mut merged = current.as_object().cloned().unwrap_or_default();
    for (key, value) in fields {
        merged.insert(key.clone(), classification_field(key, value)?);
    }
    let mut out = Map::new();
    if let Some(level) = merged
        .get("classification")
        .filter(|level| !level.is_null())
    {
        out.insert("classification".into(), level.clone());
    }
    out.insert(
        "pii".into(),
        json!(merged.get("pii").and_then(Value::as_bool).unwrap_or(false)),
    );
    Ok(Value::Object(out))
}

fn location(value: &Value) -> ToolResult<BotLocation> {
    value
        .as_str()
        .filter(|id| LOCATIONS.contains(id))
        .and_then(BotLocation::parse)
        .ok_or_else(|| ToolError::invalid("location must be server, desktop or device"))
}

fn device_id(value: &Value) -> ToolResult<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_DEVICE_ID_CHARS)
        .map(str::to_string)
        .ok_or_else(|| ToolError::invalid("device_id must be a device_id from list_bot_devices"))
}

/// The location and device a call names.
fn placement(crew: &Map<String, Value>) -> ToolResult<(Option<BotLocation>, Option<String>)> {
    let location = crew.get("location").map(location).transpose()?;
    let device_id = crew.get("device_id").map(device_id).transpose()?;
    if location == Some(BotLocation::Device) && device_id.is_none() {
        return Err(ToolError::invalid(
            "location device needs a device_id from list_bot_devices",
        ));
    }
    if location.is_some_and(|location| location != BotLocation::Device) && device_id.is_some() {
        return Err(ToolError::invalid("device_id needs location device"));
    }
    Ok((location, device_id))
}

fn capability_ids(capabilities: Option<&Value>) -> Vec<&str> {
    capabilities
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Refuses capabilities a host at `location` can't hold, with the sentence the model passes on.
fn refuse_unhosted(location: BotLocation, capabilities: &[&str]) -> ToolResult<()> {
    let refused: Vec<&str> = capabilities
        .iter()
        .copied()
        .filter(|id| BotCapability::parse(id).is_some_and(|held| !location.allows(held)))
        .collect();
    if refused.is_empty() {
        return Ok(());
    }
    Err(ToolError::invalid(format!(
        "A bot on the {} can't hold {}; leave {} out or choose another location",
        location.as_str(),
        refused.join(", "),
        if refused.len() == 1 { "it" } else { "them" }
    )))
}

/// The capabilities from list_bot_devices a device can't host.
fn unhosted_on_device() -> Vec<&'static str> {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| !capability.is_fixed() && !BotLocation::Device.allows(*capability))
        .map(BotCapability::as_str)
        .collect()
}

/// Splits crew fields off and validates the remaining settings like a configuration change.
fn split(fields: &Map<String, Value>) -> ToolResult<(Map<String, Value>, Map<String, Value>)> {
    let (crew, settings): (Map<_, _>, Map<_, _>) = fields
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .partition(|(key, _)| CREW_FIELDS.contains(&key.as_str()));
    if !settings.is_empty() {
        validate_changes(&Value::Object(settings.clone()))?;
    }
    if let Some(capabilities) = settings.get("capabilities").and_then(Value::as_array)
        && let Some(fixed) = capabilities
            .iter()
            .filter_map(Value::as_str)
            .find(|id| BotCapability::parse(id).is_some_and(BotCapability::is_fixed))
    {
        return Err(ToolError::invalid(format!(
            "Only Flux and its duplicates hold '{fixed}'; leave it out"
        )));
    }
    if let Some(value) = crew.get("look") {
        look(value)?;
    }
    if let Some(value) = crew.get("app_ids") {
        app_ids(value)?;
    }
    if let Some(value) = crew.get("classification") {
        classification(value, &Value::Null)?;
    }
    if let Some(value) = crew.get("app_restrictions") {
        restriction_entries(value)?;
    }
    Ok((settings, crew))
}

/// A new bot's look when none was chosen: stable for its name, so a retry keeps it.
fn default_look(name: &str) -> &'static str {
    let sum = name.bytes().fold(0usize, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(byte as usize)
    });
    STARTER_LOOKS[sum % STARTER_LOOKS.len()]
}

struct CreateFields {
    name: String,
    settings: Map<String, Value>,
    crew: Map<String, Value>,
}

fn create_fields(args: &Value) -> ToolResult<CreateFields> {
    let fields = args
        .as_object()
        .ok_or_else(|| ToolError::invalid("create_bot expects an object"))?;
    let name = fields
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ToolError::invalid("create_bot needs a name"))?
        .to_string();
    let (settings, crew) = split(fields)?;
    if crew.contains_key("app_restrictions") {
        return Err(ToolError::invalid(
            "Restrict apps with update_bot once the bot exists",
        ));
    }
    let (location, device_id) = placement(&crew)?;
    if device_id.is_some() && location != Some(BotLocation::Device) {
        return Err(ToolError::invalid("device_id needs location device"));
    }
    if let Some(location) = location {
        refuse_unhosted(location, &capability_ids(settings.get("capabilities")))?;
    }
    let apps = crew
        .get("app_ids")
        .map(app_ids)
        .transpose()?
        .unwrap_or_default();
    require_selected_apps(&apps, settings.get("capabilities"))?;
    Ok(CreateFields {
        name,
        settings,
        crew,
    })
}

/// Refuses malformed crew calls before any approval is asked. Ownership, limits and model ids
/// are the host's to check; [`precheck`] reads the target bot where a call needs it.
pub fn validate_call(name: &str, args: &Value) -> ToolResult<()> {
    match name {
        CREATE_BOT => create_fields(args).map(|_| ()),
        UPDATE_BOT => {
            let args: UpdateArgs = serde_json::from_value(args.clone()).map_err(|error| {
                ToolError::invalid(format!("Bad arguments for update_bot: {error}"))
            })?;
            if args.bot_id.trim().is_empty() {
                return Err(ToolError::invalid(
                    "update_bot needs a bot_id from list_bots",
                ));
            }
            update_request(&args.changes, &json!({})).map(|_| ())
        }
        DELETE_BOT => serde_json::from_value::<DeleteArgs>(args.clone())
            .map_err(|error| ToolError::invalid(format!("Bad arguments for delete_bot: {error}")))
            .and_then(|args| {
                if args.bot_id.trim().is_empty() {
                    return Err(ToolError::invalid(
                        "delete_bot needs a bot_id from list_bots",
                    ));
                }
                Ok(())
            }),
        LIST_BOTS | LIST_BOT_DEVICES => serde_json::from_value::<ListArgs>(args.clone())
            .map(|_| ())
            .map_err(|error| ToolError::invalid(format!("Bad arguments for {name}: {error}"))),
        GET_BOT => serde_json::from_value::<GetArgs>(args.clone())
            .map(|_| ())
            .map_err(|error| ToolError::invalid(format!("Bad arguments for get_bot: {error}"))),
        _ => Ok(()),
    }
}

fn target_id(args: &Value) -> &str {
    args.get("bot_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

/// Refuses a call on the bot that makes it: no bot deletes itself.
pub fn refuse_caller(name: &str, args: &Value, caller_bot_id: &str) -> ToolResult<()> {
    if name == DELETE_BOT && target_id(args) == caller_bot_id.trim() {
        return Err(ToolError::refused(
            "You can't delete yourself; only the owner can, in the Bots tab",
        ));
    }
    Ok(())
}

/// Whether [`precheck`] reads the target bot for this call: a delete or a move.
pub fn reads_target_first(name: &str, args: &Value) -> bool {
    match name {
        DELETE_BOT => true,
        UPDATE_BOT => args
            .get("changes")
            .and_then(Value::as_object)
            .is_some_and(|changes| {
                changes.contains_key("location") || changes.contains_key("device_id")
            }),
        _ => false,
    }
}

/// Checks a call against the target bot as it is now, before the owner is asked: a built-in bot
/// is never deleted, Flux never moves and a moved bot keeps only what its new host can hold.
/// Hosts run it ahead of the approval card; the tool checks the same again when it runs.
pub async fn precheck(backend: &dyn CrewBackend, name: &str, args: &Value) -> ToolResult<()> {
    if !reads_target_first(name, args) {
        return Ok(());
    }
    let (bot, _) = backend.bot(target_id(args)).await?;
    match name {
        DELETE_BOT => refuse_built_in(&bot),
        _ => update_request(args.get("changes").unwrap_or(&Value::Null), &bot).map(|_| ()),
    }
}

/// Writes the classification a call sets, merged onto the `stored` one.
fn insert_classification(
    request: &mut Map<String, Value>,
    crew: &Map<String, Value>,
    stored: &Value,
) -> ToolResult<()> {
    if let Some(value) = crew.get("classification") {
        request.insert("classification".into(), classification(value, stored)?);
    }
    Ok(())
}

/// Names the active profile on a new bot with `profile_apps`, which covers that profile's apps.
fn insert_profile(request: &mut Map<String, Value>, profile_id: Option<String>) -> ToolResult<()> {
    let wants_profile = request
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some("profile_apps")));
    if !wants_profile {
        return Ok(());
    }
    let profile_id = profile_id.ok_or_else(|| {
        ToolError::invalid(
            "profile_apps needs an active profile; use selected_apps with app_ids instead",
        )
    })?;
    request.insert("profile_id".into(), Value::String(profile_id));
    Ok(())
}

/// Writes a placement the way the Bots tab saves one: a bot that leaves a device drops it.
fn write_placement(
    request: &mut Map<String, Value>,
    location: Option<BotLocation>,
    device_id: Option<String>,
) {
    if let Some(location) = location {
        request.insert("location".into(), json!(location.as_str()));
        if location != BotLocation::Device {
            request.insert("device_id".into(), Value::Null);
        }
    }
    if let Some(device_id) = device_id {
        request.insert("device_id".into(), Value::String(device_id));
    }
}

/// The API `CreateBotRequest` and the apps to attach for validated `create_bot` arguments.
pub fn create_request(
    args: &Value,
    profile_id: Option<String>,
) -> ToolResult<(Value, Vec<String>)> {
    let CreateFields {
        name,
        settings,
        crew,
    } = create_fields(args)?;
    let mut request: Map<String, Value> = settings
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect();
    request.insert("name".into(), Value::String(name.clone()));
    let kind = crew
        .get("look")
        .map(look)
        .transpose()?
        .unwrap_or_else(|| default_look(&name));
    request.insert("appearance".into(), json!({ "kind": kind }));
    let (location, device_id) = placement(&crew)?;
    write_placement(&mut request, location, device_id);
    insert_classification(&mut request, &crew, &Value::Null)?;
    insert_profile(&mut request, profile_id)?;
    let apps = crew
        .get("app_ids")
        .map(app_ids)
        .transpose()?
        .unwrap_or_default();
    Ok((Value::Object(request), apps))
}

fn require_selected_apps(apps: &[String], capabilities: Option<&Value>) -> ToolResult<()> {
    let selected = capabilities
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some("selected_apps")));
    if apps.is_empty() || selected {
        return Ok(());
    }
    Err(ToolError::invalid(
        "app_ids need the selected_apps capability; add it to capabilities",
    ))
}

/// Refuses a move the Bots tab would not save: Flux moving, a device id for a bot that won't run
/// on a device, or capabilities the new host can't hold. `current` is `{}` before the bot was
/// read; the checks that need it wait for the read.
fn check_move(
    location: Option<BotLocation>,
    to_device: bool,
    request: &Map<String, Value>,
    current: &Value,
) -> ToolResult<()> {
    if current.get("system").and_then(Value::as_str) == Some("flux") {
        return Err(ToolError::invalid(
            "Flux runs where it was set up and can't be moved",
        ));
    }
    let stored = || {
        current
            .get("location")
            .and_then(Value::as_str)
            .and_then(BotLocation::parse)
    };
    let Some(target) = location.or_else(stored) else {
        return Ok(());
    };
    if to_device && target != BotLocation::Device {
        return Err(ToolError::invalid(
            "device_id needs location device; the bot doesn't run on a device",
        ));
    }
    let capabilities = request
        .get("capabilities")
        .or_else(|| current.get("capabilities"));
    refuse_unhosted(target, &capability_ids(capabilities))
}

/// Writes a move into `request` the way the Bots tab saves one.
fn move_request(
    crew: &Map<String, Value>,
    request: &mut Map<String, Value>,
    current: &Value,
) -> ToolResult<()> {
    let (location, device_id) = placement(crew)?;
    if location.is_none() && device_id.is_none() {
        return Ok(());
    }
    check_move(location, device_id.is_some(), request, current)?;
    write_placement(request, location, device_id);
    Ok(())
}

/// The bot's look with its character swapped, keeping everything else of its appearance.
fn appearance_with(value: &Value, current: &Value) -> ToolResult<Value> {
    let mut appearance = current
        .get("appearance")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    appearance.insert("kind".into(), Value::String(look(value)?.to_string()));
    Ok(Value::Object(appearance))
}

/// Writes the look, the data classification and a move into an `UpdateBotRequest`, each merged
/// onto the bot's current value where the Bots tab keeps the rest.
fn crew_changes(
    crew: &Map<String, Value>,
    request: &mut Map<String, Value>,
    current: &Value,
) -> ToolResult<()> {
    if let Some(value) = crew.get("look") {
        request.insert("appearance".into(), appearance_with(value, current)?);
    }
    let stored = current.get("classification").unwrap_or(&Value::Null);
    insert_classification(request, crew, stored)?;
    move_request(crew, request, current)
}

/// The API `UpdateBotRequest` for validated `update_bot` changes against the bot's current
/// settings; `None` when the changes only move or restrict apps.
pub fn update_request(changes: &Value, current: &Value) -> ToolResult<Option<Value>> {
    let fields = changes
        .as_object()
        .filter(|fields| !fields.is_empty())
        .ok_or_else(|| ToolError::invalid("changes must be a non-empty object"))?;
    let (settings, crew) = split(fields)?;
    let mut request = Value::Object(settings);
    if request.as_object().is_some_and(|fields| !fields.is_empty()) {
        merge_model_changes(&mut request, current.get("model").unwrap_or(&Value::Null))?;
    }
    let fields = request.as_object_mut().expect("settings are an object");
    crew_changes(&crew, fields, current)?;
    Ok((!fields.is_empty()).then_some(request))
}

/// The apps to attach and detach so the bot uses exactly `wanted`.
pub fn app_changes(wanted: &[String], current: &[String]) -> (Vec<String>, Vec<String>) {
    let attach = wanted
        .iter()
        .filter(|id| !current.contains(id))
        .cloned()
        .collect();
    let detach = current
        .iter()
        .filter(|id| !wanted.contains(id))
        .cloned()
        .collect();
    (attach, detach)
}

fn display_name(bot: &Value) -> Value {
    if let Some(name) = bot.get("name").and_then(Value::as_str) {
        return Value::String(name.to_string());
    }
    match bot.get("system").and_then(Value::as_str) {
        Some("general") => json!("Assistant"),
        Some("flux") => json!("Flux"),
        _ => Value::Null,
    }
}

fn bot_look(bot: &Value) -> Value {
    bot.pointer("/appearance/kind")
        .cloned()
        .unwrap_or(Value::Null)
}

fn refuse_built_in(bot: &Value) -> ToolResult<()> {
    if bot.get("system").is_none_or(Value::is_null) {
        return Ok(());
    }
    let name = display_name(bot);
    Err(ToolError::refused(format!(
        "{} is built in and can't be deleted",
        name.as_str().unwrap_or("This bot")
    )))
}

/// One line per bot in `list_bots`: never instructions, credentials or stored metadata.
pub fn summary(bot: &Value) -> Value {
    let mut out = Map::new();
    for field in [
        "id",
        "description",
        "system",
        "capabilities",
        "location",
        "device_id",
    ] {
        if let Some(value) = bot.get(field) {
            out.insert(field.into(), value.clone());
        }
    }
    out.insert("name".into(), display_name(bot));
    out.insert("look".into(), bot_look(bot));
    Value::Object(out)
}

/// An app's restrictions in the shape `app_restrictions` takes; `None` for the owner's whole role.
fn restriction_view(app: &CrewApp) -> Option<Value> {
    let restrictions = &app.restrictions;
    if restrictions.is_unrestricted() {
        return None;
    }
    let mut view = Map::new();
    view.insert("app_id".into(), json!(app.app_id));
    if let Some(permissions) = &restrictions.permissions {
        let preset = BotRestrictions {
            permissions: Some(permissions.clone()),
            scopes: BotScopes::default(),
        }
        .preset();
        match preset {
            Some(preset) => view.insert("preset".into(), json!(preset)),
            None => view.insert("permissions".into(), json!(permissions)),
        };
    }
    let files = &restrictions.scopes.files;
    if !files.prefixes.is_empty() {
        view.insert("folders".into(), json!(files.prefixes));
    }
    if files.read_only {
        view.insert("files_read_only".into(), json!(true));
    }
    for (key, list) in [
        ("tables", &restrictions.scopes.tables),
        ("events", &restrictions.scopes.events),
        ("boards", &restrictions.scopes.boards),
    ] {
        if let Some(list) = list {
            view.insert(key.into(), json!(list));
        }
    }
    Some(Value::Object(view))
}

/// A bot in full for `get_bot` and after a change: its settings, look, placement,
/// classification, attached apps and the restrictions in each.
pub fn detail(bot: &Value, apps: &[CrewApp]) -> Value {
    let mut out = snapshot(bot);
    if let Some(fields) = out.as_object_mut() {
        fields.insert("name".into(), display_name(bot));
        fields.insert("look".into(), bot_look(bot));
        for field in ["classification", "device_id"] {
            if let Some(value) = bot.get(field) {
                fields.insert(field.into(), value.clone());
            }
        }
        fields.insert(
            "app_ids".into(),
            json!(apps.iter().map(|app| &app.app_id).collect::<Vec<_>>()),
        );
        fields.insert(
            "app_restrictions".into(),
            Value::Array(apps.iter().filter_map(restriction_view).collect()),
        );
    }
    out
}

/// One device in `list_bot_devices`, from a `DeviceView`: only the owner's active devices.
pub fn device_summary(device: &Value, now_ms: i64) -> Option<Value> {
    let owned = device
        .get("relationship")
        .and_then(Value::as_str)
        .is_none_or(|relationship| relationship == "owner");
    if !owned || device.get("status").and_then(Value::as_str) != Some("active") {
        return None;
    }
    let device_id = device.get("device_id").and_then(Value::as_str)?;
    let name = ["display_name", "name"]
        .into_iter()
        .filter_map(|field| device.get(field).and_then(Value::as_str))
        .find(|name| !name.trim().is_empty())
        .unwrap_or(device_id);
    let bot_host = match device.get("bot_host_seen_at").and_then(Value::as_i64) {
        None => "never",
        Some(seen) if now_ms - seen <= BOT_HOST_FRESH_MS => "online",
        Some(_) => "offline",
    };
    Some(json!({
        "device_id": device_id,
        "name": name,
        "bot_host": bot_host,
        "cannot_host": unhosted_on_device(),
    }))
}

/// What an approval card shows: the new bot's settings, the changes to one with its `bot_id`,
/// or the bot a delete removes.
pub fn proposed_changes(name: &str, args: &Value) -> Option<Value> {
    match name {
        CREATE_BOT => args.as_object().map(|fields| Value::Object(fields.clone())),
        UPDATE_BOT => {
            let changes = args.get("changes")?.as_object()?;
            let mut proposed = Map::new();
            if let Some(bot_id) = args.get("bot_id") {
                proposed.insert("bot_id".into(), bot_id.clone());
            }
            proposed.extend(changes.clone());
            Some(Value::Object(proposed))
        }
        DELETE_BOT => Some(json!({ "bot_id": args.get("bot_id")? })),
        _ => None,
    }
}

/// The approval prompt for a create, update or delete call.
pub fn ask_message(name: &str, args: &Value) -> Option<String> {
    let field = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("?").trim();
    match name {
        CREATE_BOT => Some(format!(
            "Flux wants to create the bot '{}'. Review its settings before it is created.",
            field("name")
        )),
        UPDATE_BOT => {
            let fields = args
                .get("changes")
                .and_then(Value::as_object)
                .map(|fields| fields.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            Some(format!(
                "Flux wants to change {fields} of the bot {}. Review the proposed values; they apply from the bot's next turn.",
                field("bot_id")
            ))
        }
        DELETE_BOT => Some(format!(
            "Flux wants to delete the bot {}. Its settings, app access, secrets and workspace with files and tables are gone for good; its past messages stay.",
            field("bot_id")
        )),
        _ => None,
    }
}

fn backend(env: &dyn ToolEnv) -> ToolResult<Arc<dyn CrewBackend>> {
    env.crew().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "This host can't manage your bots. Ask the owner to use the Bots tab.",
        )
    })
}

pub async fn list_bots(env: &dyn ToolEnv, _args: ListArgs) -> ToolResult<ToolOutput> {
    let bots = backend(env)?.bots().await?;
    Ok(ToolOutput::new(json!({
        "bots": bots.iter().map(summary).collect::<Vec<_>>()
    })))
}

pub async fn get_bot(env: &dyn ToolEnv, args: GetArgs) -> ToolResult<ToolOutput> {
    let (bot, apps) = backend(env)?.bot(args.bot_id.trim()).await?;
    Ok(ToolOutput::new(detail(&bot, &apps)))
}

pub async fn create_bot(env: &dyn ToolEnv, args: Value) -> ToolResult<ToolOutput> {
    let backend = backend(env)?;
    let (request, apps) = create_request(&args, backend.profile_id())?;
    let bot = backend.create(request, &apps).await?;
    let apps: Vec<CrewApp> = apps
        .into_iter()
        .map(|app_id| CrewApp {
            app_id,
            ..CrewApp::default()
        })
        .collect();
    Ok(ToolOutput::new(json!({
        "bot": detail(&bot, &apps),
        "message": "The bot was created. It answers in its own conversation in the Bots tab; the owner can still change its look and settings there."
    })))
}

pub async fn update_bot(env: &dyn ToolEnv, args: UpdateArgs) -> ToolResult<ToolOutput> {
    update(backend(env)?.as_ref(), args).await
}

/// The bot's apps after a change: the wanted list with the new restrictions over the stored ones.
fn apps_after(
    ids: &[String],
    current: &[CrewApp],
    restricted: &[(String, BotRestrictions)],
) -> Vec<CrewApp> {
    ids.iter()
        .map(|app_id| {
            let restrictions = restricted
                .iter()
                .find(|(id, _)| id == app_id)
                .map(|(_, restrictions)| restrictions.clone())
                .or_else(|| {
                    current
                        .iter()
                        .find(|app| &app.app_id == app_id)
                        .map(|app| app.restrictions.clone())
                })
                .unwrap_or_default();
            CrewApp {
                app_id: app_id.clone(),
                restrictions,
            }
        })
        .collect()
}

/// The apps the bot uses after an update: `app_ids` when the update names them, which then need
/// `selected_apps`; otherwise the ones it has.
fn apps_wanted(
    changes: &Value,
    current: &Value,
    current_ids: &[String],
) -> ToolResult<Vec<String>> {
    let Some(wanted) = changes.get("app_ids").map(app_ids).transpose()? else {
        return Ok(current_ids.to_vec());
    };
    let capabilities = changes
        .get("capabilities")
        .or_else(|| current.get("capabilities"));
    require_selected_apps(&wanted, capabilities)?;
    Ok(wanted)
}

/// The restrictions an update sets, each for an app the bot keeps.
fn restrictions_for(changes: &Value, ids: &[String]) -> ToolResult<Vec<(String, BotRestrictions)>> {
    let restricted = changes
        .get("app_restrictions")
        .map(restriction_entries)
        .transpose()?
        .unwrap_or_default();
    if let Some((app_id, _)) = restricted.iter().find(|(id, _)| !ids.contains(id)) {
        return Err(ToolError::invalid(format!(
            "App '{app_id}' isn't attached to the bot; add it to app_ids to restrict it"
        )));
    }
    Ok(restricted)
}

/// Saves restrictions after the rest of the update, as the Bots tab's Save does.
async fn restrict_apps(
    backend: &dyn CrewBackend,
    bot_id: &str,
    restricted: &[(String, BotRestrictions)],
) -> ToolResult<()> {
    for (app_id, restrictions) in restricted {
        backend
            .restrict(bot_id, app_id, restrictions)
            .await
            .map_err(|error| {
                ToolError::new(
                    code::FAILED,
                    format!(
                        "The other changes were saved, but restricting app '{app_id}' failed: {error:#}"
                    ),
                )
            })?;
    }
    Ok(())
}

async fn update(backend: &dyn CrewBackend, args: UpdateArgs) -> ToolResult<ToolOutput> {
    let bot_id = args.bot_id.trim();
    let (current, current_apps) = backend.bot(bot_id).await?;
    let request = update_request(&args.changes, &current)?;
    let current_ids: Vec<String> = current_apps.iter().map(|app| app.app_id.clone()).collect();
    let ids = apps_wanted(&args.changes, &current, &current_ids)?;
    let restricted = restrictions_for(&args.changes, &ids)?;
    let (attach, detach) = app_changes(&ids, &current_ids);
    let bot = backend.update(bot_id, request, &attach, &detach).await?;
    restrict_apps(backend, bot_id, &restricted).await?;
    Ok(ToolOutput::new(json!({
        "bot": detail(&bot, &apps_after(&ids, &current_apps, &restricted)),
        "effective": "next_turn",
        "message": "The approved changes were saved. They apply from the bot's next turn."
    })))
}

pub async fn delete_bot(env: &dyn ToolEnv, args: DeleteArgs) -> ToolResult<ToolOutput> {
    delete(backend(env)?.as_ref(), args).await
}

async fn delete(backend: &dyn CrewBackend, args: DeleteArgs) -> ToolResult<ToolOutput> {
    let bot_id = args.bot_id.trim();
    let (bot, _) = backend.bot(bot_id).await?;
    refuse_built_in(&bot)?;
    backend.delete(bot_id).await?;
    Ok(ToolOutput::new(json!({
        "deleted": { "id": bot_id, "name": display_name(&bot) },
        "message": "The bot was deleted with its settings, app access, secrets and workspace. Its past messages stay and show it as a deleted bot."
    })))
}

pub async fn list_bot_devices(env: &dyn ToolEnv, _args: ListArgs) -> ToolResult<ToolOutput> {
    let rows = backend(env)?.devices().await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let devices: Vec<Value> = rows
        .iter()
        .filter_map(|device| device_summary(device, now_ms))
        .collect();
    let message = if devices.is_empty() {
        "The owner has no active device of their own here. Devices need an account; the owner registers them in the Devices settings."
    } else {
        "Place a bot with create_bot or update_bot, location device and a device_id. A bot on a device that is offline answers once the device's agent runs again."
    };
    Ok(ToolOutput::new(
        json!({ "devices": devices, "message": message }),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn create_requests_carry_a_look_and_refuse_flowpilot() {
        let (request, apps) = create_request(
            &json!({"name": " Weather Watch ", "description": "Morning forecast", "capabilities": ["internet", "scheduling", "selected_apps"], "app_ids": ["app-1"], "location": "server"}),
            None,
        )
        .unwrap();
        assert_eq!(request["name"], "Weather Watch");
        assert_eq!(request["location"], "server");
        assert!(STARTER_LOOKS.contains(&request["appearance"]["kind"].as_str().unwrap()));
        assert!(request.get("app_ids").is_none());
        assert_eq!(apps, ["app-1"]);
        for bad in [
            json!({"description": "no name"}),
            json!({"name": " "}),
            json!({"name": "A", "capabilities": ["flowpilot"]}),
            json!({"name": "A", "location": "device"}),
            json!({"name": "A", "location": "moon"}),
            json!({"name": "A", "device_id": "dev-1"}),
            json!({"name": "A", "location": "desktop", "device_id": "dev-1"}),
            json!({"name": "A", "look": "dragon"}),
            json!({"name": "A", "app_ids": ["x", "x"]}),
            json!({"name": "A", "owner_id": "someone"}),
            json!({"name": "A", "capabilities": ["profile_apps"]}),
            json!({"name": "A", "app_ids": ["app-1"]}),
            json!({"name": "A", "app_restrictions": [{"app_id": "app-1", "preset": "read_only"}]}),
        ] {
            assert!(create_request(&bad, None).is_err(), "{bad}");
        }
        let (request, _) = create_request(
            &json!({"name": "A", "capabilities": ["profile_apps"]}),
            Some("p1".into()),
        )
        .unwrap();
        assert_eq!(request["profile_id"], "p1");
    }

    #[test]
    fn a_new_bot_takes_a_device_and_a_classification() {
        let (request, _) = create_request(
            &json!({"name": "Hands", "capabilities": ["command_line"], "location": "device", "device_id": " dev-1 ", "classification": {"classification": "Confidential", "pii": true}}),
            None,
        )
        .unwrap();
        assert_eq!(request["location"], "device");
        assert_eq!(request["device_id"], "dev-1");
        assert_eq!(
            request["classification"],
            json!({"classification": "confidential", "pii": true})
        );
        let (request, _) = create_request(
            &json!({"name": "Quiet", "classification": {"pii": true}}),
            None,
        )
        .unwrap();
        assert_eq!(request["classification"], json!({"pii": true}));
        for bad in [
            json!({"classification": {}}),
            json!({"classification": {"classification": "secret"}}),
            json!({"classification": {"pii": "yes"}}),
            json!({"classification": {"level": "public"}}),
        ] {
            let mut args = json!({"name": "A"});
            args.as_object_mut()
                .unwrap()
                .extend(bad.as_object().unwrap().clone());
            assert!(create_request(&args, None).is_err(), "{args}");
        }
    }

    #[test]
    fn a_location_refuses_what_its_host_cannot_hold_before_the_owner_is_asked() {
        let error = create_request(
            &json!({"name": "Hands", "capabilities": ["internet", "computer_use"], "location": "server"}),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, code::INVALID_ARGS);
        assert!(
            error
                .message
                .contains("A bot on the server can't hold computer_use"),
            "{}",
            error.message
        );
        assert!(
            create_request(
                &json!({"name": "Hands", "capabilities": ["computer_use"], "location": "desktop"}),
                None,
            )
            .is_ok()
        );
        for capability in BotCapability::ALL {
            let refused = refuse_unhosted(BotLocation::Device, &[capability.as_str()]).is_err();
            assert_eq!(refused, !BotLocation::Device.allows(capability));
        }
    }

    #[test]
    fn updates_merge_models_and_looks_and_leave_apps_to_the_host() {
        let current = json!({"model": {"primary": "a", "fallbacks": ["b"], "decision": null}, "appearance": {"kind": "pip", "mute_notifications": true}});
        let request = update_request(
            &json!({"model": {"primary": "c"}, "look": "nimbus", "app_ids": ["app-2"]}),
            &current,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            request["model"],
            json!({"primary": "c", "fallbacks": ["b"], "decision": null})
        );
        assert_eq!(
            request["appearance"],
            json!({"kind": "nimbus", "mute_notifications": true})
        );
        assert!(request.get("app_ids").is_none());
        assert_eq!(
            update_request(&json!({"app_ids": []}), &current).unwrap(),
            None
        );
        assert_eq!(
            update_request(
                &json!({"app_restrictions": [{"app_id": "app-2", "preset": "full"}]}),
                &current
            )
            .unwrap(),
            None
        );
        for bad in [
            json!({}),
            json!({"location": "moon"}),
            json!({"capabilities": ["flowpilot"]}),
            json!({"system": "flux"}),
        ] {
            assert!(update_request(&bad, &current).is_err(), "{bad}");
        }
    }

    #[test]
    fn classification_changes_keep_the_field_they_leave_out() {
        let current =
            json!({"classification": {"classification": "internal", "pii": false}, "model": {}});
        let request = update_request(&json!({"classification": {"pii": true}}), &current)
            .unwrap()
            .unwrap();
        assert_eq!(
            request["classification"],
            json!({"classification": "internal", "pii": true})
        );
        let cleared = update_request(
            &json!({"classification": {"classification": null}}),
            &current,
        )
        .unwrap()
        .unwrap();
        assert_eq!(cleared["classification"], json!({"pii": false}));
    }

    #[test]
    fn a_move_is_one_patch_and_drops_nothing_on_its_own() {
        let server = json!({"location": "server", "capabilities": ["internet"], "device_id": null});
        let request = update_request(
            &json!({"location": "device", "device_id": "dev-1"}),
            &server,
        )
        .unwrap()
        .unwrap();
        assert_eq!(request, json!({"location": "device", "device_id": "dev-1"}));
        let device =
            json!({"location": "device", "capabilities": ["command_line"], "device_id": "dev-1"});
        let request = update_request(&json!({"device_id": "dev-2"}), &device)
            .unwrap()
            .unwrap();
        assert_eq!(request, json!({"device_id": "dev-2"}));
        let home = update_request(&json!({"location": "desktop"}), &device)
            .unwrap()
            .unwrap();
        assert_eq!(home, json!({"location": "desktop", "device_id": null}));

        let error = update_request(&json!({"location": "server"}), &device).unwrap_err();
        assert!(error.message.contains("command_line"), "{}", error.message);
        let moved = update_request(
            &json!({"location": "server", "capabilities": ["internet"]}),
            &device,
        )
        .unwrap()
        .unwrap();
        assert_eq!(moved["location"], "server");
        assert!(update_request(&json!({"device_id": "dev-2"}), &server).is_err());
        assert!(update_request(&json!({"location": "device"}), &server).is_err());

        let flux = json!({"system": "flux", "location": "server", "capabilities": ["flowpilot"]});
        let error = update_request(&json!({"location": "desktop"}), &flux).unwrap_err();
        assert!(error.message.contains("Flux"), "{}", error.message);
        assert!(update_request(&json!({"name": "Builder"}), &flux).is_ok());
    }

    #[test]
    fn restrictions_take_a_preset_or_permissions_and_scopes() {
        let entries = restriction_entries(&json!([
            {"app_id": "app-1", "preset": "read_only"},
            {"app_id": "app-2", "permissions": ["read_files"], "folders": ["/reports/2026/"], "files_read_only": true, "tables": []},
            {"app_id": "app-3"},
        ]))
        .unwrap();
        assert_eq!(entries[0].1, BotPreset::ReadOnly.restrictions());
        assert_eq!(
            entries[1].1.permissions,
            Some(vec![BotPermission::ReadFiles])
        );
        assert_eq!(entries[1].1.scopes.files.prefixes, ["reports/2026"]);
        assert!(entries[1].1.scopes.files.read_only);
        assert_eq!(entries[1].1.scopes.tables, Some(vec![]));
        assert!(entries[2].1.is_unrestricted());
        for bad in [
            json!([{"app_id": "a", "preset": "read_only", "permissions": ["read_files"]}]),
            json!([{"app_id": "a"}, {"app_id": "a"}]),
            json!([{"app_id": "a", "permissions": ["admin"]}]),
            json!([{"app_id": "a", "folders": ["../etc"]}]),
            json!([{"app_id": "a", "tables": [" "]}]),
            json!([{"app_id": " "}]),
            json!([{"app_id": "a", "owner": true}]),
            json!({"app-1": "read_only"}),
        ] {
            assert!(restriction_entries(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn calls_are_checked_before_approval() {
        validate_call(
            CREATE_BOT,
            &json!({"name": "A", "capabilities": ["internet"]}),
        )
        .unwrap();
        validate_call(
            UPDATE_BOT,
            &json!({"bot_id": "bot_1", "changes": {"look": "hexa"}}),
        )
        .unwrap();
        validate_call(
            UPDATE_BOT,
            &json!({"bot_id": "bot_1", "changes": {"location": "device", "device_id": "dev-1"}}),
        )
        .unwrap();
        validate_call(GET_BOT, &json!({"bot_id": "bot_1"})).unwrap();
        validate_call(DELETE_BOT, &json!({"bot_id": "bot_1"})).unwrap();
        validate_call(LIST_BOT_DEVICES, &json!({})).unwrap();
        for (tool, args) in [
            (UPDATE_BOT, json!({"bot_id": " ", "changes": {"name": "A"}})),
            (UPDATE_BOT, json!({"bot_id": "bot_1", "changes": {}})),
            (
                UPDATE_BOT,
                json!({"bot_id": "bot_1", "changes": {"name": "A"}, "owner_id": "x"}),
            ),
            (
                UPDATE_BOT,
                json!({"bot_id": "bot_1", "changes": {"location": "server", "capabilities": ["browser_use"]}}),
            ),
            (
                CREATE_BOT,
                json!({"name": "A", "capabilities": ["flowpilot"]}),
            ),
            (LIST_BOTS, json!({"owner_id": "x"})),
            (LIST_BOT_DEVICES, json!({"all": true})),
            (DELETE_BOT, json!({"bot_id": " "})),
            (DELETE_BOT, json!({"bot_id": "bot_1", "force": true})),
        ] {
            assert!(validate_call(tool, &args).is_err(), "{tool} {args}");
        }
    }

    #[test]
    fn no_bot_deletes_itself() {
        let args = json!({"bot_id": " bot_flux_copy "});
        let error = refuse_caller(DELETE_BOT, &args, "bot_flux_copy").unwrap_err();
        assert_eq!(error.code, code::SCOPE_REFUSED);
        refuse_caller(DELETE_BOT, &args, "bot_other").unwrap();
        refuse_caller(UPDATE_BOT, &args, "bot_flux_copy").unwrap();
    }

    #[test]
    fn cards_name_the_bot_they_change_or_delete() {
        let proposed = proposed_changes(
            UPDATE_BOT,
            &json!({"bot_id": "bot_1", "changes": {"name": "Atlas"}}),
        )
        .unwrap();
        assert_eq!(proposed, json!({"bot_id": "bot_1", "name": "Atlas"}));
        assert_eq!(
            proposed_changes(DELETE_BOT, &json!({"bot_id": "bot_1"})).unwrap(),
            json!({"bot_id": "bot_1"})
        );
        let message = ask_message(DELETE_BOT, &json!({"bot_id": "bot_1"})).unwrap();
        assert!(message.contains("bot_1") && message.contains("for good"));
        assert_eq!(proposed_changes(LIST_BOT_DEVICES, &json!({})), None);
    }

    #[test]
    fn app_changes_attach_and_detach_the_difference() {
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(
            app_changes(&ids(&["a", "c"]), &ids(&["a", "b"])),
            (ids(&["c"]), ids(&["b"]))
        );
    }

    #[test]
    fn summaries_name_built_ins_and_hide_instructions() {
        let flux = summary(
            &json!({"id": "bot_1", "system": "flux", "name": null, "instructions": "secret plan", "appearance": {"kind": "flux"}}),
        );
        assert_eq!(flux["name"], "Flux");
        assert_eq!(flux["look"], "flux");
        assert!(flux.get("instructions").is_none());
    }

    #[test]
    fn details_show_restrictions_only_where_they_narrow_the_role() {
        let apps = [
            CrewApp {
                app_id: "open".into(),
                restrictions: BotRestrictions::default(),
            },
            CrewApp {
                app_id: "reader".into(),
                restrictions: BotPreset::ReadOnly.restrictions(),
            },
            CrewApp {
                app_id: "custom".into(),
                restrictions: BotRestrictions {
                    permissions: Some(vec![BotPermission::ReadFiles]),
                    scopes: BotScopes {
                        tables: Some(vec!["sales".into()]),
                        ..BotScopes::default()
                    },
                },
            },
        ];
        let bot = json!({"id": "bot_1", "name": "Scout", "classification": {"pii": true}, "device_id": "dev-1", "secret": "x"});
        let detail = detail(&bot, &apps);
        assert_eq!(detail["app_ids"], json!(["open", "reader", "custom"]));
        assert_eq!(
            detail["app_restrictions"],
            json!([
                {"app_id": "reader", "preset": "read_only"},
                {"app_id": "custom", "permissions": ["read_files"], "tables": ["sales"]},
            ])
        );
        assert_eq!(detail["classification"], json!({"pii": true}));
        assert_eq!(detail["device_id"], "dev-1");
        assert!(detail.get("secret").is_none());
        let round_trip = restriction_entries(&detail["app_restrictions"]).unwrap();
        assert_eq!(round_trip[0].1, apps[1].restrictions);
        assert_eq!(round_trip[1].1, apps[2].restrictions);
    }

    #[test]
    fn devices_list_only_the_owner_s_active_ones_with_their_host_state() {
        let now = 10_000_000;
        let rows = [
            json!({"device_id": "d1", "name": "pi", "display_name": "Kitchen", "status": "active", "relationship": "owner", "bot_host_seen_at": now - 60_000}),
            json!({"device_id": "d2", "name": "nas", "display_name": null, "status": "active", "relationship": "owner", "bot_host_seen_at": now - 600_000}),
            json!({"device_id": "d3", "name": "old", "status": "active", "relationship": "owner"}),
            json!({"device_id": "d4", "name": "gone", "status": "revoked", "relationship": "owner"}),
            json!({"device_id": "d5", "name": "theirs", "status": "active", "relationship": "shared"}),
        ];
        let devices: Vec<Value> = rows
            .iter()
            .filter_map(|row| device_summary(row, now))
            .collect();
        let view: Vec<(&str, &str, &str)> = devices
            .iter()
            .map(|device| {
                (
                    device["device_id"].as_str().unwrap(),
                    device["name"].as_str().unwrap(),
                    device["bot_host"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            view,
            [
                ("d1", "Kitchen", "online"),
                ("d2", "nas", "offline"),
                ("d3", "old", "never")
            ]
        );
        assert_eq!(devices[0]["cannot_host"], json!(unhosted_on_device()));
    }

    #[test]
    fn only_changes_ask() {
        for spec in specs() {
            assert_eq!(spec.approval == Approval::Ask, requires_approval(spec.name));
            assert_eq!(spec.mutating, requires_approval(spec.name));
            assert!(is_crew_tool(spec.name));
            assert_eq!(spec.capability, None);
        }
        assert!(crate::configuration::requires_approval(DELETE_BOT));
        assert!(crate::configuration::is_settings_tool(LIST_BOT_DEVICES));
        assert!(!requires_approval(LIST_BOT_DEVICES));
    }

    #[derive(Default)]
    struct FakeCrew {
        bots: Vec<Value>,
        apps: Vec<CrewApp>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeCrew {
        fn with(bots: Vec<Value>, apps: Vec<CrewApp>) -> Self {
            Self {
                bots,
                apps,
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    #[async_trait]
    impl CrewBackend for FakeCrew {
        async fn bots(&self) -> Result<Vec<Value>> {
            Ok(self.bots.clone())
        }

        async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<CrewApp>)> {
            let bot = self
                .bots
                .iter()
                .find(|bot| bot["id"] == bot_id)
                .cloned()
                .ok_or_else(|| flow_like_types::anyhow!("Bot '{bot_id}' not found"))?;
            Ok((bot, self.apps.clone()))
        }

        fn profile_id(&self) -> Option<String> {
            None
        }

        async fn create(&self, request: Value, _app_ids: &[String]) -> Result<Value> {
            self.record(format!("create {request}"));
            Ok(request)
        }

        async fn update(
            &self,
            bot_id: &str,
            request: Option<Value>,
            attach: &[String],
            detach: &[String],
        ) -> Result<Value> {
            self.record(format!(
                "update {bot_id} {} +{attach:?} -{detach:?}",
                request.unwrap_or(Value::Null)
            ));
            Ok(self.bot(bot_id).await?.0)
        }

        async fn restrict(
            &self,
            bot_id: &str,
            app_id: &str,
            restrictions: &BotRestrictions,
        ) -> Result<()> {
            self.record(format!(
                "restrict {bot_id} {app_id} {}",
                json!(restrictions)
            ));
            Ok(())
        }

        async fn delete(&self, bot_id: &str) -> Result<()> {
            self.record(format!("delete {bot_id}"));
            Ok(())
        }

        async fn devices(&self) -> Result<Vec<Value>> {
            Ok(Vec::new())
        }
    }

    fn crew() -> FakeCrew {
        FakeCrew::with(
            vec![
                json!({"id": "bot_general", "system": "general", "name": null, "capabilities": ["internet"], "location": "server"}),
                json!({"id": "bot_flux", "system": "flux", "name": null, "capabilities": ["flowpilot"], "location": "server"}),
                json!({"id": "bot_scout", "system": null, "name": "Scout", "capabilities": ["internet", "selected_apps", "command_line"], "location": "desktop", "model": {}}),
            ],
            vec![CrewApp {
                app_id: "app-1".into(),
                restrictions: BotRestrictions::default(),
            }],
        )
    }

    #[tokio::test]
    async fn built_in_bots_and_moves_that_lose_capabilities_are_refused_before_the_card() {
        let crew = crew();
        for (tool, args) in [
            (DELETE_BOT, json!({"bot_id": "bot_general"})),
            (DELETE_BOT, json!({"bot_id": "bot_flux"})),
            (DELETE_BOT, json!({"bot_id": "bot_missing"})),
            (
                UPDATE_BOT,
                json!({"bot_id": "bot_flux", "changes": {"location": "desktop"}}),
            ),
            (
                UPDATE_BOT,
                json!({"bot_id": "bot_scout", "changes": {"location": "server"}}),
            ),
        ] {
            assert!(precheck(&crew, tool, &args).await.is_err(), "{tool} {args}");
        }
        precheck(&crew, DELETE_BOT, &json!({"bot_id": "bot_scout"}))
            .await
            .unwrap();
        precheck(
            &crew,
            UPDATE_BOT,
            &json!({"bot_id": "bot_flux", "changes": {"name": "Builder"}}),
        )
        .await
        .unwrap();
        assert!(!reads_target_first(
            UPDATE_BOT,
            &json!({"bot_id": "bot_flux", "changes": {"name": "Builder"}})
        ));
        assert!(crew.calls().is_empty());
    }

    #[tokio::test]
    async fn a_delete_runs_once_the_target_is_not_built_in() {
        let crew = crew();
        let error = delete(
            &crew,
            DeleteArgs {
                bot_id: "bot_flux".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, code::SCOPE_REFUSED);
        assert!(crew.calls().is_empty());
        let output = delete(
            &crew,
            DeleteArgs {
                bot_id: " bot_scout ".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(crew.calls(), ["delete bot_scout"]);
        assert_eq!(output.result["deleted"]["name"], "Scout");
    }

    #[tokio::test]
    async fn an_update_saves_settings_then_apps_then_restrictions() {
        let crew = crew();
        let output = update(
            &crew,
            UpdateArgs {
                bot_id: "bot_scout".into(),
                changes: json!({
                    "classification": {"classification": "restricted"},
                    "app_ids": ["app-1", "app-2"],
                    "app_restrictions": [{"app_id": "app-2", "preset": "read_and_call_events"}],
                }),
            },
        )
        .await
        .unwrap();
        let restricted = json!(BotPreset::ReadAndCallEvents.restrictions());
        assert_eq!(
            crew.calls(),
            [
                r#"update bot_scout {"classification":{"classification":"restricted","pii":false}} +["app-2"] -[]"#.to_string(),
                format!("restrict bot_scout app-2 {restricted}"),
            ]
        );
        assert_eq!(
            output.result["bot"]["app_restrictions"],
            json!([{"app_id": "app-2", "preset": "read_and_call_events"}])
        );
        let error = update(
            &crew,
            UpdateArgs {
                bot_id: "bot_scout".into(),
                changes: json!({"app_restrictions": [{"app_id": "app-9", "preset": "read_only"}]}),
            },
        )
        .await
        .unwrap_err();
        assert!(error.message.contains("app-9"), "{}", error.message);
        assert_eq!(crew.calls().len(), 2);
    }
}
