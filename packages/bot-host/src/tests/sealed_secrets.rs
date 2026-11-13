use super::{DeviceHub, checkpoint_encryption::Vault};
use crate::{
    e2e::E2eHost,
    hub::HubFailure,
    secrets::{CODE_SECRET_SEALED, read_turn_secret, sealed_route, value_route},
};
use flow_like_bot_tools::{ToolError, error::code};
use flow_like_device_crypto::inbox::{
    InboxKeyring, PublicInboxKey, RecipientKey, SecretScope, seal_secret,
};
use serde_json::{Value, json};
use std::sync::Arc;

const TURN: &str = "bt_1";

fn sealed_for(bot_id: &str, name: &str, key: &PublicInboxKey, value: &str) -> Value {
    let keys = [RecipientKey {
        key_id: key.key_id.clone(),
        public_key: key.public_key.clone(),
    }];
    seal_secret(&SecretScope { bot_id, name }, &keys, value.as_bytes()).unwrap()
}

fn hub_serving(name: &str, envelope: Value) -> Arc<DeviceHub> {
    let hub = Arc::new(DeviceHub::default());
    let mut writes = hub.writes.lock().unwrap();
    writes.insert(
        value_route(TURN, name),
        Err(HubFailure::answered(
            409,
            Some(CODE_SECRET_SEALED),
            "sealed to the bot's own key",
        )),
    );
    writes.insert(
        sealed_route(TURN, name),
        Ok(json!({ "name": name, "envelope": envelope })),
    );
    drop(writes);
    hub
}

fn host(hub: Arc<DeviceHub>, vault: Arc<Vault>) -> E2eHost {
    E2eHost::new(hub, vault, "device:test".into())
}

fn tool_code(error: flow_like_types::Error) -> &'static str {
    ToolError::from(error).code
}

#[tokio::test]
async fn a_sealed_secret_opens_on_the_host_holding_the_bots_key() {
    let vault = Arc::new(Vault::default());
    let (key, _) = InboxKeyring::new(vault.clone()).ensure("bot_a").unwrap();
    let hub = hub_serving("API_KEY", sealed_for("bot_a", "API_KEY", &key, "sk-1"));
    let e2e = host(hub.clone(), vault);
    let value = read_turn_secret(hub.as_ref(), Some(&e2e), "bot_a", TURN, "API_KEY")
        .await
        .unwrap();
    assert_eq!(value.as_deref(), Some("sk-1"));
}

#[tokio::test]
async fn an_envelope_replayed_under_another_name_or_bot_asks_to_enter_it_again() {
    let vault = Arc::new(Vault::default());
    let ring = InboxKeyring::new(vault.clone());
    let (key, _) = ring.ensure("bot_a").unwrap();
    let envelope = sealed_for("bot_a", "API_KEY", &key, "sk-1");
    let hub = hub_serving("OTHER_KEY", envelope.clone());
    let e2e = host(hub.clone(), vault.clone());
    let error = read_turn_secret(hub.as_ref(), Some(&e2e), "bot_a", TURN, "OTHER_KEY")
        .await
        .unwrap_err();
    assert_eq!(tool_code(error), code::SECRET_REENTER);

    ring.ensure("bot_b").unwrap();
    let other_bot = e2e.open_secret("bot_b", "API_KEY", &envelope).unwrap_err();
    assert_eq!(other_bot.code, code::SECRET_REENTER);
    assert_eq!(other_bot.details, Some(json!({ "name": "API_KEY" })));
}

#[tokio::test]
async fn a_host_whose_key_is_not_in_the_envelope_asks_to_enter_it_again() {
    let elsewhere = Arc::new(Vault::default());
    let (old_key, _) = InboxKeyring::new(elsewhere).ensure("bot_a").unwrap();
    let vault = Arc::new(Vault::default());
    InboxKeyring::new(vault.clone()).ensure("bot_a").unwrap();
    let hub = hub_serving("API_KEY", sealed_for("bot_a", "API_KEY", &old_key, "sk-1"));
    let e2e = host(hub.clone(), vault);
    let error = read_turn_secret(hub.as_ref(), Some(&e2e), "bot_a", TURN, "API_KEY")
        .await
        .unwrap_err();
    let error = ToolError::from(error);
    assert_eq!(error.code, code::SECRET_REENTER);
    assert!(
        error.message.contains("enter 'API_KEY' again"),
        "{}",
        error.message
    );
    assert!(error.message.contains("does not hold"), "{}", error.message);
}

#[tokio::test]
async fn a_host_without_keys_says_the_secret_is_sealed() {
    let vault = Arc::new(Vault::default());
    let (key, _) = InboxKeyring::new(vault).ensure("bot_a").unwrap();
    let hub = hub_serving("API_KEY", sealed_for("bot_a", "API_KEY", &key, "sk-1"));
    let error = read_turn_secret(hub.as_ref(), None, "bot_a", TURN, "API_KEY")
        .await
        .unwrap_err();
    assert_eq!(tool_code(error), code::SECRET_SEALED);
}

#[tokio::test]
async fn server_values_pass_through_and_missing_secrets_are_none() {
    let hub = Arc::new(DeviceHub::default());
    hub.writes.lock().unwrap().insert(
        value_route(TURN, "TOKEN"),
        Ok(json!({ "name": "TOKEN", "value": "t-1" })),
    );
    hub.writes.lock().unwrap().insert(
        value_route(TURN, "LOST"),
        Err(HubFailure::answered(
            409,
            Some("BOT_TURN_LEASE_LOST"),
            "not running",
        )),
    );
    let read = |name: &'static str| {
        let hub = hub.clone();
        async move { read_turn_secret(hub.as_ref(), None, "bot_a", TURN, name).await }
    };
    assert_eq!(read("TOKEN").await.unwrap().as_deref(), Some("t-1"));
    assert_eq!(read("MISSING").await.unwrap(), None);
    let lost = read("LOST").await.unwrap_err().to_string();
    assert!(lost.contains("BOT_TURN_LEASE_LOST"), "{lost}");
}
