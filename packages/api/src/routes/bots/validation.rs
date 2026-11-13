use super::model::{BotSettings, CreateBotRequest, UpdateBotRequest};
use crate::error::ApiError;
use axum::http::StatusCode;
use flow_like_types::{
    Value,
    bot::{BotCapability, BotExecutionMode, BotLocation, BotModelPolicy, BotSystemKind},
    data_classification::DataSensitivity,
};

pub(super) const NAME_MAX_CHARS: usize = 64;
pub(super) const DESCRIPTION_MAX_CHARS: usize = 2_000;
pub(super) const INSTRUCTIONS_MAX_CHARS: usize = 8_000;
pub(super) const PRESET_ID_MAX_CHARS: usize = 128;
pub(super) const REFERENCE_ID_MAX_CHARS: usize = 256;
pub(super) const APPEARANCE_MAX_BYTES: usize = 16 * 1024;
pub(super) const MODEL_ID_MAX_CHARS: usize = 256;
pub(super) const MAX_FALLBACKS: usize = 3;
pub(super) const MAX_CUSTOM_BOTS: u64 = 50;

pub(super) const BOT_CAPABILITY_LOCATION: &str = "BOT_CAPABILITY_LOCATION";
pub(super) const BOT_CAPABILITIES_FIXED: &str = "BOT_CAPABILITIES_FIXED";
pub(super) const BOT_SYSTEM_UNDELETABLE: &str = "BOT_SYSTEM_UNDELETABLE";
pub(super) const BOT_LIMIT_REACHED: &str = "BOT_LIMIT_REACHED";
pub(super) const BOT_SERVER_LIMIT_REACHED: &str = "BOT_SERVER_LIMIT_REACHED";

/// The preset id and settings of a new bot: every field goes through the same validation as
/// an update of an empty bot, so a fixed capability can't be switched on.
pub(super) fn create_settings(
    request: CreateBotRequest,
) -> Result<(Option<String>, BotSettings), ApiError> {
    let (preset_id, empty, fields) = new_bot(request)?;
    Ok((preset_id, apply_patch(&empty, None, fields)?))
}

/// A bot moving in from a device: validated like a new bot, except that it keeps a fixed
/// capability it holds (a duplicate of Flux made on the device keeps `flowpilot`).
pub(super) fn moved_settings(
    request: CreateBotRequest,
) -> Result<(Option<String>, BotSettings), ApiError> {
    let (preset_id, empty, fields) = new_bot(request)?;
    Ok((preset_id, merge(&empty, None, fields)?))
}

/// A new bot's preset id, the empty bot it starts from and its fields as a patch.
fn new_bot(
    request: CreateBotRequest,
) -> Result<(Option<String>, BotSettings, UpdateBotRequest), ApiError> {
    let preset_id = optional_text("preset_id", request.preset_id, PRESET_ID_MAX_CHARS)?;
    let empty = BotSettings {
        name: None,
        description: None,
        instructions: None,
        appearance: Value::Null,
        model: BotModelPolicy::default(),
        embedding_model: None,
        stt_model: None,
        tts_model: None,
        image_model: None,
        vision_model: None,
        subtask_model: None,
        classification: DataSensitivity::default(),
        capabilities: Vec::new(),
        profile_id: None,
        location: BotLocation::Server,
        execution_mode: BotExecutionMode::Native,
        device_id: None,
    };
    let fields = UpdateBotRequest {
        name: Some(Some(request.name)),
        description: Some(request.description),
        instructions: Some(request.instructions),
        appearance: Some(request.appearance),
        model: request.model,
        embedding_model: Some(request.embedding_model),
        stt_model: Some(request.stt_model),
        tts_model: Some(request.tts_model),
        image_model: Some(request.image_model),
        vision_model: Some(request.vision_model),
        subtask_model: Some(request.subtask_model),
        classification: request.classification,
        capabilities: request.capabilities,
        profile_id: Some(request.profile_id),
        location: request.location,
        execution_mode: request.execution_mode,
        device_id: Some(request.device_id),
    };
    Ok((preset_id, empty, fields))
}

/// `current` with `patch` applied. Omitted fields stay, `null` clears nullable fields, the
/// merged result must satisfy every rule a new bot does, and no fixed capability is gained.
pub(super) fn apply_patch(
    current: &BotSettings,
    system: Option<BotSystemKind>,
    patch: UpdateBotRequest,
) -> Result<BotSettings, ApiError> {
    let next = merge(current, system, patch)?;
    refuse_gained_fixed(&current.capabilities, &next.capabilities)?;
    Ok(next)
}

