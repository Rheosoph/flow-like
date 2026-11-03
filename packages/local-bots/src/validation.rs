//! The API's bot and secret rules (`routes/bots/validation.rs`, `routes/bots/secrets.rs`), with
//! what a bot without an account can't have refused: a server or device location, a device.

use crate::{
    Result,
    error::{Error, codes},
    model::{CreateBotRequest, UpdateBotRequest},
};
use flow_like_types::{
    bot::{BotCapability, BotLocation, BotModelPolicy, BotSystemKind},
    data_classification::DataSensitivity,
};
use serde_json::{Value, json};

pub(crate) const NAME_MAX_CHARS: usize = 64;
pub(crate) const DESCRIPTION_MAX_CHARS: usize = 2_000;
pub(crate) const INSTRUCTIONS_MAX_CHARS: usize = 8_000;
pub(crate) const PRESET_ID_MAX_CHARS: usize = 128;
pub(crate) const REFERENCE_ID_MAX_CHARS: usize = 256;
pub(crate) const APPEARANCE_MAX_BYTES: usize = 16 * 1024;
pub(crate) const MODEL_ID_MAX_CHARS: usize = 256;
pub(crate) const MAX_FALLBACKS: usize = 3;
pub(crate) const MAX_LOCAL_BOTS: i64 = 50;
pub(crate) const MAX_APPS_PER_BOT: i64 = 200;
pub(crate) const SECRET_NAME_MAX_LEN: usize = 64;
pub(crate) const MAX_SECRETS_PER_BOT: i64 = 50;
pub(crate) const SECRET_VALUE_MAX_BYTES: usize = 16 * 1024;
pub(crate) const SECRET_DESCRIPTION_MAX_CHARS: usize = 2_000;

pub(crate) const SIGN_IN_TO_LEAVE_THE_DESKTOP: &str =
    "Sign in to run a bot on the server or a device";

/// Everything a person can change on a local bot, validated and normalized.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BotSettings {
    /// Null only on a built-in the person never renamed.
    pub name: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub appearance: Value,
    pub model: BotModelPolicy,
    pub embedding_model: Option<String>,
    pub classification: DataSensitivity,
    pub capabilities: Vec<BotCapability>,
    pub profile_id: Option<String>,
}

