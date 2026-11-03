use super::{
    account::{AccountConfig, AccountHost},
    e2e::{
        CODE_E2E_MEMBER_KEY, CODE_E2E_UNAVAILABLE, CODE_E2E_UNREADABLE, E2eHost, is_sealed,
    },
    host::TurnEvents,
    hub::{HubFailure, TurnHub},
    tests::{FakeRunner, Recorded, settle, until},
};
use async_trait::async_trait;
use chrono::Utc;
use flow_like_device_crypto::inbox::{
    InboxKeyVault, InboxKeyring, PublicInboxKey, Recipient, Scope, seal,
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_turn::{BotTurn, TurnHostKind, TurnStatus},
    inbox::InboxEntryKind,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

const BOT: &str = "bot1";
const CONV: &str = "conv";
const ALICE: &str = "alice";
const BOB: &str = "bob";

#[derive(Default)]
struct MemVault(Mutex<HashMap<String, Vec<u8>>>);

impl InboxKeyVault for MemVault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        Ok(self.0.lock().unwrap().get(owner).cloned().map(Zeroizing::new))
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap().insert(owner.into(), secret.to_vec());
        Ok(())
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(owner);
        Ok(())
    }
}

#[derive(Default)]
struct Hub {
    turns: Mutex<Vec<BotTurn>>,
    entries: Mutex<Vec<InboxEntry>>,
    bots: Mutex<Vec<Value>>,
    members: Mutex<Vec<String>>,
    keys: Mutex<Vec<Value>>,
    sent: Mutex<Vec<Value>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<String>>,
    puts: Mutex<Vec<Value>>,
    deletes: Mutex<Vec<(String, Vec<(&'static str, String)>)>>,
}

#[async_trait]
impl TurnHub for Hub {
    async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
        Ok(std::mem::take(&mut *self.turns.lock().unwrap()))
    }

    async fn claim(&self, turn_id: &str, _: &str) -> Result<BotTurn, HubFailure> {
        Ok(turn(turn_id))
    }

    async fn heartbeat(&self, turn_id: &str, _: &str) -> Result<BotTurn, HubFailure> {
        Ok(turn(turn_id))
    }

    async fn entries(&self, _: &str, _: &str) -> Result<Vec<InboxEntry>, HubFailure> {
        Ok(self.entries.lock().unwrap().clone())
    }

    async fn send(
        &self,
        _: &str,
        _: &str,
        message: Value,
        _: &str,
        _: &str,
    ) -> Result<(), HubFailure> {
        self.sent.lock().unwrap().push(message);
        Ok(())
    }

    async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn report_host(&self, _: &str, _: &[String]) -> Result<Vec<String>, HubFailure> {
        Ok(Vec::new())
    }

    async fn delete_host(&self, _: &str) -> Result<(), HubFailure> {
        Ok(())
    }

    async fn cancel_turn(&self, _: &str, _: &str) -> Result<(), HubFailure> {
        Ok(())
    }

    async fn read(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.reads.lock().unwrap().push(path.to_owned());
        match path {
            "bots" => Ok(json!({ "bots": *self.bots.lock().unwrap() })),
            "inbox/keys" => {
                let asked = &query[0].1;
                let keys: Vec<Value> = self
                    .keys
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|key| asked.split(',').any(|id| key["principal_id"] == id))
                    .cloned()
                    .collect();
                Ok(json!({ "keys": keys }))
            }
            p if p == format!("inbox/conversations/{CONV}") => {
                let members: Vec<Value> = self
                    .members
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|id| json!({ "principal_id": id }))
                    .collect();
                Ok(json!({ "encryption": "e2e", "members": members }))
            }
            _ => Err(HubFailure::answered(404, None, "unexpected read")),
        }
    }

    async fn write(
        &self,
        path: &str,
        _: Vec<(&'static str, String)>,
        _: Value,
    ) -> Result<Value, HubFailure> {
        self.writes.lock().unwrap().push(path.to_owned());
        Ok(Value::Null)
    }

    async fn put(
        &self,
        _: &str,
        _: Vec<(&'static str, String)>,
        body: Value,
    ) -> Result<Value, HubFailure> {
        self.puts.lock().unwrap().push(body);
        Ok(Value::Null)
    }

    async fn delete(
        &self,
        path: &str,
        query: Vec<(&'static str, String)>,
    ) -> Result<Value, HubFailure> {
        self.deletes.lock().unwrap().push((path.to_owned(), query));
        Ok(Value::Null)
    }
}

fn turn(id: &str) -> BotTurn {
    BotTurn {
        id: id.into(),
        bot_id: BOT.into(),
        conversation_id: CONV.into(),
        trigger_message_id: "m1".into(),
        host_kind: TurnHostKind::Desktop,
        status: TurnStatus::Queued,
        attempts: 0,
        host_id: None,
        lease_until: None,
        reply_message_id: None,
        error_code: None,
        hops: 0,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        batched: Vec::new(),
    }
}

fn bot_json(capabilities: &[&str]) -> Value {
    json!({ "id": BOT, "location": "desktop", "capabilities": capabilities })
}

fn directory_key(principal: &str, key: &PublicInboxKey) -> Value {
    json!({
        "principal_id": principal,
        "key_id": key.key_id,
        "algorithm": key.algorithm,
        "public_key": key.public_key,
    })
}

fn recipient(principal: &str, key: &PublicInboxKey) -> Recipient {
    Recipient {
        principal_id: principal.into(),
        key_id: key.key_id.clone(),
        public_key: key.public_key.clone(),
    }
}

fn person_ring() -> InboxKeyring {
    InboxKeyring::new(Arc::new(MemVault::default()))
}

fn scope<'a>(message_id: &'a str, sender: &'a str) -> Scope<'a> {
    Scope {
        conversation_id: CONV,
        message_id,
        sender_id: sender,
    }
}

fn config() -> AccountConfig {
    AccountConfig {
        host_id: "desktop:test".into(),
        poll: Duration::from_millis(20),
        max_backoff: Duration::from_millis(80),
        heartbeat: Duration::from_millis(15),
        deadline: Duration::from_secs(5),
        send_retries: 1,
        send_retry_delay: Duration::from_millis(5),
        presence: Duration::from_millis(40),
    }
}

struct Fixture {
    hub: Arc<Hub>,
    runner: Arc<FakeRunner>,
    host: AccountHost,
    e2e: Arc<E2eHost>,
    alice: InboxKeyring,
    bob: InboxKeyring,
    alice_key: PublicInboxKey,
    bob_key: PublicInboxKey,
}

impl Fixture {
    async fn start(capabilities: &[&str], bob_has_key: bool) -> Self {
        let hub = Arc::new(Hub::default());
        *hub.bots.lock().unwrap() = vec![bot_json(capabilities)];
        let e2e = Arc::new(E2eHost::new(
            hub.clone(),
            Arc::new(MemVault::default()),
            "desktop:test".into(),
        ));
        e2e.sync().await.unwrap();
        let (alice, bob) = (person_ring(), person_ring());
        let alice_key = alice.ensure(ALICE).unwrap().0;
        let bob_key = bob.ensure(BOB).unwrap().0;
        let mut keys = vec![directory_key(ALICE, &alice_key)];
        if bob_has_key {
            keys.push(directory_key(BOB, &bob_key));
        }
        if let Some(put) = hub.puts.lock().unwrap().first() {
            keys.push(put.clone());
        }
        *hub.keys.lock().unwrap() = keys;
        *hub.members.lock().unwrap() = vec![ALICE.into(), BOB.into(), BOT.into()];
        let runner = FakeRunner::new(true);
        let host = AccountHost::new(
            hub.clone(),
            runner.clone(),
            Arc::new(Recorded::default()) as Arc<dyn TurnEvents>,
            Arc::new(Semaphore::new(2)),
            config(),
        );
        host.enable_e2e(e2e.clone());
        Self {
            hub,
            runner,
            host,
            e2e,
            alice,
            bob,
            alice_key,
            bob_key,
        }
    }

    fn bot_key(&self) -> PublicInboxKey {
        self.e2e.status(BOT).key_id.map_or_else(
            || panic!("no key"),
            |key_id| {
                let put = self.hub.puts.lock().unwrap();
                let found = put.iter().find(|put| put["key_id"] == key_id.as_str()).unwrap();
                PublicInboxKey {
                    key_id,
                    algorithm: found["algorithm"].as_str().unwrap().into(),
                    public_key: found["public_key"].as_str().unwrap().into(),
                    fingerprint: String::new(),
                }
            },
        )
    }

    fn sealed_entry(&self, seq: i64, message_id: &str, text: &str) -> InboxEntry {
        let plaintext = serde_json::to_vec(&json!({ "text": text })).unwrap();
        let envelope = seal(
            &scope(message_id, ALICE),
            &recipient(BOT, &self.bot_key()),
            &plaintext,
        )
        .unwrap();
        InboxEntry {
            id: format!("e{seq}"),
            message_id: message_id.into(),
            conversation_id: CONV.into(),
            sender_id: ALICE.into(),
            kind: InboxEntryKind::Message,
            envelope,
            created_at: Utc::now().fixed_offset(),
            seq: Some(seq),
            cursor: String::new(),
        }
    }

    async fn run_turn(&self, entries: Vec<InboxEntry>) -> Option<(TurnStatus, Option<String>)> {
        *self.hub.entries.lock().unwrap() = entries;
        self.hub.turns.lock().unwrap().push(turn("bt_1"));
        tokio::spawn(self.host.clone().run());
        until("the turn ends", || {
            self.host.turns().iter().any(|t| {
                matches!(t.status, TurnStatus::Done | TurnStatus::Failed)
            })
        })
        .await;
        settle().await;
        self.host
            .turns()
            .first()
            .map(|t| (t.status, t.error_code.clone()))
    }
}

#[tokio::test]
async fn the_bot_opens_the_sealed_message_and_seals_its_reply_to_every_member() {
    let f = Fixture::start(&["encryption"], true).await;
    let entry = f.sealed_entry(1, "m1", "hello secret");
    let (status, _) = f.run_turn(vec![entry]).await.unwrap();
    assert_eq!(status, TurnStatus::Done);

    assert_eq!(f.runner.jobs.lock().unwrap()[0].prompt, "hello secret");
    let sent = f.hub.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].get("envelope").is_none());
    let envelopes = sent[0]["envelopes"].as_object().unwrap();
    let mut members: Vec<_> = envelopes.keys().map(String::as_str).collect();
    members.sort_unstable();
    assert_eq!(members, [ALICE, BOB, BOT]);

    let message_id = sent[0]["message_id"].as_str().unwrap();
    let open = |ring: &InboxKeyring, who: &str| -> Value {
        let plaintext = ring
            .open(who, &scope(message_id, BOT), &envelopes[who])
            .unwrap();
        serde_json::from_slice(&plaintext).unwrap()
    };
    assert_eq!(open(&f.alice, ALICE)["text"], "answer to: hello secret");
    assert_eq!(open(&f.bob, BOB)["text"], "answer to: hello secret");
    assert!(f.alice.open(ALICE, &scope(message_id, BOT), &envelopes[BOB]).is_err());
    assert_eq!(f.alice_key.algorithm, f.bob_key.algorithm);
}