fn merge(
    current: &BotSettings,
    system: Option<BotSystemKind>,
    mut patch: UpdateBotRequest,
) -> Result<BotSettings, ApiError> {
    let mut next = current.clone();
    patch_texts(&mut next, system, &mut patch)?;
    patch_look(&mut next, &mut patch)?;
    patch_scope(&mut next, current, system, &mut patch)?;
    check_rules(&next)?;
    Ok(next)
}

/// Only Flux holds `flowpilot` from the start; other bots get it by duplicating one that holds
/// it, never by switching it on.
fn refuse_gained_fixed(before: &[BotCapability], after: &[BotCapability]) -> Result<(), ApiError> {
    match after
        .iter()
        .find(|capability| capability.is_fixed() && !before.contains(capability))
    {
        Some(capability) => Err(ApiError::coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            BOT_CAPABILITIES_FIXED,
            format!(
                "Only Flux and its duplicates hold '{}'; duplicate Flux to get a bot with it",
                capability.as_str()
            ),
        )),
        None => Ok(()),
    }
}

/// Sets `target` to the parsed `value` when the patch carries one.
fn replace<V, T>(
    target: &mut T,
    value: Option<V>,
    parse: impl FnOnce(V) -> Result<T, ApiError>,
) -> Result<(), ApiError> {
    if let Some(value) = value {
        *target = parse(value)?;
    }
    Ok(())
}

fn patch_texts(
    next: &mut BotSettings,
    system: Option<BotSystemKind>,
    patch: &mut UpdateBotRequest,
) -> Result<(), ApiError> {
    replace(&mut next.name, patch.name.take(), |name| {
        patched_name(name, system)
    })?;
    replace(&mut next.description, patch.description.take(), |text| {
        optional_text("description", text, DESCRIPTION_MAX_CHARS)
    })?;
    replace(&mut next.instructions, patch.instructions.take(), |text| {
        optional_text("instructions", text, INSTRUCTIONS_MAX_CHARS)
    })
}

fn patch_look(next: &mut BotSettings, patch: &mut UpdateBotRequest) -> Result<(), ApiError> {
    replace(&mut next.appearance, patch.appearance.take(), appearance)?;
    replace(&mut next.model, patch.model.take(), model_policy)?;
    replace(
        &mut next.embedding_model,
        patch.embedding_model.take(),
        |id| optional_text("embedding_model", id, MODEL_ID_MAX_CHARS),
    )?;
    replace(&mut next.stt_model, patch.stt_model.take(), |id| {
        optional_text("stt_model", id, MODEL_ID_MAX_CHARS)
    })?;
    replace(&mut next.tts_model, patch.tts_model.take(), |id| {
        optional_text("tts_model", id, MODEL_ID_MAX_CHARS)
    })?;
    replace(&mut next.image_model, patch.image_model.take(), |id| {
        optional_text("image_model", id, MODEL_ID_MAX_CHARS)
    })?;
    replace(&mut next.vision_model, patch.vision_model.take(), |id| {
        optional_text("vision_model", id, MODEL_ID_MAX_CHARS)
    })?;
    replace(&mut next.subtask_model, patch.subtask_model.take(), |id| {
        optional_text("subtask_model", id, MODEL_ID_MAX_CHARS)
    })?;
    replace(&mut next.classification, patch.classification.take(), Ok)
}

fn patch_scope(
    next: &mut BotSettings,
    current: &BotSettings,
    system: Option<BotSystemKind>,
    patch: &mut UpdateBotRequest,
) -> Result<(), ApiError> {
    replace(&mut next.capabilities, patch.capabilities.take(), |ids| {
        patched_capabilities(current, system, ids)
    })?;
    replace(&mut next.profile_id, patch.profile_id.take(), |id| {
        reference_id("profile_id", id)
    })?;
    replace(&mut next.location, patch.location.take(), Ok)?;
    replace(&mut next.execution_mode, patch.execution_mode.take(), Ok)?;
    replace(&mut next.device_id, patch.device_id.take(), |id| {
        reference_id("device_id", id)
    })
}

/// `null` resets a built-in bot to its localized default name; a bot the owner created keeps one.
fn patched_name(
    name: Option<String>,
    system: Option<BotSystemKind>,
) -> Result<Option<String>, ApiError> {
    match name {
        Some(name) => bot_name(&name).map(Some),
        None if system.is_some() => Ok(None),
        None => Err(ApiError::bad_request(
            "A bot you created needs a name; send a name instead of null",
        )),
    }
}

