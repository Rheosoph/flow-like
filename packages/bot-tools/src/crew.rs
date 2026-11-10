//! Flux manages its owner's other bots: it lists and reads them, creates new ones and changes
//! their settings. Hosts bind the backend to the owner, so a call can never reach another
//! person's bots. Creating or changing a bot always needs the owner's explicit approval.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use flow_like_types::{Result, bot::BotCapability};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    ToolEnv, ToolError, ToolOutput, ToolResult,
    configuration::{merge_model_changes, settings_properties, snapshot, validate_changes},
    spec::{Approval, ToolSpec, schema_of},
};

pub const LIST_BOTS: &str = "list_bots";
pub const GET_BOT: &str = "get_bot";
pub const CREATE_BOT: &str = "create_bot";
pub const UPDATE_BOT: &str = "update_bot";

/// The characters every owner has, so a bot Flux creates never needs a pack.
pub const STARTER_LOOKS: [&str; 5] = ["pip", "mochi", "hexa", "pebble", "nimbus"];
/// Where Flux may place a new bot; a `device` bot needs a device the owner picks in the editor.
const LOCATIONS: [&str; 2] = ["server", "desktop"];
const MAX_APPS: usize = 50;
const MAX_APP_ID_CHARS: usize = 128;
/// Fields that are not bot settings: the look, the placement and the attached apps.
const CREW_FIELDS: [&str; 3] = ["look", "location", "app_ids"];

pub fn is_crew_tool(name: &str) -> bool {
    matches!(name, LIST_BOTS | GET_BOT | CREATE_BOT | UPDATE_BOT)
}

pub fn requires_approval(name: &str) -> bool {
    matches!(name, CREATE_BOT | UPDATE_BOT)
}

/// The owner's bots as the API's `Bot`. The host must obtain approval for the exact request
/// before [`CrewBackend::create`] or [`CrewBackend::update`].
#[async_trait]
pub trait CrewBackend: Send + Sync {
    async fn bots(&self) -> Result<Vec<Value>>;
    /// One of the owner's bots with the ids of the apps attached to it.
    async fn bot(&self, bot_id: &str) -> Result<(Value, Vec<String>)>;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    pub bot_id: String,
    pub changes: Value,
}

fn assignable_capabilities() -> Vec<&'static str> {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| !capability.is_fixed())
        .map(BotCapability::as_str)
        .collect()
}

fn crew_properties(properties: &mut Map<String, Value>, app_ids: &str) {
    properties.insert(
        "look".into(),
        json!({"type": "string", "enum": STARTER_LOOKS, "description": "The character the bot appears as. Omitted on a new bot: one is picked from its name."}),
    );
    properties.insert(
        "app_ids".into(),
        json!({"type": "array", "maxItems": MAX_APPS, "uniqueItems": true, "description": app_ids,
            "items": {"type": "string", "minLength": 1, "maxLength": MAX_APP_ID_CHARS}}),
    );
}

fn create_schema() -> Value {
    let mut properties = settings_properties(&assignable_capabilities());
    crew_properties(
        &mut properties,
        "Apps from list_apps the bot may use with the owner's role there. Needs the selected_apps capability.",
    );
    properties.insert(
        "location".into(),
        json!({"type": "string", "enum": LOCATIONS, "description": "Where the bot runs: server (default when signed in; works while the owner is offline) or desktop (this device, needed for computer_use, browser_use, command_line and encryption). Signed out, bots always run on this desktop."}),
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
    );
    json!({"type": "object", "additionalProperties": false, "required": ["bot_id", "changes"], "properties": {
        "bot_id": {"type": "string", "minLength": 1, "description": "The bot's id from list_bots."},
        "changes": {"type": "object", "additionalProperties": false, "minProperties": 1, "properties": properties},
    }})
}

const CREATE_DESCRIPTION: &str = "Create a new bot for the owner. A bot is a persona that chats in its own conversation and in rooms, acts with the owner's rights and holds only the capabilities you switch on. Give it a short name, a one-line description and instructions (its system prompt: role, tone, what to do and what to avoid). Switch on only the capabilities the purpose needs: internet (web research), web_requests (direct API and URL calls), memory, scheduling (one-time and recurring tasks), push_notifications, email, selected_apps (with app_ids) or profile_apps (every app of the current profile), storage, data_studio, documents, barcodes (QR codes and barcodes in images), knowledge, code_execution, image_generation, skills, connectors, channels, delegation, user_context, webhook; computer_use, browser_use, command_line and encryption need location desktop. Use bot_models for exact model ids; omit models to use the owner's defaults. Building apps, workflows and pages stays your work: build the app first, then give the bot that app. Every call asks the owner, including in auto mode.";