#[tokio::test]
async fn the_server_never_receives_plaintext_or_memory_of_an_encrypted_turn() {
    let f = Fixture::start(&["encryption"], true).await;
    let entry = f.sealed_entry(1, "m1", "launch code 4711");
    f.run_turn(vec![entry]).await.unwrap();

    let everything = format!(
        "{:?}{:?}{:?}{:?}",
        f.hub.sent.lock().unwrap(),
        f.hub.writes.lock().unwrap(),
        f.hub.puts.lock().unwrap(),
        f.hub.reads.lock().unwrap(),
    );
    assert!(!everything.contains("4711"), "{everything}");
    assert!(!everything.contains("answer to"), "{everything}");
    assert!(f.hub.writes.lock().unwrap().is_empty(), "no summaries written");
    assert!(
        f.hub.reads.lock().unwrap().iter().all(|p| !p.contains("segments")),
        "no hub memory read"
    );
    assert!(f.runner.jobs.lock().unwrap()[0].memory.is_none());
}

#[tokio::test]
async fn a_member_without_a_key_stops_the_turn_and_nothing_is_posted() {
    let f = Fixture::start(&["encryption"], false).await;
    let entry = f.sealed_entry(1, "m1", "hi");
    let (status, code) = f.run_turn(vec![entry]).await.unwrap();
    assert_eq!(status, TurnStatus::Failed);
    assert_eq!(code.as_deref(), Some(CODE_E2E_MEMBER_KEY));
    assert!(f.hub.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_bot_without_the_capability_never_opens_or_answers() {
    let f = Fixture::start(&[], true).await;
    let entry = f.sealed_entry(1, "m1", "hi");
    let (status, code) = f.run_turn(vec![entry]).await.unwrap();
    assert_eq!(status, TurnStatus::Failed);
    assert_eq!(code.as_deref(), Some(CODE_E2E_UNAVAILABLE));
    assert!(f.hub.sent.lock().unwrap().is_empty());
    assert_eq!(f.runner.started(), 0);
}

#[tokio::test]
async fn a_trigger_sealed_to_another_key_ends_the_turn_unread() {
    let f = Fixture::start(&["encryption"], true).await;
    let mut entry = f.sealed_entry(1, "m1", "hi");
    entry.envelope = seal(
        &scope("m1", ALICE),
        &recipient(BOT, &f.alice_key),
        b"{\"text\":\"hi\"}",
    )
    .unwrap();
    let (status, code) = f.run_turn(vec![entry]).await.unwrap();
    assert_eq!(status, TurnStatus::Failed);
    assert_eq!(code.as_deref(), Some(CODE_E2E_UNREADABLE));
    assert!(f.hub.sent.lock().unwrap().is_empty());
    assert!(is_sealed(&f.hub.entries.lock().unwrap()[0]));
}

#[tokio::test]
async fn sync_publishes_the_public_key_once_as_the_bot_and_rotation_revokes_the_old_one() {
    let f = Fixture::start(&["encryption"], true).await;
    f.e2e.sync().await.unwrap();
    let first = {
        let puts = f.hub.puts.lock().unwrap();
        assert_eq!(puts.len(), 1, "published once");
        assert_eq!(puts[0]["principal_id"], BOT);
        assert_eq!(puts[0]["algorithm"], "hpke-x25519-sha256-chacha20poly1305");
        puts[0]["key_id"].as_str().unwrap().to_owned()
    };
    let status = f.e2e.rotate(BOT).await.unwrap();
    assert!(status.published && status.enabled);
    assert_ne!(status.key_id.as_deref(), Some(first.as_str()));
    assert_eq!(status.host_id, "desktop:test");
    assert_eq!(f.hub.puts.lock().unwrap().len(), 2);
    let deletes = f.hub.deletes.lock().unwrap();
    assert_eq!(deletes[0].0, format!("inbox/keys/{first}"));
    assert_eq!(deletes[0].1, [("principal_id", BOT.to_owned())]);
}

#[tokio::test]
async fn rotation_is_refused_for_a_bot_without_the_capability() {
    let f = Fixture::start(&[], true).await;
    assert!(f.e2e.rotate(BOT).await.is_err());
    assert!(f.hub.puts.lock().unwrap().is_empty());
    assert!(!f.e2e.status(BOT).enabled);
}