/// The capabilities of Flux and of every bot that holds a fixed capability (its duplicates) are
/// fixed: sending the same set again is fine, a different one is not.
fn patched_capabilities(
    current: &BotSettings,
    system: Option<BotSystemKind>,
    ids: Vec<String>,
) -> Result<Vec<BotCapability>, ApiError> {
    let next = capabilities(ids)?;
    let built_in = system.filter(|kind| kind.capabilities_fixed());
    let fixed = built_in.is_some() || current.capabilities.iter().any(|c| c.is_fixed());
    if !fixed || next == current.capabilities {
        return Ok(next);
    }
    let whose = match built_in {
        Some(kind) => format!("the built-in bot '{}'", kind.as_str()),
        None => "a bot with 'flowpilot'".to_string(),
    };
    Err(ApiError::coded(
        StatusCode::UNPROCESSABLE_ENTITY,
        BOT_CAPABILITIES_FIXED,
        format!("The capabilities of {whose} can't be changed"),
    ))
}

pub(super) fn location_refusal(location: BotLocation, capability: BotCapability) -> String {
    let why = if capability == BotCapability::Encryption {
        " (its key lives only on the host that runs it, and the server must never read encrypted chats)"
    } else {
        ""
    };
    format!(
        "A bot on the {} can't hold '{}'{why}; move it to a desktop or device first",
        location.as_str(),
        capability.as_str()
    )
}

/// Rules between fields, checked on every create and on the merged result of every update.
pub(super) fn check_rules(settings: &BotSettings) -> Result<(), ApiError> {
    if settings.execution_mode == BotExecutionMode::Docker
        && settings.location != BotLocation::Device
    {
        return Err(ApiError::coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "BOT_EXECUTION_MODE_LOCATION",
            "Docker hosting requires a managed device. Choose native execution explicitly before moving this bot to another host",
        ));
    }
    if let Some(capability) = settings
        .capabilities
        .iter()
        .find(|capability| !settings.location.allows(**capability))
    {
        return Err(ApiError::coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            BOT_CAPABILITY_LOCATION,
            location_refusal(settings.location, *capability),
        ));
    }
    if settings.has(BotCapability::ProfileApps) && settings.profile_id.is_none() {
        return Err(ApiError::bad_request(
            "The 'profile_apps' capability needs a profile_id",
        ));
    }
    if settings.location == BotLocation::Device && settings.device_id.is_none() {
        return Err(ApiError::bad_request("A bot on a device needs a device_id"));
    }
    Ok(())
}

fn too_long(field: &str, max_chars: usize, chars: usize) -> ApiError {
    ApiError::bad_request(format!(
        "Bot {field} has {chars} characters; at most {max_chars} are allowed"
    ))
}

fn bot_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Bot name must not be empty"));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "Bot name must not contain control characters or line breaks",
        ));
    }
    let chars = name.chars().count();
    if chars > NAME_MAX_CHARS {
        return Err(too_long("name", NAME_MAX_CHARS, chars));
    }
    Ok(name.to_string())
}

/// Trimmed text, `None` when empty.
fn optional_text(
    field: &str,
    value: Option<String>,
    max_chars: usize,
) -> Result<Option<String>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let chars = value.chars().count();
    if chars > max_chars {
        return Err(too_long(field, max_chars, chars));
    }
    Ok(Some(value.to_string()))
}

fn reference_id(field: &str, value: Option<String>) -> Result<Option<String>, ApiError> {
    optional_text(field, value, REFERENCE_ID_MAX_CHARS)
}

fn appearance(value: Value) -> Result<Value, ApiError> {
    let Value::Object(fields) = &value else {
        return Err(ApiError::bad_request(
            "Bot appearance must be a JSON object with a string 'kind'",
        ));
    };
    if !fields
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.trim().is_empty())
    {
        return Err(ApiError::bad_request(
            "Bot appearance needs a non-empty string 'kind'",
        ));
    }
    let size = serde_json::to_vec(&value)
        .map_err(|error| ApiError::bad_request(format!("Bot appearance is not JSON: {error}")))?
        .len();
    if size > APPEARANCE_MAX_BYTES {
        return Err(ApiError::bad_request(format!(
            "Bot appearance is {size} bytes; at most {APPEARANCE_MAX_BYTES} are allowed"
        )));
    }
    Ok(value)
}