const UPDATE_DESCRIPTION: &str = "Change another bot's settings: name, description, instructions (system prompt), capabilities, models, look or apps. Read get_bot first. Send only the fields to change; null clears a nullable field. capabilities and app_ids replace the full list, model fields are patched individually. Flux's own capabilities are fixed; change your own name, description, instructions or models with bot_update_configuration. Every call asks the owner, including in auto mode. Changes apply from the bot's next turn.";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: LIST_BOTS,
            description: "List the owner's bots: id, name, description, built-in kind, capabilities, location and look. Use it before creating a bot that may already exist and to find the bot_id for get_bot and update_bot.",
            capability: None,
            module: "crew",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ListArgs>(),
        },
        ToolSpec {
            name: GET_BOT,
            description: "Read one of the owner's bots in full: instructions, capabilities, model mappings, location, look and the ids of the apps it may use. Read it before update_bot.",
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

fn app_ids(value: &Value) -> ToolResult<Vec<String>> {
    let ids = value
        .as_array()
        .filter(|ids| ids.len() <= MAX_APPS)
        .ok_or_else(|| ToolError::invalid(format!("app_ids must list at most {MAX_APPS} apps")))?;
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(|id| {
            let id = id
                .as_str()
                .map(str::trim)
                .filter(|id| !id.is_empty() && id.chars().count() <= MAX_APP_ID_CHARS)
                .ok_or_else(|| ToolError::invalid("Each app id must be an id from list_apps"))?;
            if !seen.insert(id) {
                return Err(ToolError::invalid(format!("App '{id}' is listed twice")));
            }
            Ok(id.to_string())
        })
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
    if let Some(location) = crew.get("location")
        && !location.as_str().is_some_and(|id| LOCATIONS.contains(&id))
    {
        return Err(ToolError::invalid("location must be server or desktop"));
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
/// are the host's to check.
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
        LIST_BOTS => serde_json::from_value::<ListArgs>(args.clone())
            .map(|_| ())
            .map_err(|error| ToolError::invalid(format!("Bad arguments for list_bots: {error}"))),
        GET_BOT => serde_json::from_value::<GetArgs>(args.clone())
            .map(|_| ())
            .map_err(|error| ToolError::invalid(format!("Bad arguments for get_bot: {error}"))),
        _ => Ok(()),
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
    let kind = match crew.get("look") {
        Some(value) => look(value)?,
        None => default_look(&name),
    };
    request.insert("appearance".into(), json!({ "kind": kind }));
    if let Some(location) = crew.get("location") {
        request.insert("location".into(), location.clone());
    }
    let wants_profile = request
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some("profile_apps")));
    if wants_profile {
        let profile_id = profile_id.ok_or_else(|| {
            ToolError::invalid(
                "profile_apps needs an active profile; use selected_apps with app_ids instead",
            )
        })?;
        request.insert("profile_id".into(), Value::String(profile_id));
    }
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

/// The API `UpdateBotRequest` for validated `update_bot` changes against the bot's current
/// settings; `None` when the changes only move apps.
pub fn update_request(changes: &Value, current: &Value) -> ToolResult<Option<Value>> {
    let fields = changes
        .as_object()
        .filter(|fields| !fields.is_empty())
        .ok_or_else(|| ToolError::invalid("changes must be a non-empty object"))?;
    if fields.contains_key("location") {
        return Err(ToolError::invalid(
            "update_bot can't move a bot; the owner changes Runs on in the bot's settings",
        ));
    }
    let (settings, crew) = split(fields)?;
    let mut request = Value::Object(settings);
    if request.as_object().is_some_and(|fields| !fields.is_empty()) {
        merge_model_changes(&mut request, current.get("model").unwrap_or(&Value::Null))?;
    }
    let fields = request.as_object_mut().expect("settings are an object");
    if let Some(value) = crew.get("look") {
        let mut appearance = current
            .get("appearance")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        appearance.insert("kind".into(), Value::String(look(value)?.to_string()));
        fields.insert("appearance".into(), Value::Object(appearance));
    }
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

/// One line per bot in `list_bots`: never instructions, credentials or stored metadata.
pub fn summary(bot: &Value) -> Value {
    let mut out = Map::new();
    for field in ["id", "description", "system", "capabilities", "location"] {
        if let Some(value) = bot.get(field) {
            out.insert(field.into(), value.clone());
        }
    }
    out.insert("name".into(), display_name(bot));
    out.insert("look".into(), bot_look(bot));
    Value::Object(out)
}

/// A bot in full for `get_bot` and after a change: its settings, look and attached apps.
pub fn detail(bot: &Value, app_ids: &[String]) -> Value {
    let mut out = snapshot(bot);
    if let Some(fields) = out.as_object_mut() {
        fields.insert("name".into(), display_name(bot));
        fields.insert("look".into(), bot_look(bot));
        fields.insert("app_ids".into(), json!(app_ids));
    }
    out
}

/// What an approval card shows: the new bot's settings, or the changes to one with its
/// `bot_id`.
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
        _ => None,
    }
}

