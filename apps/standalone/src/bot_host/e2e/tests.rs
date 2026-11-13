use super::{DeviceKeyHub, DeviceKeyVault, host, publish_keys};
use crate::bot_host::hub::{DeviceApi, DeviceTurnHub};
use async_trait::async_trait;
use chrono::Utc;
use flow_like::flow::copilot::run_recovery::BotRunRecovery;
use flow_like_bot_host::{hub::TurnHub, recovery::HubRecovery};
use flow_like_device_crypto::inbox::{InboxKeyVault, InboxKeyring};
use flow_like_types::bot_turn::{
    BotRunCheckpoint, BotTurn, TurnAudience, TurnHostKind, TurnStatus,
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
    time::Duration,
};

const DEVICE: &str = "dev1";
const SEALING: &str = "bot-sealing";
const PLAIN: &str = "bot-plain";
const CONVERSATION: &str = "conv-1";
const TURN: &str = "bt_1";

type Recorded = Vec<(Method, String, Option<Value>)>;

/// The device routes of a hub: a poll listing one encryption bot and one plain bot, key
/// publishing, a claim and the checkpoint routes, which keep what the device saved.
#[derive(Default)]
struct Hub {
    requests: Mutex<Recorded>,
    checkpoint: Mutex<Option<Value>>,
}

fn turn(status: TurnStatus) -> BotTurn {
    BotTurn {
        id: TURN.into(),
        bot_id: SEALING.into(),
        conversation_id: CONVERSATION.into(),
        trigger_message_id: "m1".into(),
        host_kind: TurnHostKind::Device,
        status,
        attempts: 1,
        host_id: Some(format!("device:{DEVICE}")),
        lease_until: None,
        reply_message_id: None,
        error_code: None,
        hops: 0,
        owner_id: "owner".into(),
        requester_id: None,
        requester_name: None,
        audience: TurnAudience::Owner,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        batched: Vec::new(),
    }
}

impl Hub {
    fn requests(&self) -> Recorded {
        self.requests.lock().unwrap().clone()
    }

    fn calls(&self, tail: &str) -> Vec<Option<Value>> {
        self.requests()
            .into_iter()
            .filter(|(_, path, _)| path.split('?').next().unwrap().ends_with(tail))
            .map(|(_, _, body)| body)
            .collect()
    }

    fn answer(&self, method: &Method, path: &str, body: Option<&Value>) -> Value {
        let route = path.split('?').next().unwrap();
        let bots = format!("/devices/{DEVICE}/bots/");
        match (method.as_str(), route.strip_prefix(&bots).unwrap_or(route)) {
            ("POST", "poll") => json!({
                "bots": [
                    { "id": SEALING, "capabilities": ["encryption"], "updated_at": "2026-11-12T00:00:00Z" },
                    { "id": PLAIN, "capabilities": [], "updated_at": "2026-11-12T00:00:00Z" },
                ],
                "turns": [],
                "next_poll_ms": 30_000,
            }),
            ("POST", "inbox-keys") => body.cloned().unwrap(),
            ("POST", tail) if tail.ends_with("/claim") => json!({
                "turn": turn(TurnStatus::Running),
                "host_id": format!("device:{DEVICE}"),
                "conversation": { "id": CONVERSATION, "kind": "direct" },
                "bot": { "bot_id": SEALING, "name": "Sealer" },
                "models": { "model_id": "m", "completion": {} },
                "bot_token": { "token": "bot-token", "expires_at": Utc::now().timestamp() + 600 },
                "executor_token": { "token": "executor-token", "expires_at": Utc::now().timestamp() + 600 },
                "usage": { "operation_id": "op-1", "deadline_ms": 1 },
                "deadline": Utc::now().timestamp() + 600,
            }),
            ("PUT", tail) if tail.ends_with("/checkpoint") => {
                *self.checkpoint.lock().unwrap() = Some(body.unwrap()["checkpoint"].clone());
                json!({ "saved": true })
            }
            ("GET", tail) if tail.ends_with("/checkpoint") => {
                json!({ "checkpoint": self.checkpoint.lock().unwrap().clone() })
            }
            _ => panic!("unexpected device call {method} {path}"),
        }
    }
}

#[async_trait]
impl DeviceApi for Hub {
    fn base_url(&self) -> &str {
        "https://hub.test/api/v1"
    }