impl BotSettings {
    pub fn has(&self, capability: BotCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// What a built-in starts with, like the API's: no name (the client shows the localized one),
/// Assistant's look with `internet` and `memory` on, Flux's look with `flowpilot`.
pub(crate) fn built_in(kind: BotSystemKind) -> BotSettings {
    let (look, capabilities) = match kind {
        BotSystemKind::General => ("pip", vec![BotCapability::Internet, BotCapability::Memory]),
        BotSystemKind::Flux => ("flux", kind.fixed_capabilities().to_vec()),
    };
    BotSettings {
        name: None,
        description: None,
        instructions: None,
        appearance: json!({ "kind": look }),
        model: BotModelPolicy::default(),
        embedding_model: None,
        classification: DataSensitivity::default(),
        capabilities,
        profile_id: None,
    }
}

/// The preset id and settings of a new bot.
pub(crate) fn create_settings(request: CreateBotRequest) -> Result<(Option<String>, BotSettings)> {
    let preset_id = optional_text("preset_id", request.preset_id, PRESET_ID_MAX_CHARS)?;
    let settings = BotSettings {
        name: Some(bot_name(&request.name)?),
        description: optional_text("description", request.description, DESCRIPTION_MAX_CHARS)?,
        instructions: optional_text("instructions", request.instructions, INSTRUCTIONS_MAX_CHARS)?,
        appearance: appearance(request.appearance)?,
        model: request
            .model
            .map(model_policy)
            .transpose()?
            .unwrap_or_default(),
        embedding_model: embedding_model(request.embedding_model)?,
        classification: request.classification.unwrap_or_default(),
        capabilities: request
            .capabilities
            .map(capabilities)
            .transpose()?
            .unwrap_or_default(),
        profile_id: reference_id("profile_id", request.profile_id)?,
    };
    keeps_fixed_capabilities(&[], &settings.capabilities)?;
    stays_on_the_desktop(request.location, request.device_id)?;
    check_rules(&settings)?;
    Ok((preset_id, settings))
}

/// `current` with `patch` applied: omitted fields stay, `null` clears nullable fields (a
/// built-in's name too, which brings back its default), and the result must satisfy every rule
/// a new bot does.
pub(crate) fn apply_patch(
    current: &BotSettings,
    system: Option<BotSystemKind>,
    patch: UpdateBotRequest,
) -> Result<BotSettings> {
    let mut next = current.clone();
    if let Some(name) = patch.name {
        next.name = patched_name(name, system)?;
    }
    if let Some(description) = patch.description {
        next.description = optional_text("description", description, DESCRIPTION_MAX_CHARS)?;
    }
    if let Some(instructions) = patch.instructions {
        next.instructions = optional_text("instructions", instructions, INSTRUCTIONS_MAX_CHARS)?;
    }
    if let Some(look) = patch.appearance {
        next.appearance = appearance(look)?;
    }
    if let Some(policy) = patch.model {
        next.model = model_policy(policy)?;
    }
    if let Some(model) = patch.embedding_model {
        next.embedding_model = embedding_model(model)?;
    }
    if let Some(classification) = patch.classification {
        next.classification = classification;
    }
    if let Some(ids) = patch.capabilities {
        next.capabilities = capabilities(ids)?;
        keeps_fixed_capabilities(&current.capabilities, &next.capabilities)?;
    }
    if let Some(profile_id) = patch.profile_id {
        next.profile_id = reference_id("profile_id", profile_id)?;
    }
    stays_on_the_desktop(patch.location, patch.device_id.flatten())?;
    check_rules(&next)?;
    Ok(next)
}

/// A new name; `null` brings a built-in's default back, a bot the person made needs one.
fn patched_name(name: Option<String>, system: Option<BotSystemKind>) -> Result<Option<String>> {
    match name {
        Some(name) => Ok(Some(bot_name(&name)?)),
        None if system.is_some() => Ok(None),
        None => Err(Error::invalid(
            "A bot you created needs a name; send a name instead of null",
        )),
    }
}

/// The Bit id of the embedding model; empty lets the host choose.
fn embedding_model(value: Option<String>) -> Result<Option<String>> {
    optional_text("embedding_model", value, MODEL_ID_MAX_CHARS)
}

/// The API's rule for a fixed capability (`flowpilot`): only bots that already hold it have it,
/// and they keep the capabilities they have.
fn keeps_fixed_capabilities(before: &[BotCapability], after: &[BotCapability]) -> Result<()> {
    if before.iter().any(|capability| capability.is_fixed()) && after != before {
        return Err(Error::new(
            codes::BOT_CAPABILITIES_FIXED,
            "The capabilities of a bot with 'flowpilot' can't be changed",
        ));
    }
    match after
        .iter()
        .find(|capability| capability.is_fixed() && !before.contains(capability))
    {
        Some(capability) => Err(Error::new(
            codes::BOT_CAPABILITIES_FIXED,
            format!(
                "Only Flux and its duplicates hold '{}'; duplicate Flux to get a bot with it",
                capability.as_str()
            ),
        )),
        None => Ok(()),
    }
}

/// A bot without an account runs on this desktop: no other location, no managed device.
fn stays_on_the_desktop(location: Option<BotLocation>, device_id: Option<String>) -> Result<()> {
    let elsewhere = location.is_some_and(|location| location != BotLocation::Desktop);
    if elsewhere || reference_id("device_id", device_id)?.is_some() {
        return Err(Error::account_required(SIGN_IN_TO_LEAVE_THE_DESKTOP));
    }
    Ok(())
}

fn check_rules(settings: &BotSettings) -> Result<()> {
    if settings.has(BotCapability::ProfileApps) && settings.profile_id.is_none() {
        return Err(Error::invalid(
            "The 'profile_apps' capability needs a profile_id",
        ));
    }
    Ok(())
}

fn too_long(field: &str, max_chars: usize, chars: usize) -> Error {
    Error::invalid(format!(
        "Bot {field} has {chars} characters; at most {max_chars} are allowed"
    ))
}

fn bot_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid("Bot name must not be empty"));
    }
    let chars = name.chars().count();
    if chars > NAME_MAX_CHARS {
        return Err(too_long("name", NAME_MAX_CHARS, chars));
    }
    Ok(name.to_string())
}