/// The approval prompt for a create or update call.
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
        _ => None,
    }
}

fn backend(env: &dyn ToolEnv) -> ToolResult<Arc<dyn CrewBackend>> {
    env.crew().ok_or_else(|| {
        ToolError::new(
            crate::error::code::NOT_YET,
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
    Ok(ToolOutput::new(json!({
        "bot": detail(&bot, &apps),
        "message": "The bot was created. It answers in its own conversation in the Bots tab; the owner can still change its look and settings there."
    })))
}

pub async fn update_bot(env: &dyn ToolEnv, args: UpdateArgs) -> ToolResult<ToolOutput> {
    let backend = backend(env)?;
    let bot_id = args.bot_id.trim();
    let (current, current_apps) = backend.bot(bot_id).await?;
    let request = update_request(&args.changes, &current)?;
    let wanted = args.changes.get("app_ids").map(app_ids).transpose()?;
    if let Some(wanted) = &wanted {
        let capabilities = args
            .changes
            .get("capabilities")
            .or_else(|| current.get("capabilities"));
        require_selected_apps(wanted, capabilities)?;
    }
    let (attach, detach) = wanted
        .as_deref()
        .map(|wanted| app_changes(wanted, &current_apps))
        .unwrap_or_default();
    let bot = backend.update(bot_id, request, &attach, &detach).await?;
    let apps = wanted.unwrap_or(current_apps);
    Ok(ToolOutput::new(json!({
        "bot": detail(&bot, &apps),
        "effective": "next_turn",
        "message": "The approved changes were saved. They apply from the bot's next turn."
    })))
}

#[cfg(test)]
mod tests {
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
            json!({"name": "A", "look": "dragon"}),
            json!({"name": "A", "app_ids": ["x", "x"]}),
            json!({"name": "A", "owner_id": "someone"}),
            json!({"name": "A", "capabilities": ["profile_apps"]}),
            json!({"name": "A", "app_ids": ["app-1"]}),
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
        for bad in [
            json!({}),
            json!({"location": "server"}),
            json!({"capabilities": ["flowpilot"]}),
            json!({"system": "flux"}),
        ] {
            assert!(update_request(&bad, &current).is_err(), "{bad}");
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
        validate_call(GET_BOT, &json!({"bot_id": "bot_1"})).unwrap();
        for (tool, args) in [
            (UPDATE_BOT, json!({"bot_id": " ", "changes": {"name": "A"}})),
            (UPDATE_BOT, json!({"bot_id": "bot_1", "changes": {}})),
            (
                UPDATE_BOT,
                json!({"bot_id": "bot_1", "changes": {"name": "A"}, "owner_id": "x"}),
            ),
            (
                CREATE_BOT,
                json!({"name": "A", "capabilities": ["flowpilot"]}),
            ),
            (LIST_BOTS, json!({"owner_id": "x"})),
        ] {
            assert!(validate_call(tool, &args).is_err(), "{tool} {args}");
        }
    }

    #[test]
    fn update_cards_name_the_bot_they_change() {
        let proposed = proposed_changes(
            UPDATE_BOT,
            &json!({"bot_id": "bot_1", "changes": {"name": "Atlas"}}),
        )
        .unwrap();
        assert_eq!(proposed, json!({"bot_id": "bot_1", "name": "Atlas"}));
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
    fn only_create_and_update_ask() {
        for spec in specs() {
            assert_eq!(spec.approval == Approval::Ask, requires_approval(spec.name));
            assert_eq!(spec.mutating, requires_approval(spec.name));
            assert!(is_crew_tool(spec.name));
            assert_eq!(spec.capability, None);
        }
        assert!(crate::configuration::requires_approval(CREATE_BOT));
        assert!(crate::configuration::is_settings_tool(UPDATE_BOT));
    }
}