fn model_id(field: &str, value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!(
            "Bot model {field} must not be empty"
        )));
    }
    let chars = value.chars().count();
    if chars > MODEL_ID_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "Bot model {field} has {chars} characters; at most {MODEL_ID_MAX_CHARS} are allowed"
        )));
    }
    Ok(value.to_string())
}

fn model_policy(policy: BotModelPolicy) -> Result<BotModelPolicy, ApiError> {
    if policy.fallbacks.len() > MAX_FALLBACKS {
        return Err(ApiError::bad_request(format!(
            "Bot model has {} fallbacks; at most {MAX_FALLBACKS} are allowed",
            policy.fallbacks.len()
        )));
    }
    Ok(BotModelPolicy {
        decision: policy
            .decision
            .filter(|id| !id.trim().is_empty())
            .map(|id| model_id("decision", &id))
            .transpose()?,
        primary: policy
            .primary
            .filter(|primary| !primary.trim().is_empty())
            .map(|primary| model_id("primary", &primary))
            .transpose()?,
        fallbacks: policy
            .fallbacks
            .iter()
            .map(|fallback| model_id("fallback", fallback))
            .collect::<Result<_, _>>()?,
    })
}

/// Known ids in their canonical order, without duplicates.
fn capabilities(ids: Vec<String>) -> Result<Vec<BotCapability>, ApiError> {
    let mut parsed = Vec::with_capacity(ids.len());
    for id in ids {
        let capability = BotCapability::parse(&id).ok_or_else(|| {
            let known = BotCapability::ALL
                .iter()
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            ApiError::bad_request(format!("Unknown bot capability '{id}'; use one of {known}"))
        })?;
        parsed.push(capability);
    }
    Ok(BotCapability::ALL
        .into_iter()
        .filter(|capability| parsed.contains(capability))
        .collect())
}