/// Trimmed text, `None` when empty.
fn optional_text(field: &str, value: Option<String>, max_chars: usize) -> Result<Option<String>> {
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

fn reference_id(field: &str, value: Option<String>) -> Result<Option<String>> {
    optional_text(field, value, REFERENCE_ID_MAX_CHARS)
}

fn appearance(value: Value) -> Result<Value> {
    let Value::Object(fields) = &value else {
        return Err(Error::invalid(
            "Bot appearance must be a JSON object with a string 'kind'",
        ));
    };
    if !fields
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.trim().is_empty())
    {
        return Err(Error::invalid(
            "Bot appearance needs a non-empty string 'kind'",
        ));
    }
    let size = serde_json::to_vec(&value)?.len();
    if size > APPEARANCE_MAX_BYTES {
        return Err(Error::invalid(format!(
            "Bot appearance is {size} bytes; at most {APPEARANCE_MAX_BYTES} are allowed"
        )));
    }
    Ok(value)
}

fn model_id(field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::invalid(format!(
            "Bot model {field} must not be empty"
        )));
    }
    let chars = value.chars().count();
    if chars > MODEL_ID_MAX_CHARS {
        return Err(Error::invalid(format!(
            "Bot model {field} has {chars} characters; at most {MODEL_ID_MAX_CHARS} are allowed"
        )));
    }
    Ok(value.to_string())
}

fn model_policy(policy: BotModelPolicy) -> Result<BotModelPolicy> {
    if policy.fallbacks.len() > MAX_FALLBACKS {
        return Err(Error::invalid(format!(
            "Bot model has {} fallbacks; at most {MAX_FALLBACKS} are allowed",
            policy.fallbacks.len()
        )));
    }
    Ok(BotModelPolicy {
        primary: policy
            .primary
            .filter(|primary| !primary.trim().is_empty())
            .map(|primary| model_id("primary", &primary))
            .transpose()?,
        fallbacks: policy
            .fallbacks
            .iter()
            .map(|fallback| model_id("fallback", fallback))
            .collect::<Result<_>>()?,
    })
}

/// Known ids in their canonical order, without duplicates.
fn capabilities(ids: Vec<String>) -> Result<Vec<BotCapability>> {
    let mut parsed = Vec::with_capacity(ids.len());
    for id in ids {
        let capability = BotCapability::parse(&id).ok_or_else(|| {
            let known = BotCapability::ALL
                .iter()
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Error::invalid(format!("Unknown bot capability '{id}'; use one of {known}"))
        })?;
        parsed.push(capability);
    }
    Ok(BotCapability::ALL
        .into_iter()
        .filter(|capability| parsed.contains(capability))
        .collect())
}

/// Capability ids a stored bot holds; ids this version doesn't know are skipped.
pub(crate) fn stored_capabilities(ids: &[String]) -> Vec<BotCapability> {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| {
            ids.iter()
                .any(|id| BotCapability::parse(id) == Some(*capability))
        })
        .collect()
}

pub(crate) fn capability_ids(capabilities: &[BotCapability]) -> Vec<&'static str> {
    BotCapability::ALL
        .into_iter()
        .filter(|capability| capabilities.contains(capability))
        .map(BotCapability::as_str)
        .collect()
}

pub(crate) fn app_id(app_id: &str) -> Result<()> {
    if app_id.is_empty() || app_id.chars().count() > REFERENCE_ID_MAX_CHARS {
        return Err(Error::invalid(format!(
            "An app id has 1 to {REFERENCE_ID_MAX_CHARS} characters"
        )));
    }
    Ok(())
}

pub(crate) fn secret_name(name: &str) -> Result<()> {
    let mut bytes = name.bytes();
    let valid = name.len() <= SECRET_NAME_MAX_LEN
        && bytes.next().is_some_and(|first| first.is_ascii_uppercase())
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        return Ok(());
    }
    Err(Error::invalid(format!(
        "Secret name '{name}' must start with A-Z and continue with up to 63 of A-Z, 0-9 or _"
    )))
}

pub(crate) fn secret_value(name: &str, value: &str) -> Result<()> {
    if value.len() > SECRET_VALUE_MAX_BYTES {
        return Err(Error::invalid(format!(
            "Secret '{name}' is {} bytes; at most {SECRET_VALUE_MAX_BYTES} are allowed",
            value.len()
        )));
    }
    Ok(())
}

/// Trimmed, `None` when empty.
pub(crate) fn secret_description(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    let chars = value.chars().count();
    if chars > SECRET_DESCRIPTION_MAX_CHARS {
        return Err(Error::invalid(format!(
            "Secret description has {chars} characters; at most {SECRET_DESCRIPTION_MAX_CHARS} are allowed"
        )));
    }
    Ok(Some(value).filter(|value| !value.is_empty()))
}