    fn device_id(&self) -> &str {
        DEVICE
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<(u16, Vec<u8>)> {
        let answer = self.answer(&method, path, body);
        self.requests
            .lock()
            .unwrap()
            .push((method, path.to_owned(), body.cloned()));
        Ok((200, serde_json::to_vec(&answer)?))
    }
}

struct Device {
    api: Arc<Hub>,
    hub: DeviceTurnHub,
    state: tempfile::TempDir,
}

fn device() -> Device {
    let api = Arc::new(Hub::default());
    let hub = DeviceTurnHub::new(api.clone(), format!("device:{DEVICE}"));
    Device {
        api,
        hub,
        state: tempfile::tempdir().unwrap(),
    }
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn stored_seed(vault: &DeviceKeyVault, bot: &str) -> String {
    let stored: Value = serde_json::from_slice(&vault.load(bot).unwrap().unwrap()).unwrap();
    stored["keys"][0]["seed"].as_str().unwrap().to_owned()
}

#[test]
fn each_bots_keys_live_in_a_private_file_that_outlasts_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let vault = DeviceKeyVault::open(state.path()).unwrap();
    assert!(vault.load(SEALING).unwrap().is_none());
    vault.store(SEALING, b"first").unwrap();
    vault.store(SEALING, b"second").unwrap();
    vault.store("../../escape", b"other").unwrap();
    let directory = state.path().join(super::VAULT_DIR);
    assert_eq!(mode(&directory), 0o700);
    let files: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    for file in &files {
        assert_eq!(mode(file), 0o600, "{file:?}");
        assert_eq!(file.parent().unwrap(), directory);
    }

    let reopened = Arc::new(DeviceKeyVault::open(state.path()).unwrap());
    assert_eq!(
        reopened.load(SEALING).unwrap().unwrap().as_slice(),
        b"second"
    );
    assert_eq!(
        reopened.load("../../escape").unwrap().unwrap().as_slice(),
        b"other"
    );
    let key = InboxKeyring::new(reopened.clone()).ensure(PLAIN).unwrap().0;
    let restarted = InboxKeyring::new(Arc::new(DeviceKeyVault::open(state.path()).unwrap()));
    assert_eq!(restarted.active(PLAIN).unwrap(), Some(key));
    reopened.remove(SEALING).unwrap();
    reopened.remove(SEALING).unwrap();
    assert!(reopened.load(SEALING).unwrap().is_none());
}

#[tokio::test]
async fn the_device_publishes_only_its_encryption_bots_public_keys_as_the_device() {
    let device = device();
    let e2e = host(device.state.path(), &device.hub).unwrap();
    device.hub.list().await.unwrap();
    e2e.sync().await.unwrap();
    e2e.sync().await.unwrap();

    let published = device.api.calls("/bots/inbox-keys");
    assert_eq!(published.len(), 1, "published once");
    let body = published[0].clone().unwrap();
    let status = e2e.status(SEALING);
    assert!(status.enabled && status.published);
    assert_eq!(body["principal_id"], SEALING);
    assert_eq!(body["key_id"], status.key_id.unwrap().as_str());
    assert_eq!(body["algorithm"], "hpke-x25519-sha256-chacha20poly1305");
    assert!(!e2e.status(PLAIN).enabled);

    let vault = DeviceKeyVault::open(device.state.path()).unwrap();
    assert!(vault.load(PLAIN).unwrap().is_none());
    let seed = stored_seed(&vault, SEALING);
    let sent = format!("{:?}", device.api.requests());
    assert!(!sent.contains(&seed), "the private key left the device");
}

#[tokio::test]
async fn keys_are_published_as_soon_as_a_poll_lists_the_bots() {
    let device = device();
    let e2e = host(device.state.path(), &device.hub).unwrap();
    let publishing = tokio::spawn(publish_keys(device.hub.clone(), e2e));
    device.hub.list().await.unwrap();
    for _ in 0..100 {
        if !device.api.calls("/bots/inbox-keys").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    publishing.abort();
    assert_eq!(device.api.calls("/bots/inbox-keys").len(), 1);
}

#[tokio::test]
async fn a_sealed_checkpoint_round_trips_through_the_device_routes() {
    let device = device();
    let e2e = host(device.state.path(), &device.hub).unwrap();
    device.hub.list().await.unwrap();
    e2e.sync().await.unwrap();
    let claimed = device
        .hub
        .claim_turn(TURN, &format!("device:{DEVICE}"))
        .await
        .unwrap();
    let hub: Arc<dyn TurnHub> = Arc::new(device.hub.clone());
    let recovery = HubRecovery::new(hub, &claimed.turn, &format!("device:{DEVICE}"), 31_000)
        .with_encryption(e2e, CONVERSATION.into());
    let checkpoint = BotRunCheckpoint {
        revision: 1,
        started_at_ms: 1_000,
        deadline_at_ms: 31_000,
        state: json!({ "version": 1, "history": "launch code 4711" }),
    };
    recovery.save(&checkpoint).await.unwrap();

    let saved = device.api.calls("/checkpoint").pop().unwrap().unwrap();
    assert_eq!(saved["checkpoint"]["state"]["sealed_checkpoint"], 1);
    assert!(!saved.to_string().contains("4711"), "{saved}");
    assert_eq!(recovery.load().await.unwrap(), Some(checkpoint));
}

#[tokio::test]
async fn member_reads_need_the_turn_the_device_holds_in_that_conversation() {
    let device = device();
    let keys = DeviceKeyHub::new(device.hub.clone());
    let asked = vec![
        ("principals", format!("owner,{SEALING}")),
        ("as", SEALING.to_owned()),
        ("conversation_id", CONVERSATION.to_owned()),
    ];
    let failure = keys.read("inbox/keys", asked).await.unwrap_err();
    assert!(failure.message.contains("holds no turn"), "{failure}");
    for path in [
        "inbox/conversations/conv-1/entries",
        "bots/bot-sealing/secrets",
    ] {
        let failure = keys
            .read(path, vec![("as", SEALING.to_owned())])
            .await
            .unwrap_err();
        assert!(failure.message.contains("key host"), "{path}: {failure}");
    }
    assert!(keys.put("bots/x", Vec::new(), json!({})).await.is_err());
    assert!(device.api.requests().is_empty());
}