/// The settings Assistant and Flux start with.
pub(super) fn system_defaults(kind: BotSystemKind) -> BotSettings {
    let (look, capabilities) = match kind {
        BotSystemKind::General => ("pip", vec![BotCapability::Internet, BotCapability::Memory]),
        BotSystemKind::Flux => ("flux", BotSystemKind::Flux.fixed_capabilities().to_vec()),
    };
    BotSettings {
        name: None,
        description: None,
        instructions: None,
        appearance: serde_json::json!({ "kind": look }),
        model: BotModelPolicy::default(),
        embedding_model: None,
        stt_model: None,
        tts_model: None,
        image_model: None,
        vision_model: None,
        subtask_model: None,
        classification: DataSensitivity::default(),
        capabilities,
        profile_id: None,
        location: BotLocation::Server,
        execution_mode: BotExecutionMode::Native,
        device_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create(body: Value) -> Result<(Option<String>, BotSettings), ApiError> {
        create_settings(serde_json::from_value(body).unwrap())
    }

    fn patch(
        current: &BotSettings,
        system: Option<BotSystemKind>,
        body: Value,
    ) -> Result<BotSettings, ApiError> {
        apply_patch(current, system, serde_json::from_value(body).unwrap())
    }

    fn custom() -> BotSettings {
        create(json!({
            "name": "Scout",
            "description": "Finds things",
            "appearance": { "kind": "pip" },
            "capabilities": ["internet"],
        }))
        .unwrap()
        .1
    }

    fn code(error: &ApiError) -> (StatusCode, &str) {
        (error.status(), error.public_code())
    }

    #[test]
    fn bot_names_reject_control_characters_and_line_breaks() {
        for name in ["Scout\nwrote:", "Sc\tout", "Scout\u{7}", "a\r\nb"] {
            let error = bot_name(name).unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name:?}");
        }
        assert_eq!(bot_name("  Scout  ").unwrap(), "Scout");
    }

    #[test]
    fn a_new_bot_starts_with_only_what_it_asks_for() {
        let (preset, settings) = create(json!({
            "name": "  Scout  ",
            "preset_id": "researcher",
            "appearance": { "kind": "pip" },
            "capabilities": ["webhook", "INTERNET", "internet"],
        }))
        .unwrap();
        assert_eq!(preset.as_deref(), Some("researcher"));
        assert_eq!(settings.name.as_deref(), Some("Scout"));
        assert_eq!(
            settings.capabilities,
            [BotCapability::Internet, BotCapability::Webhook]
        );
        assert_eq!(settings.location, BotLocation::Server);
        assert_eq!(settings.model, BotModelPolicy::default());

        let bare = create(json!({ "name": "Bare", "appearance": { "kind": "pip" } }))
            .unwrap()
            .1;
        assert!(bare.capabilities.is_empty());
    }

    #[test]
    fn docker_is_device_only_and_never_silently_downgraded() {
        let native = custom();
        assert_eq!(native.execution_mode, BotExecutionMode::Native);
        for location in ["desktop", "server"] {
            let error = patch(
                &native,
                None,
                json!({"location":location, "execution_mode":"docker"}),
            )
            .unwrap_err();
            assert_eq!(error.public_code(), "BOT_EXECUTION_MODE_LOCATION");
        }
        let isolated = patch(
            &native,
            None,
            json!({
                "location":"device", "device_id":"device-1", "execution_mode":"docker"
            }),
        )
        .unwrap();
        assert_eq!(
            patch(&isolated, None, json!({"name":"Renamed"}))
                .unwrap()
                .execution_mode,
            BotExecutionMode::Docker
        );
        assert!(patch(&isolated, None, json!({"location":"desktop"})).is_err());
        assert_eq!(
            patch(
                &isolated,
                None,
                json!({"location":"desktop", "execution_mode":"native"})
            )
            .unwrap()
            .execution_mode,
            BotExecutionMode::Native
        );
    }

    #[test]
    fn a_server_bot_cannot_hold_screen_or_shell_capabilities() {
        for capability in ["computer_use", "browser_use", "command_line", "encryption"] {
            let error = create(json!({
                "name": "Hands",
                "appearance": { "kind": "pip" },
                "capabilities": [capability],
            }))
            .unwrap_err();
            assert_eq!(
                code(&error),
                (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITY_LOCATION)
            );
        }
        let desktop = create(json!({
            "name": "Hands",
            "appearance": { "kind": "pip" },
            "capabilities": ["computer_use", "command_line"],
            "location": "desktop",
        }))
        .unwrap()
        .1;
        assert_eq!(desktop.location, BotLocation::Desktop);

        let error = patch(&desktop, None, json!({ "location": "server" })).unwrap_err();
        assert_eq!(
            code(&error),
            (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITY_LOCATION)
        );
        let moved = patch(
            &desktop,
            None,
            json!({ "location": "server", "capabilities": ["internet"] }),
        )
        .unwrap();
        assert_eq!(moved.location, BotLocation::Server);
    }

    #[test]
    fn encryption_is_refused_on_the_server_with_a_reason_and_allowed_elsewhere() {
        let error = create(json!({
            "name": "Vault",
            "appearance": { "kind": "pip" },
            "capabilities": ["encryption"],
        }))
        .unwrap_err();
        assert!(
            format!("{error:?}").contains("server must never read"),
            "{error:?}"
        );
        let bot = create(json!({
            "name": "Vault",
            "appearance": { "kind": "pip" },
            "capabilities": ["encryption"],
            "location": "desktop",
        }))
        .unwrap()
        .1;
        assert!(bot.has(BotCapability::Encryption));
    }

    #[test]
    fn a_device_holds_what_a_desktop_holds_and_a_move_keeps_the_capabilities() {
        for id in ["connectors", "skills", "image_generation", "encryption"] {
            let on_device = create(json!({
                "name": "Worker",
                "appearance": { "kind": "pip" },
                "capabilities": [id],
                "location": "device",
                "device_id": "device-1",
            }))
            .unwrap_or_else(|error| panic!("{id} on a device: {error:?}"))
            .1;
            assert_eq!(on_device.location, BotLocation::Device, "{id}");
            assert_eq!(on_device.capabilities.len(), 1, "{id}");
        }
        let desktop = create(json!({
            "name": "Worker",
            "appearance": { "kind": "pip" },
            "capabilities": ["skills", "command_line", "encryption"],
            "location": "desktop",
        }))
        .unwrap()
        .1;
        let moved = patch(
            &desktop,
            None,
            json!({ "location": "device", "device_id": "device-1" }),
        )
        .unwrap();
        assert_eq!(moved.location, BotLocation::Device);
        assert_eq!(moved.capabilities, desktop.capabilities);
    }

    #[test]
    fn flux_keeps_its_capabilities() {
        let flux = system_defaults(BotSystemKind::Flux);
        let error = patch(
            &flux,
            Some(BotSystemKind::Flux),
            json!({ "capabilities": ["internet"] }),
        )
        .unwrap_err();
        assert_eq!(
            code(&error),
            (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITIES_FIXED)
        );
        let unchanged = patch(
            &flux,
            Some(BotSystemKind::Flux),
            json!({ "capabilities": ["flowpilot"], "name": "Flux Prime" }),
        )
        .unwrap();
        assert_eq!(unchanged.name.as_deref(), Some("Flux Prime"));
        assert_eq!(unchanged.capabilities, [BotCapability::Flowpilot]);
        let error = patch(
            &flux,
            Some(BotSystemKind::Flux),
            json!({ "capabilities": [] }),
        )
        .unwrap_err();
        assert_eq!(
            code(&error),
            (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITIES_FIXED)
        );

        let general = system_defaults(BotSystemKind::General);
        let widened = patch(
            &general,
            Some(BotSystemKind::General),
            json!({ "capabilities": ["internet", "email"] }),
        )
        .unwrap();
        assert_eq!(
            widened.capabilities,
            [BotCapability::Internet, BotCapability::Email]
        );
    }

    #[test]
    fn patch_keeps_omitted_fields_and_clears_explicit_nulls() {
        let current = custom();
        let same = patch(&current, None, json!({})).unwrap();
        assert_eq!(same, current);

        let cleared = patch(&current, None, json!({ "description": null })).unwrap();
        assert_eq!(cleared.description, None);
        assert_eq!(cleared.name, current.name);

        let error = patch(&current, None, json!({ "name": null })).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);

        let renamed = system_defaults(BotSystemKind::General);
        let renamed = patch(
            &renamed,
            Some(BotSystemKind::General),
            json!({ "name": "Ada" }),
        )
        .unwrap();
        assert_eq!(renamed.name.as_deref(), Some("Ada"));
        let reset = patch(
            &renamed,
            Some(BotSystemKind::General),
            json!({ "name": null }),
        )
        .unwrap();
        assert_eq!(reset.name, None);
    }

    #[test]
    fn scoped_capabilities_and_locations_need_their_references() {
        let error =
            patch(&custom(), None, json!({ "capabilities": ["profile_apps"] })).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        let scoped = patch(
            &custom(),
            None,
            json!({ "capabilities": ["profile_apps"], "profile_id": "profile-1" }),
        )
        .unwrap();
        assert_eq!(scoped.profile_id.as_deref(), Some("profile-1"));
        let error = patch(&scoped, None, json!({ "profile_id": null })).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);

        let error = patch(&custom(), None, json!({ "location": "device" })).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        let device = patch(
            &custom(),
            None,
            json!({ "location": "device", "device_id": "device-1" }),
        )
        .unwrap();
        assert_eq!(device.device_id.as_deref(), Some("device-1"));
    }

    #[test]
    fn limits_name_the_field_and_the_size() {
        let long_name = "n".repeat(NAME_MAX_CHARS + 1);
        let error =
            create(json!({ "name": long_name, "appearance": { "kind": "pip" } })).unwrap_err();
        assert!(error.public_message().unwrap().contains("name"));
        assert!(
            create(json!({ "name": "ü".repeat(NAME_MAX_CHARS), "appearance": { "kind": "pip" } }))
                .is_ok()
        );
        assert!(create(json!({ "name": "   ", "appearance": { "kind": "pip" } })).is_err());

        let current = custom();
        for (field, max) in [
            ("description", DESCRIPTION_MAX_CHARS),
            ("instructions", INSTRUCTIONS_MAX_CHARS),
        ] {
            let error = patch(&current, None, json!({ field: "x".repeat(max + 1) })).unwrap_err();
            assert!(error.public_message().unwrap().contains(field), "{field}");
            assert!(patch(&current, None, json!({ field: "x".repeat(max) })).is_ok());
        }

        let fallbacks = patch(
            &current,
            None,
            json!({ "model": { "primary": "a", "fallbacks": ["b", "c", "d", "e"] } }),
        )
        .unwrap_err();
        assert!(fallbacks.public_message().unwrap().contains("fallbacks"));
        let long_model = patch(
            &current,
            None,
            json!({ "model": { "primary": "m".repeat(MODEL_ID_MAX_CHARS + 1) } }),
        )
        .unwrap_err();
        assert!(long_model.public_message().unwrap().contains("primary"));
        let empty_fallback = patch(&current, None, json!({ "model": { "fallbacks": [" "] } }));
        assert!(empty_fallback.is_err());
        let model = patch(
            &current,
            None,
            json!({ "model": { "primary": " a ", "fallbacks": ["b", "c", "d"] } }),
        )
        .unwrap()
        .model;
        assert_eq!(model.primary.as_deref(), Some("a"));
        assert_eq!(model.fallbacks, ["b", "c", "d"]);
    }

    #[test]
    fn appearance_is_a_bounded_object_with_a_kind() {
        let current = custom();
        for bad in [
            json!("pip"),
            json!({}),
            json!({ "kind": 3 }),
            json!({ "kind": " " }),
        ] {
            assert!(patch(&current, None, json!({ "appearance": bad })).is_err());
        }
        let padding = "x".repeat(APPEARANCE_MAX_BYTES);
        let error = patch(
            &current,
            None,
            json!({ "appearance": { "kind": "pip", "padding": padding } }),
        )
        .unwrap_err();
        assert!(error.public_message().unwrap().contains("bytes"));
        assert!(
            patch(
                &current,
                None,
                json!({ "appearance": { "kind": "pip", "frame": "round", "dark": true } }),
            )
            .is_ok()
        );
    }

    #[test]
    fn unknown_capabilities_are_refused_by_name() {
        let error = patch(
            &custom(),
            None,
            json!({ "capabilities": ["internet", "telepathy"] }),
        )
        .unwrap_err();
        let message = error.public_message().unwrap();
        assert!(message.contains("'telepathy'"), "{message}");
        assert!(message.contains("webhook"), "{message}");
    }

    #[test]
    fn built_in_bots_start_from_their_defaults() {
        let general = system_defaults(BotSystemKind::General);
        assert_eq!(general.appearance, json!({ "kind": "pip" }));
        assert_eq!(
            general.capabilities,
            [BotCapability::Internet, BotCapability::Memory]
        );
        assert_eq!(general.name, None);
        assert_eq!(general.embedding_model, None);
        let flux = system_defaults(BotSystemKind::Flux);
        assert_eq!(flux.appearance, json!({ "kind": "flux" }));
        assert_eq!(flux.capabilities, [BotCapability::Flowpilot]);
        assert_eq!(flux.location, BotLocation::Server);
        assert!(check_rules(&general).is_ok() && check_rules(&flux).is_ok());
    }

    #[test]
    fn flowpilot_is_never_switched_on_and_its_holders_keep_their_capabilities() {
        let fixed = (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITIES_FIXED);
        let asked = json!({
            "name": "Builder",
            "appearance": { "kind": "flux" },
            "capabilities": ["flowpilot"],
        });
        let error = create(asked.clone()).unwrap_err();
        assert_eq!(code(&error), fixed);
        assert!(error.public_message().unwrap().contains("'flowpilot'"));
        let error = patch(
            &custom(),
            None,
            json!({ "capabilities": ["internet", "flowpilot"] }),
        )
        .unwrap_err();
        assert_eq!(code(&error), fixed);
        let error = patch(
            &system_defaults(BotSystemKind::General),
            Some(BotSystemKind::General),
            json!({ "capabilities": ["flowpilot"] }),
        )
        .unwrap_err();
        assert_eq!(code(&error), fixed);

        let (_, moved) = moved_settings(serde_json::from_value(asked).unwrap()).unwrap();
        assert_eq!(moved.capabilities, [BotCapability::Flowpilot]);
        let duplicate = BotSettings {
            name: Some("Flux copy".into()),
            ..system_defaults(BotSystemKind::Flux)
        };
        for capabilities in [
            json!([]),
            json!(["flowpilot", "internet"]),
            json!(["internet"]),
        ] {
            let error =
                patch(&duplicate, None, json!({ "capabilities": capabilities })).unwrap_err();
            assert_eq!(code(&error), fixed, "{capabilities}");
            assert!(error.public_message().unwrap().contains("'flowpilot'"));
        }
        let renamed = patch(
            &duplicate,
            None,
            json!({ "capabilities": ["FLOWPILOT"], "name": "Builder", "location": "desktop" }),
        )
        .unwrap();
        assert_eq!(renamed.capabilities, [BotCapability::Flowpilot]);
        assert_eq!(renamed.location, BotLocation::Desktop);
    }

    #[test]
    fn tool_capabilities_are_accepted_on_every_location_and_kept_in_canonical_order() {
        let ids: Vec<&str> = vec![
            "delegation",
            "channels",
            "knowledge",
            "data_connections",
            "documents",
        ];
        for location in ["server", "desktop", "device"] {
            let (_, bot) = create(json!({
                "name": "Worker",
                "appearance": { "kind": "pip" },
                "location": location,
                "device_id": (location == "device").then_some("device-1"),
                "capabilities": ids,
            }))
            .unwrap();
            let wire: Vec<&str> = bot.capabilities.iter().map(|c| c.as_str()).collect();
            let mut expected = ids.clone();
            expected.reverse();
            assert_eq!(wire, expected, "{location}");
            let next = patch(&bot, None, json!({ "capabilities": ["knowledge"] })).unwrap();
            assert_eq!(next.capabilities, [BotCapability::Knowledge]);
        }
    }

    #[test]
    fn the_stt_tts_image_and_vision_pins_are_optional_bounded_ids_with_patch_semantics() {
        let (_, pinned) = create(json!({
            "name": "Pinned",
            "appearance": { "kind": "pip" },
            "stt_model": " stt-a ",
            "tts_model": "tts-a",
            "image_model": "image-a",
            "vision_model": "vision-a",
            "subtask_model": " small-a ",
        }))
        .unwrap();
        assert_eq!(pinned.stt_model.as_deref(), Some("stt-a"));
        assert_eq!(pinned.tts_model.as_deref(), Some("tts-a"));
        assert_eq!(pinned.image_model.as_deref(), Some("image-a"));
        assert_eq!(pinned.vision_model.as_deref(), Some("vision-a"));
        assert_eq!(pinned.subtask_model.as_deref(), Some("small-a"));

        let renamed = patch(&pinned, None, json!({ "name": "Pinned 2" })).unwrap();
        assert_eq!(renamed.stt_model.as_deref(), Some("stt-a"));
        assert_eq!(renamed.vision_model.as_deref(), Some("vision-a"));
        assert_eq!(renamed.subtask_model.as_deref(), Some("small-a"));

        let set = patch(&pinned, None, json!({ "tts_model": "tts-b" })).unwrap();
        assert_eq!(set.tts_model.as_deref(), Some("tts-b"));
        assert_eq!(pinned.changed_fields(&set), ["tts_model"]);

        let cleared = patch(
            &pinned,
            None,
            json!({ "stt_model": null, "image_model": " ", "vision_model": null, "subtask_model": null }),
        )
        .unwrap();
        assert_eq!(cleared.stt_model, None);
        assert_eq!(cleared.image_model, None);
        assert_eq!(cleared.vision_model, None);
        assert_eq!(cleared.subtask_model, None);
        assert_eq!(cleared.tts_model.as_deref(), Some("tts-a"));
        assert_eq!(
            pinned.changed_fields(&cleared),
            ["stt_model", "image_model", "vision_model", "subtask_model"]
        );

        for field in [
            "stt_model",
            "tts_model",
            "image_model",
            "vision_model",
            "subtask_model",
        ] {
            let error = patch(
                &pinned,
                None,
                json!({ field: "e".repeat(MODEL_ID_MAX_CHARS + 1) }),
            )
            .unwrap_err();
            assert!(error.public_message().unwrap().contains(field));
        }
    }

    #[test]
    fn memory_is_a_capability_and_the_embedding_model_an_optional_bounded_id() {
        let (_, keeper) = create(json!({
            "name": "Keeper",
            "appearance": { "kind": "pip" },
            "capabilities": ["memory"],
            "embedding_model": " embed-a ",
        }))
        .unwrap();
        assert_eq!(keeper.capabilities, [BotCapability::Memory]);
        assert_eq!(keeper.embedding_model.as_deref(), Some("embed-a"));
        let renamed = patch(&keeper, None, json!({ "name": "Keeper 2" })).unwrap();
        assert_eq!(renamed.embedding_model.as_deref(), Some("embed-a"));
        let cleared = patch(&keeper, None, json!({ "embedding_model": null })).unwrap();
        assert_eq!(cleared.embedding_model, None);
        assert_eq!(keeper.changed_fields(&cleared), ["embedding_model"]);
        let blank = patch(&keeper, None, json!({ "embedding_model": " " })).unwrap();
        assert_eq!(blank.embedding_model, None);
        let longest = "e".repeat(MODEL_ID_MAX_CHARS);
        assert!(patch(&keeper, None, json!({ "embedding_model": longest })).is_ok());
        let error = patch(
            &keeper,
            None,
            json!({ "embedding_model": "e".repeat(MODEL_ID_MAX_CHARS + 1) }),
        )
        .unwrap_err();
        assert!(error.public_message().unwrap().contains("embedding_model"));
        let forgetful = patch(&keeper, None, json!({ "capabilities": [] })).unwrap();
        assert!(forgetful.capabilities.is_empty());
    }
}
