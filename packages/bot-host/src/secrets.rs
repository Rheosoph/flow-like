//! A bot's secrets read during its turn with the bot's runtime token. The hub hands a SERVER
//! secret's value; a SEALED one only as the envelope the owner sealed to the bot's keys, which a
//! host opens with its [`E2eHost`]. A host without one says so instead of guessing.

use crate::e2e::E2eHost;
use crate::hub::TurnHub;
use flow_like_bot_tools::{ToolError, error::code};
use flow_like_types::{Result, anyhow};
use serde_json::{Value, json};

/// The hub's answer on the value route when the secret is sealed to the bot's own key.
pub const CODE_SECRET_SEALED: &str = "BOT_SECRET_SEALED";

pub fn value_route(turn_id: &str, name: &str) -> String {
    format!("bots/turns/{turn_id}/secrets/{name}")
}

pub fn sealed_route(turn_id: &str, name: &str) -> String {
    format!("bots/turns/{turn_id}/secrets/{name}/sealed")
}

/// A sealed secret asked for on a host that holds no key of the bot.
pub fn sealed_without_key(name: &str) -> ToolError {
    ToolError::new(
        code::SECRET_SEALED,
        format!(
            "The secret '{name}' is sealed to this bot's own key, and this host holds no key of the bot, so it cannot open it. Tell the owner: '{name}' only works where the bot's key lives (the desktop or device that runs the bot with end-to-end encryption)."
        ),
    )
    .with_details(json!({ "name": name }))
}

/// The envelope of a sealed secret as the sealed route returns it.
pub fn envelope_of(read: &Value, name: &str) -> Result<Value> {
    read.get("envelope")
        .filter(|envelope| envelope.is_object())
        .cloned()
        .ok_or_else(|| anyhow!("The hub returned no envelope for the sealed secret '{name}'"))
}

/// Secret `name` of `bot_id` for turn `turn_id`, read through `hub` as the bot; `None` when the bot
/// has no such secret.
pub async fn read_turn_secret(
    hub: &dyn TurnHub,
    e2e: Option<&E2eHost>,
    bot_id: &str,
    turn_id: &str,
    name: &str,
) -> Result<Option<String>> {
    let failure = match hub
        .write(&value_route(turn_id, name), Vec::new(), Value::Null)
        .await
    {
        Ok(read) => {
            return Ok(read.get("value").and_then(Value::as_str).map(str::to_owned));
        }
        Err(failure) if failure.status == Some(404) => return Ok(None),
        Err(failure) => failure,
    };
    if failure.code.as_deref() != Some(CODE_SECRET_SEALED) {
        return Err(anyhow!("Reading secret '{name}' failed: {failure}"));
    }
    let Some(e2e) = e2e else {
        return Err(sealed_without_key(name).into());
    };
    let read = match hub
        .write(&sealed_route(turn_id, name), Vec::new(), Value::Null)
        .await
    {
        Ok(read) => read,
        Err(failure) if failure.status == Some(404) => return Ok(None),
        Err(failure) => return Err(anyhow!("Reading sealed secret '{name}' failed: {failure}")),
    };
    let envelope = envelope_of(&read, name)?;
    Ok(Some(e2e.open_secret(bot_id, name, &envelope)?))
}
