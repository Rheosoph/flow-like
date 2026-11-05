//! The move into an account, against the real store and a hub in memory that mints ids and
//! keys imports by `local_id` like the API. The spec was the desktop's TypeScript move
//! (`apps/desktop/lib/__tests__/local-bot-move.test.ts`).

use super::*;
use crate::{
    BotMover, HubBody, HubCall, HubError, HubMethod, LOCAL_OWNER_ID, MoveError, MoveIssue,
    MoveProgress, MoveReport, MoveStep, MovedBot, MovedConversation, WorkspaceSummary,
    WriteSegmentRequest, codes,
    transfer::{
        AccountHub, BOT_SECRET_UNREADABLE, HUB_UNAVAILABLE, WORKSPACE_NOT_VERIFIED,
        requests::{
            AccountIds, ImportConversationRequest, PositionMap, import_batch_size,
            import_conversation_requests, segment_requests, unmovable_segments,
        },
    },
};
use flow_like_types::{bot::BotSystemKind, bot_message::MessageBody};
use serde_json::Map;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::MutexGuard,
};

const SUB: &str = "user-1";

type Files = Arc<Mutex<HashMap<String, BTreeMap<String, u64>>>>;

struct AccountBot {
    id: String,
    settings: Map<String, Value>,
    apps: Vec<String>,
}

/// The account's side: bots and chats under the ids it minted, found again by `local_id`.
#[derive(Default)]
struct Account {
    bots: Vec<AccountBot>,
    bot_keys: HashMap<String, String>,
    chats: HashMap<String, Vec<Value>>,
    chat_keys: HashMap<String, String>,
    /// Direct chats the account already holds, by the local id whose history merges into them.
    existing_direct: HashMap<String, String>,
    issues: Vec<Value>,
    fail_next_import_of: Option<String>,
    fail_chat_delete_once: bool,
    deleted_chats: Vec<String>,
    minted_bots: usize,
    minted_chats: usize,
    log: Vec<String>,
    checks: Vec<Value>,
    patches: Vec<Value>,
    imports: Vec<Value>,
    /// `(account bot, name, value)` of every secret put.
    secrets: Vec<(String, String, String)>,
    /// The account's positions per conversation, parallel to `chats`. A new conversation's
    /// first position went to a join and every call leaves one for an event of the server.
    seqs: HashMap<String, Vec<i64>>,
    next_seq: HashMap<String, i64>,
    /// Entries one read returns at most; 0 is the API's 100.
    page_cap: usize,
    /// `(conversation, bot, body)` of every segment written, once per id.
    segments: Vec<(String, String, Value)>,
    fail_segments_once: bool,
    /// Files of the local workspace apps and, by upload path, of the account's copies.
    local_files: Files,
    remote_files: HashMap<String, BTreeMap<String, u64>>,
    uploads: usize,
    /// From this upload on, one file is lost on the way.
    lose_a_file_from_upload: Option<usize>,
    fail_delete: bool,
    deleted_workspaces: Vec<String>,
    upload_requests: Vec<Value>,
    /// Every connector created on the account, in order.
    connectors: Vec<Value>,
}

fn not_found(what: String) -> HubError {
    HubError::answered(404, Some("NOT_FOUND".into()), what)
}

impl Account {
    fn view(&self, id: &str) -> Value {
        let mut view = self
            .bots
            .iter()
            .find(|bot| bot.id == id)
            .map(|bot| bot.settings.clone())
            .unwrap_or_default();
        view.insert("id".into(), json!(id));
        view.insert("owner_id".into(), json!(SUB));
        Value::Object(view)
    }

    fn bot(&mut self, id: &str) -> Result<&mut AccountBot, HubError> {
        self.bots
            .iter_mut()
            .find(|bot| bot.id == id)
            .ok_or_else(|| not_found(format!("Bot '{id}' not found")))
    }

    fn handle(&mut self, call: HubCall<'_>) -> Result<Option<Value>, HubError> {
        let body = match call.body {
            HubBody::Json(body) => body.clone(),
            _ => Value::Null,
        };
        self.log.push(match call.path {
            ["inbox", "import"] => format!("{call} {}", body["local_id"].as_str().unwrap()),
            _ => call.to_string(),
        });
        match (call.method, call.path) {
            (HubMethod::Get, ["bots"]) => {
                let bots: Vec<Value> = self.bots.iter().map(|bot| self.view(&bot.id)).collect();
                Ok(Some(json!({ "bots": bots })))
            }
            (HubMethod::Post, ["bots", "import", "check"]) => {
                self.checks.push(body);
                Ok(Some(json!({ "issues": self.issues })))
            }
            (HubMethod::Post, ["bots", "import"]) => Ok(Some(self.import_bot(&body))),
            (HubMethod::Patch, ["bots", id]) => {
                let bot = self.bot(id)?;
                for (field, value) in body.as_object().cloned().unwrap_or_default() {
                    bot.settings.insert(field, value);
                }
                self.patches.push(body);
                Ok(Some(self.view(id)))
            }
            (HubMethod::Get, ["bots", id, "apps"]) => {
                let apps: Vec<Value> = self
                    .bot(id)?
                    .apps
                    .iter()
                    .map(|app_id| json!({ "app_id": app_id }))
                    .collect();
                Ok(Some(json!({ "apps": apps })))
            }
            (HubMethod::Put, ["bots", id, "apps", app_id]) => {
                let bot = self.bot(id)?;
                if !bot.apps.iter().any(|known| known == app_id) {
                    bot.apps.push(app_id.to_string());
                }
                Ok(None)
            }
            (HubMethod::Delete, ["bots", id, "apps", app_id]) => {
                self.bot(id)?.apps.retain(|known| known != app_id);
                Ok(None)
            }
            (HubMethod::Put, ["bots", id, "secrets", name]) => {
                let HubBody::Secret(secret) = call.body else {
                    panic!("a secret travels as a secret body");
                };
                self.bot(id)?;
                self.secrets
                    .push((id.to_string(), name.to_string(), secret.value.to_string()));
                Ok(Some(json!({ "name": name })))
            }
            (HubMethod::Get, ["bots", id, "connectors"]) => {
                self.bot(id)?;
                Ok(Some(json!({ "connectors": self.connectors })))
            }
            (HubMethod::Post, ["bots", id, "connectors"]) => {
                self.bot(id)?;
                self.connectors.push(body.clone());
                Ok(Some(body))
            }
            (HubMethod::Post, ["inbox", "import"]) => self.import_chat(body),
            (HubMethod::Delete, ["inbox", "conversations", chat]) => {
                if std::mem::take(&mut self.fail_chat_delete_once) {
                    return Err(HubError::failed(
                        "OFFLINE",
                        "Deletion did not reach the account",
                    ));
                }
                if !self.deleted_chats.iter().any(|id| id == chat) {
                    self.deleted_chats.push(chat.to_string());
                }
                Ok(None)
            }
            (HubMethod::Post, ["bots", id, "workspace"]) => {
                self.bot(id)?;
                Ok(Some(json!({ "app_id": format!("ws_{id}") })))
            }
            (HubMethod::Post, ["bots", id, "workspace", "upload"]) => {
                self.bot(id)?;
                self.upload_requests.push(body);
                Ok(Some(json!({
                    "upload_path": format!("apps/ws_{id}"),
                    "shared_credentials": {},
                    "expiration": null,
                })))
            }
            (HubMethod::Get, ["inbox", "conversations", chat, "entries"]) => {
                Ok(Some(self.read_entries(chat, call.query)))
            }
            (HubMethod::Post, ["inbox", "conversations", chat, "segments"]) => {
                self.write_segment(chat, call.query, body)
            }
            _ => Err(not_found(format!("No route for {call}"))),
        }
    }

    fn import_bot(&mut self, body: &Value) -> Value {
        let local_id = body["local_id"].as_str().unwrap().to_string();
        let id = match self.bot_keys.get(&local_id) {
            Some(id) => id.clone(),
            None => {
                self.minted_bots += 1;
                let id = format!("bot_acct{}", self.minted_bots);
                let mut settings = body.as_object().cloned().unwrap_or_default();
                settings.remove("local_id");
                let apps = settings
                    .remove("app_ids")
                    .and_then(|apps| serde_json::from_value(apps).ok())
                    .unwrap_or_default();
                self.bots.push(AccountBot {
                    id: id.clone(),
                    settings,
                    apps,
                });
                self.bot_keys.insert(local_id, id.clone());
                id
            }
        };
        self.view(&id)
    }

    fn read_entries(&self, chat: &str, query: &[(&str, &str)]) -> Value {
        let number = |name: &str| {
            query
                .iter()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value.parse::<usize>().ok())
        };
        let from = number("from_seq").unwrap_or(1) as i64;
        let limit = number("limit").unwrap_or(100).min(if self.page_cap == 0 {
            100
        } else {
            self.page_cap
        });
        let rows: Vec<Value> = self.chats[chat]
            .iter()
            .zip(&self.seqs[chat])
            .filter(|(_, seq)| **seq >= from)
            .map(|(entry, seq)| {
                json!({
                    "message_id": entry["message_id"],
                    "sender_id": entry["sender_id"],
                    "kind": entry["kind"],
                    "seq": seq,
                })
            })
            .take(limit + 1)
            .collect();
        let next = rows.get(limit).map(|row| row["seq"].clone());
        json!({ "entries": &rows[..rows.len().min(limit)], "next_from_seq": next })
    }

    fn write_segment(
        &mut self,
        chat: &str,
        query: &[(&str, &str)],
        body: Value,
    ) -> Result<Option<Value>, HubError> {
        if std::mem::take(&mut self.fail_segments_once) {
            return Err(HubError::unreachable("Network unavailable: POST segments"));
        }
        let bot = query
            .iter()
            .find(|(key, _)| *key == "as")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| not_found("as is missing".into()))?;
        self.bot(&bot)?;
        let seqs = self
            .seqs
            .get(chat)
            .ok_or_else(|| not_found(format!("Conversation '{chat}' not found")))?;
        if let Some((_, _, written)) = self.segments.iter().find(|(held_in, author, written)| {
            held_in == chat && *author == bot && written["id"] == body["id"]
        }) {
            return Ok(Some(written.clone()));
        }
        let (from, to) = (
            body["seq_from"].as_i64().unwrap(),
            body["seq_to"].as_i64().unwrap(),
        );
        let holds_an_entry = self.chats[chat]
            .iter()
            .zip(seqs)
            .any(|(entry, seq)| entry["kind"] != "member" && (from..=to).contains(seq));
        let past_the_end = seqs.last().is_none_or(|last| to > *last);
        if past_the_end || !holds_an_entry {
            return Err(HubError::answered(
                422,
                Some("INBOX_SEGMENT_RANGE".into()),
                format!("Bot '{bot}' holds no entries at #{from}-#{to}"),
            ));
        }
        if let Some(parent) = body["parent_id"].as_str() {
            let covers = self.segments.iter().any(|(held_in, author, written)| {
                held_in == chat
                    && *author == bot
                    && written["id"] == parent
                    && written["level"].as_i64() > body["level"].as_i64()
                    && written["seq_from"].as_i64() <= Some(from)
                    && written["seq_to"].as_i64() >= Some(to)
            });
            if !covers {
                return Err(HubError::answered(
                    422,
                    Some("INBOX_SEGMENT_PARENT".into()),
                    format!("No parent '{parent}' covers #{from}-#{to}"),
                ));
            }
        }
        self.segments.push((chat.to_string(), bot, body.clone()));
        Ok(Some(body))
    }

    fn import_chat(&mut self, body: Value) -> Result<Option<Value>, HubError> {
        let local_id = body["local_id"].as_str().unwrap().to_string();
        if self.fail_next_import_of.as_deref() == Some(local_id.as_str()) {
            self.fail_next_import_of = None;
            return Err(HubError::unreachable(
                "Network unavailable: POST inbox/import",
            ));
        }
        let known = self
            .chat_keys
            .get(&local_id)
            .or_else(|| self.existing_direct.get(&local_id))
            .cloned();
        let id = known.unwrap_or_else(|| {
            self.minted_chats += 1;
            format!("chat_acct{}", self.minted_chats)
        });
        self.chat_keys.insert(local_id, id.clone());
        let chat = self.chats.entry(id.clone()).or_default();
        let seqs = self.seqs.entry(id.clone()).or_default();
        let next = self.next_seq.entry(id.clone()).or_insert(5);
        let mut imported = 0;
        for entry in body["entries"].as_array().into_iter().flatten() {
            let repeated = chat.iter().any(|known| {
                known["sender_id"] == entry["sender_id"]
                    && known["message_id"] == entry["message_id"]
            });
            if !repeated {
                chat.push(entry.clone());
                seqs.push(*next);
                *next += 1;
                imported += 1;
            }
        }
        *next += 1;
        self.imports.push(body);
        Ok(Some(
            json!({ "conversation": { "id": id }, "imported": imported }),
        ))
    }
}

struct FakeHub(Mutex<Account>);

impl FakeHub {
    /// An account that holds `bot_b`, a bot that moved before.
    fn new() -> Self {
        let hub = Self(Mutex::new(Account::default()));
        hub.add_bot("bot_b");
        hub
    }

    fn account(&self) -> MutexGuard<'_, Account> {
        self.0.lock().unwrap()
    }

    fn add_bot(&self, id: &str) {
        self.account().bots.push(AccountBot {
            id: id.into(),
            settings: Map::new(),
            apps: Vec::new(),
        });
    }

    fn local_files(&self) -> Files {
        self.account().local_files.clone()
    }

    fn log(&self) -> Vec<String> {
        self.account().log.clone()
    }

    fn clear_log(&self) {
        self.account().log.clear();
    }

    /// The `inbox/import` bodies for the conversation with `local_id`.
    fn imports(&self, local_id: &str) -> Vec<Value> {
        self.account()
            .imports
            .iter()
            .filter(|body| body["local_id"] == local_id)
            .cloned()
            .collect()
    }

    /// The message ids of each `inbox/import` call for `local_id`.
    fn imported_ids(&self, local_id: &str) -> Vec<Vec<String>> {
        self.imports(local_id)
            .iter()
            .map(|body| message_ids(&body["entries"]))
            .collect()
    }

    /// The message ids the account's conversation for `local_id` holds.
    fn chat(&self, local_id: &str) -> Vec<String> {
        let account = self.account();
        let id = &account.chat_keys[local_id];
        message_ids(&Value::Array(account.chats[id].clone()))
    }

    /// The settings and apps of the account bot imported from `local_id`.
    fn bot_of(&self, local_id: &str) -> (Map<String, Value>, Vec<String>) {
        let id = self.account().bot_keys[local_id].clone();
        self.bot(&id)
    }

    /// The settings and apps of account bot `id`.
    fn bot(&self, id: &str) -> (Map<String, Value>, Vec<String>) {
        let account = self.account();
        let bot = account.bots.iter().find(|bot| bot.id == id).unwrap();
        (bot.settings.clone(), bot.apps.clone())
    }

    /// The account's own Assistant or Flux, edited on the web, with an app of its own.
    fn add_built_in(&self, id: &str, system: &str, location: &str) {
        let settings = json!({
            "system": system,
            "name": null,
            "instructions": "From the web",
            "location": location,
        });
        self.account().bots.push(AccountBot {
            id: id.into(),
            settings: settings.as_object().cloned().unwrap(),
            apps: vec!["app-web".into()],
        });
    }
}

impl AccountHub for FakeHub {
    fn caller(&self) -> &str {
        SUB
    }

    async fn call(&self, call: HubCall<'_>) -> Result<Option<Value>, HubError> {
        self.account().handle(call)
    }

    async fn summarize_workspace(&self, app_id: &str) -> Result<WorkspaceSummary, HubError> {
        let account = self.account();
        let local = account.local_files.lock().unwrap();
        let files = local.get(app_id);
        Ok(WorkspaceSummary {
            total_size_bytes: files.map_or(0, |files| files.values().sum()),
            total_object_count: files.map_or(0, |files| files.len() as u64),
        })
    }

    async fn upload_workspace(&self, app_id: &str, grant: &Value) -> Result<(), HubError> {
        let mut account = self.account();
        account.log.push(format!("UPLOAD {app_id}"));
        account.uploads += 1;
        let mut files = account
            .local_files
            .lock()
            .unwrap()
            .get(app_id)
            .cloned()
            .unwrap_or_default();
        if account
            .lose_a_file_from_upload
            .is_some_and(|from| account.uploads >= from)
        {
            files.pop_last();
        }
        let path = grant["upload_path"].as_str().unwrap().to_string();
        account.remote_files.insert(path, files);
        Ok(())
    }

    async fn workspace_differences(
        &self,
        app_id: &str,
        grant: &Value,
    ) -> Result<Vec<String>, HubError> {
        let mut account = self.account();
        account.log.push(format!("COMPARE {app_id}"));
        let there = account
            .remote_files
            .get(grant["upload_path"].as_str().unwrap())
            .cloned()
            .unwrap_or_default();
        let here = account
            .local_files
            .lock()
            .unwrap()
            .get(app_id)
            .cloned()
            .unwrap_or_default();
        Ok(here
            .into_iter()
            .filter(|(path, bytes)| there.get(path) != Some(bytes))
            .map(|(path, _)| path)
            .collect())
    }

    async fn delete_workspace(&self, app_id: &str) -> Result<(), HubError> {
        let mut account = self.account();
        account.log.push(format!("DELETE {app_id}"));
        if account.fail_delete {
            return Err(HubError::failed("STORAGE", "the disk is busy"));
        }
        account.local_files.lock().unwrap().remove(app_id);
        account.deleted_workspaces.push(app_id.to_string());
        Ok(())
    }
}

fn message_ids(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| entry["message_id"].as_str().unwrap().to_string())
        .collect()
}

type Hook = Box<dyn FnMut() + Send>;

/// The steps a move told, and writes that land right before a step, as from another window.
#[derive(Default)]
struct Steps {
    seen: Mutex<Vec<MoveProgress>>,
    hooks: Mutex<Vec<(MoveStep, usize, Hook)>>,
}

impl Steps {
    fn before(&self, step: MoveStep, times: usize, hook: impl FnMut() + Send + 'static) {
        self.hooks
            .lock()
            .unwrap()
            .push((step, times, Box::new(hook)));
    }

    fn record(&self, progress: MoveProgress) {
        let step = progress.step;
        self.seen.lock().unwrap().push(progress);
        for (when, left, hook) in self.hooks.lock().unwrap().iter_mut() {
            if *when == step && *left > 0 {
                *left -= 1;
                hook();
            }
        }
    }

    fn of(&self, bot_id: &str) -> Vec<(MoveStep, u8)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|progress| progress.bot_id == bot_id)
            .map(|progress| (progress.step, progress.pass))
            .collect()
    }

    fn count(&self, step: MoveStep) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|progress| progress.step == step)
            .count()
    }
}

/// A clean move after its review: import, sync, chats, forgets, rebind, the bot last.
const CLEAN_STEPS: [(MoveStep, u8); 11] = [
    (MoveStep::Check, 0),
    (MoveStep::ImportBot, 0),
    (MoveStep::UpdateBot, 0),
    (MoveStep::Apps, 0),
    (MoveStep::Secrets, 0),
    (MoveStep::Conversations, 1),
    (MoveStep::ForgetConversations, 1),
    (MoveStep::Verify, 1),
    (MoveStep::Rebind, 1),
    (MoveStep::ForgetBot, 1),
    (MoveStep::Done, 1),
];

fn group(f: &Fixture, title: &str, bots: &[&Bot]) -> String {
    let members: Vec<&str> = bots.iter().map(|bot| bot.id.as_str()).collect();
    f.store
        .create_conversation(conversation(
            json!({ "kind": "group", "title": title, "members": members }),
        ))
        .unwrap()
        .id
}

fn say(
    f: &Fixture,
    conversation_id: &str,
    sender: &str,
    message_id: &str,
    kind: &str,
    body: Value,
) {
    f.send(conversation_id, sender, message_id, kind, body)
        .unwrap();
}

/// What the person sends from another window while a move runs.
fn send_text(store: &LocalBotStore, conversation_id: &str, message_id: &str) {
    store
        .send(
            conversation_id,
            LOCAL_OWNER_ID,
            entry_request(json!({
                "message_id": message_id,
                "kind": "message",
                "envelope": plain_envelope(json!({ "text": message_id })),
            })),
        )
        .unwrap();
}

/// A bot that moved to the account before: its chats name its account id, it is gone here.
fn moved_earlier(f: &Fixture, bot: &Bot, account_id: &str) {
    f.store.rebind_bot(&bot.id, account_id).unwrap();
    f.store.forget_bot(&bot.id, None).unwrap();
}

/// `count` messages from `sender`, written into the file in one go, a millisecond apart.
fn bulk_messages(
    f: &Fixture,
    conversation_id: &str,
    sender: &str,
    prefix: &str,
    count: usize,
    body: &Value,
) {
    let envelope = plain_envelope(body.clone()).to_string();
    let start = chrono::Utc::now().timestamp_millis() * 1_000;
    let mut db = f.raw();
    let tx = db.transaction().unwrap();
    for n in 0..count {
        tx.execute(
            "INSERT INTO entries (id, conversation_id, message_id, sender_id, kind, envelope, created_at)
             VALUES (?1, ?2, ?3, ?4, 'message', ?5, ?6)",
            rusqlite::params![
                format!("{prefix}e{n}"),
                conversation_id,
                format!("{prefix}{n}"),
                sender,
                envelope,
                start + n as i64 * 1_000
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

/// Gives secret `name` the sealed value of `from`, which never opens under another name.
fn break_secret(path: &Path, bot_id: &str, name: &str, from: &str) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE bot_secrets SET ciphertext = (
                 SELECT ciphertext FROM bot_secrets WHERE bot_id = ?1 AND name = ?3)
             WHERE bot_id = ?1 AND name = ?2",
            [bot_id, name, from],
        )
        .unwrap();
}

fn unreadable(name: &str) -> MoveIssue {
    MoveIssue {
        part: "secret".into(),
        code: BOT_SECRET_UNREADABLE.into(),
        message: format!("Secret {name} can't be read on this computer anymore"),
        ref_id: Some(name.into()),
    }
}

/// Ada with apps, secrets and three chats: a direct one, a group with Bee (moved before) and a
/// group with Cy, who stays here.
struct World {
    f: Fixture,
    hub: FakeHub,
    mover: BotMover,
    steps: Steps,
    ada: Bot,
    cy: Bot,
    direct: String,
    team: String,
    waiting: String,
}

fn world() -> World {
    let f = fixture();
    let ada = f
        .store
        .create_bot(create(json!({
            "name": "Ada",
            "preset_id": "preset-helper",
            "description": "Ada helps",
            "instructions": "Be brief.",
            "appearance": { "kind": "pip" },
            "model": { "primary": "bit-1", "fallbacks": ["bit-2"] },
            "embedding_model": "bit-embed",
            "stt_model": "bit-stt",
            "tts_model": "bit-tts",
            "image_model": "bit-image",
            "vision_model": "bit-vision",
            "classification": { "classification": "internal", "pii": false },
            "capabilities": ["internet", "computer_use"],
            "profile_id": "profile-1",
        })))
        .unwrap();
    f.store.attach_app(&ada.id, "app-1").unwrap();
    f.store.attach_app(&ada.id, "app-2").unwrap();
    f.store
        .set_secret(&ada.id, "API_KEY", "s3cret", Some(Some("Search".into())))
        .unwrap();
    f.store.set_secret(&ada.id, "TOKEN", "t0ken", None).unwrap();
    let cy = f.bot("Cy");
    let bee = f.bot("Bee");
    let direct = f.direct(&ada).id;
    say(
        &f,
        &direct,
        LOCAL_OWNER_ID,
        "m1",
        "message",
        json!({ "text": "hi" }),
    );
    say(
        &f,
        &direct,
        &ada.id,
        "m2",
        "message",
        json!({ "text": "hello" }),
    );
    say(
        &f,
        &direct,
        LOCAL_OWNER_ID,
        "m3",
        "reaction",
        json!({ "target": "m2" }),
    );
    say(
        &f,
        &direct,
        LOCAL_OWNER_ID,
        "m4",
        "edit",
        json!({ "target": "m1", "text": "hi!" }),
    );
    later();
    let team = group(&f, "Team", &[&ada, &bee]);
    say(
        &f,
        &team,
        &bee.id,
        "g1",
        "message",
        json!({ "text": "status?" }),
    );
    say(
        &f,
        &team,
        LOCAL_OWNER_ID,
        "g2",
        "message",
        json!({ "text": "fine" }),
    );
    later();
    let waiting = group(&f, "Later", &[&ada, &cy]);
    say(
        &f,
        &waiting,
        &cy.id,
        "w1",
        "message",
        json!({ "text": "later" }),
    );
    moved_earlier(&f, &bee, "bot_b");
    World {
        f,
        hub: FakeHub::new(),
        mover: BotMover::default(),
        steps: Steps::default(),
        ada,
        cy,
        direct,
        team,
        waiting,
    }
}

impl World {
    async fn check(&self, bot_id: &str) -> Result<MoveReport, MoveError> {
        self.mover.check(&self.f.store, &self.hub, bot_id).await
    }

    async fn move_bot(&self, bot_id: &str) -> Result<MovedBot, MoveError> {
        self.mover
            .move_bot(&self.f.store, &self.hub, bot_id, &|progress| {
                self.steps.record(progress)
            })
            .await
    }

    fn here(&self, bot_id: &str) -> bool {
        self.f.store.get_bot(bot_id).unwrap().is_some()
    }

    fn conversations(&self) -> BTreeSet<String> {
        self.f
            .store
            .conversations()
            .unwrap()
            .into_iter()
            .map(|conversation| conversation.id)
            .collect()
    }

    fn clean_move(&self) -> Vec<String> {
        [
            "POST bots/import/check",
            "GET bots",
            "POST bots/import",
            "PATCH bots/bot_acct1",
            "GET bots/bot_acct1/apps",
            "PUT bots/bot_acct1/secrets/API_KEY",
            "PUT bots/bot_acct1/secrets/TOKEN",
            &format!("POST inbox/import {}", self.direct),
            &format!("POST inbox/import {}", self.team),
        ]
        .map(str::to_string)
        .to_vec()
    }
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// The caller first, then the other members in any order.
fn members(request: &Value) -> (String, BTreeSet<String>) {
    let mut ids = request["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string());
    (ids.next().unwrap(), ids.collect())
}

#[tokio::test]
async fn a_check_reports_what_moves_now_and_what_waits_for_other_local_bots() {
    let w = world();
    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!(
        report,
        MoveReport {
            bot_id: w.ada.id.clone(),
            issues: Vec::new(),
            apps: 2,
            secrets: 2,
            conversations: 2,
            messages: 4,
            waiting_conversations: 1,
            skipped_messages: 0,
            segments: 0,
            warnings: 0,
        }
    );
    assert_eq!(
        sorted_keys(&serde_json::to_value(&report).unwrap()),
        [
            "apps",
            "bot_id",
            "conversations",
            "issues",
            "messages",
            "secrets",
            "segments",
            "skipped_messages",
            "waiting_conversations",
            "warnings"
        ]
    );
    assert_eq!(w.hub.log(), ["POST bots/import/check", "GET bots"]);
    assert!(w.here(&w.ada.id));
    assert_eq!(w.conversations().len(), 3, "a check changes nothing");
}

#[tokio::test]
async fn the_check_sends_the_local_id_as_import_key_with_settings_apps_and_creation_time() {
    let w = world();
    w.check(&w.ada.id).await.unwrap();
    let bot = w.f.store.get_bot(&w.ada.id).unwrap().unwrap();
    assert_eq!(
        w.hub.account().checks,
        [json!({
            "local_id": w.ada.id,
            "name": "Ada",
            "preset_id": "preset-helper",
            "description": "Ada helps",
            "instructions": "Be brief.",
            "appearance": { "kind": "pip" },
            "model": { "primary": "bit-1", "fallbacks": ["bit-2"] },
            "embedding_model": "bit-embed",
            "stt_model": "bit-stt",
            "tts_model": "bit-tts",
            "image_model": "bit-image",
            "vision_model": "bit-vision",
            "subtask_model": null,
            "classification": { "classification": "internal", "pii": false },
            "capabilities": ["internet", "computer_use"],
            "profile_id": "profile-1",
            "location": "desktop",
            "device_id": null,
            "app_ids": ["app-1", "app-2"],
            "created_at": bot.created_at,
        })]
    );
}

#[tokio::test]
async fn the_accounts_issues_pass_through() {
    let w = world();
    w.hub.account().issues = vec![json!({
        "part": "app",
        "code": "BOT_APP_UNAVAILABLE",
        "message": "You can't use app app-2",
        "ref_id": "app-2",
    })];
    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!(
        report.issues,
        [MoveIssue {
            part: "app".into(),
            code: "BOT_APP_UNAVAILABLE".into(),
            message: "You can't use app app-2".into(),
            ref_id: Some("app-2".into()),
        }]
    );
}

#[tokio::test]
async fn a_secret_this_device_cant_open_is_an_issue_and_the_move_refuses_before_writing() {
    let w = world();
    break_secret(&w.f.path, &w.ada.id, "TOKEN", "API_KEY");

    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!(report.secrets, 2);
    assert_eq!(report.issues, [unreadable("TOKEN")]);

    w.hub.clear_log();
    let error = w.move_bot(&w.ada.id).await.unwrap_err();
    assert_eq!(error.code, BOT_SECRET_UNREADABLE);
    assert_eq!(error.issue.as_deref(), Some(&unreadable("TOKEN")));
    assert_eq!(w.hub.log(), ["POST bots/import/check", "GET bots"]);
    assert!(w.hub.account().secrets.is_empty());
    assert!(w.here(&w.ada.id));
}

#[tokio::test]
async fn a_locked_keychain_fails_the_check_with_secret_key_and_no_issue() {
    let w = world();
    let locked = Arc::new(w.f.open_again(TestKey::refusing()).unwrap());
    let error = w.mover.check(&locked, &w.hub, &w.ada.id).await.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.step, error.issue),
        (codes::SECRET_KEY, MoveStep::Check, None)
    );
    assert_eq!(
        error.message,
        format!(
            "Could not export local bot {}: the keychain is locked",
            w.ada.id
        )
    );
    assert!(w.hub.log().is_empty());
}

#[tokio::test]
async fn entries_from_senders_outside_the_chat_and_the_account_stay_out_and_are_counted() {
    let w = world();
    let gone = w.f.bot("Gone");
    let old = w.f.bot("Old");
    w.f.store
        .add_members(&w.team, vec![gone.id.clone(), old.id.clone()])
        .unwrap();
    say(
        &w.f,
        &w.team,
        &gone.id,
        "x1",
        "message",
        json!({ "text": "bye" }),
    );
    say(
        &w.f,
        &w.team,
        &gone.id,
        "x2",
        "reaction",
        json!({ "target": "g1" }),
    );
    say(
        &w.f,
        &w.team,
        &old.id,
        "x3",
        "message",
        json!({ "text": "old" }),
    );
    w.f.store.remove_member(&w.team, &old.id).unwrap();
    w.f.store.delete_bot(&gone.id).unwrap();
    moved_earlier(&w.f, &old, "bot_old");
    w.hub.add_bot("bot_old");

    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!((report.messages, report.skipped_messages), (5, 1));

    w.move_bot(&w.ada.id).await.unwrap();
    let [request] = w.hub.imports(&w.team).try_into().unwrap();
    let senders: Vec<(&str, &str)> = request["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["message_id"].as_str().unwrap(),
                entry["sender_id"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(senders, [("g1", "bot_b"), ("g2", SUB), ("x3", "bot_old")]);
}

#[tokio::test]
async fn a_move_carries_connectors_and_leaves_oauth_grants_behind_with_a_warning() {
    use flow_like_types::bot_connector::{ConnectorAuth, ConnectorKind, CreateBotConnector};
    let w = world();
    let connector = |label: &str, auth: ConnectorAuth, grant: Option<&str>| CreateBotConnector {
        kind: ConnectorKind::Mcp,
        label: label.into(),
        url: Some(format!("https://{}.example.com/mcp", label.to_lowercase())),
        auth,
        grant: grant.map(str::to_string),
        scopes: Vec::new(),
        default_policy: None,
    };
    for request in [
        connector("Docs", ConnectorAuth::None, None),
        connector(
            "Search",
            ConnectorAuth::Secret {
                name: "API_KEY".into(),
            },
            None,
        ),
        connector(
            "Mail",
            ConnectorAuth::Oauth {
                provider_id: "microsoft".into(),
            },
            Some("fl_oauth_sealed"),
        ),
    ] {
        w.f.store.create_connector(&w.ada.id, request).unwrap();
    }
    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!(report.warnings, 1);

    let moved = w.move_bot(&w.ada.id).await.unwrap();
    assert_eq!(moved.warnings, 1);
    let posted = w.hub.account().connectors.clone();
    let labels: Vec<&str> = posted.iter().filter_map(|c| c["label"].as_str()).collect();
    assert_eq!(labels, ["Docs", "Search"]);
    assert_eq!(posted[1]["auth"]["name"], json!("API_KEY"));
    assert_eq!(posted[0]["url"], json!("https://docs.example.com/mcp"));
    assert!(
        posted
            .iter()
            .all(|c| c["grant"].is_null() && !c.to_string().contains("fl_oauth_sealed"))
    );
}

#[tokio::test]
async fn a_move_imports_the_bot_then_its_settings_apps_secrets_and_ready_chats_and_removes_it_last()
{
    let w = world();
    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(
        (&moved.bot["id"], &moved.bot["owner_id"], &moved.bot["name"]),
        (&json!("bot_acct1"), &json!(SUB), &json!("Ada"))
    );
    assert_eq!(w.hub.log(), w.clean_move());
    assert_eq!(w.steps.of(&w.ada.id), CLEAN_STEPS);
    let (settings, apps) = w.hub.bot_of(&w.ada.id);
    assert_eq!(
        (&settings["name"], &settings["instructions"]),
        (&json!("Ada"), &json!("Be brief."))
    );
    assert_eq!(apps, ["app-1", "app-2"]);
    let secrets = w.hub.account().secrets.clone();
    assert_eq!(
        secrets,
        [
            (
                "bot_acct1".to_string(),
                "API_KEY".to_string(),
                "s3cret".to_string()
            ),
            (
                "bot_acct1".to_string(),
                "TOKEN".to_string(),
                "t0ken".to_string()
            ),
        ]
    );
    assert_eq!(
        moved.conversations,
        [
            MovedConversation {
                local_id: w.direct.clone(),
                account_id: "chat_acct1".into(),
            },
            MovedConversation {
                local_id: w.team.clone(),
                account_id: "chat_acct2".into(),
            },
        ]
    );
    assert!(!w.here(&w.ada.id));
    assert_eq!(w.conversations(), set(&[&w.waiting]));
    let waiting: BTreeSet<String> =
        w.f.visible(&w.waiting)
            .members
            .into_iter()
            .map(|member| member.principal_id)
            .collect();
    assert_eq!(waiting, set(&[LOCAL_OWNER_ID, "bot_acct1", &w.cy.id]));
}

#[tokio::test]
async fn moved_chats_name_the_person_and_the_account_bot_and_leave_join_events_to_the_server() {
    let w = world();
    let mut local = w.f.all_entries(&w.direct);
    local.reverse();
    w.move_bot(&w.ada.id).await.unwrap();

    let [direct] = w.hub.imports(&w.direct).try_into().unwrap();
    let account = |sender: &str| {
        if sender == LOCAL_OWNER_ID {
            SUB
        } else {
            "bot_acct1"
        }
    };
    let entries: Vec<Value> = local
        .iter()
        .map(|entry| {
            json!({
                "message_id": entry.message_id,
                "sender_id": account(&entry.sender_id),
                "kind": entry.kind,
                "envelope": entry.envelope,
                "created_at": entry.created_at,
            })
        })
        .collect();
    assert_eq!(
        direct,
        json!({
            "local_id": w.direct,
            "kind": "direct",
            "title": null,
            "members": [SUB, "bot_acct1"],
            "entries": entries,
        })
    );
    assert_eq!(message_ids(&direct["entries"]), ["m1", "m2", "m3", "m4"]);

    let [team] = w.hub.imports(&w.team).try_into().unwrap();
    assert_eq!(
        members(&team),
        (SUB.to_string(), set(&["bot_acct1", "bot_b"]))
    );
    assert_eq!(team["title"], "Team");
    assert_eq!(message_ids(&team["entries"]), ["g1", "g2"]);
    assert_eq!(team["entries"][0]["sender_id"], "bot_b");
    assert_eq!(team["entries"][1]["sender_id"], SUB);
}

#[tokio::test]
async fn a_direct_chat_the_account_already_holds_takes_the_history() {
    let w = world();
    w.hub
        .account()
        .existing_direct
        .insert(w.direct.clone(), "chat_existing".into());
    bulk_messages(
        &w.f,
        &w.direct,
        LOCAL_OWNER_ID,
        "x",
        997,
        &json!({ "text": "hi" }),
    );

    let moved = w.move_bot(&w.ada.id).await.unwrap();

    let imports: Vec<String> = w
        .hub
        .account()
        .imports
        .iter()
        .map(|body| body["local_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        imports,
        [w.direct.as_str(), w.direct.as_str(), w.team.as_str()]
    );
    assert_eq!(w.hub.account().chats["chat_existing"].len(), 1_001);
    assert_eq!(
        moved.conversations[0],
        MovedConversation {
            local_id: w.direct.clone(),
            account_id: "chat_existing".into(),
        }
    );
}

#[tokio::test]
async fn a_move_refuses_with_the_first_issue_and_its_code_changing_nothing() {
    let w = world();
    w.hub.account().issues = vec![
        json!({ "part": "app", "code": "BOT_APP_UNAVAILABLE", "message": "You can't use app app-2", "ref_id": "app-2" }),
        json!({ "part": "profile", "code": "BOT_PROFILE_UNAVAILABLE", "message": "Profile profile-1 is not yours", "ref_id": "profile-1" }),
    ];
    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, "BOT_APP_UNAVAILABLE");
    assert_eq!(error.step, MoveStep::Check);
    assert_eq!(
        error
            .issue
            .as_ref()
            .and_then(|issue| issue.ref_id.as_deref()),
        Some("app-2")
    );
    assert!(error.message.contains("You can't use app app-2"), "{error}");
    assert_eq!(w.hub.log(), ["POST bots/import/check", "GET bots"]);
    assert!(w.here(&w.ada.id));
    assert_eq!(w.conversations().len(), 3);
}

#[tokio::test]
async fn a_move_interrupted_while_importing_chats_completes_when_repeated_under_the_same_ids() {
    let w = world();
    w.hub.account().fail_next_import_of = Some(w.team.clone());

    let error = w.move_bot(&w.ada.id).await.unwrap_err();
    assert_eq!(
        error.message,
        format!(
            "Could not move conversation {} of bot {}: Network unavailable: POST inbox/import",
            w.team, w.ada.id
        )
    );
    assert_eq!(
        (error.code.as_str(), error.step),
        (HUB_UNAVAILABLE, MoveStep::Conversations)
    );
    assert_eq!(w.steps.count(MoveStep::ForgetConversations), 0);
    assert!(w.here(&w.ada.id));
    assert_eq!(w.conversations().len(), 3);

    w.hub.clear_log();
    w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(w.hub.log(), w.clean_move());
    assert_eq!(w.hub.account().bots.len(), 2);
    assert_eq!(w.hub.chat(&w.direct).len(), 4);
    assert_eq!(w.hub.chat(&w.team).len(), 2);
    assert!(!w.here(&w.ada.id));
    assert_eq!(w.conversations(), set(&[&w.waiting]));
}

#[tokio::test]
async fn a_repeated_move_carries_over_what_changed_here_in_between() {
    let w = world();
    w.hub.account().fail_next_import_of = Some(w.direct.clone());
    w.move_bot(&w.ada.id).await.unwrap_err();

    w.f.store
        .update_bot(&w.ada.id, patch(json!({ "name": "Ada Lovelace" })))
        .unwrap();
    w.f.store.detach_app(&w.ada.id, "app-1").unwrap();
    w.f.store.attach_app(&w.ada.id, "app-3").unwrap();
    w.hub.clear_log();
    w.move_bot(&w.ada.id).await.unwrap();

    let (settings, apps) = w.hub.bot_of(&w.ada.id);
    assert_eq!(settings["name"], "Ada Lovelace");
    assert_eq!(apps, ["app-2", "app-3"]);
    let log = w.hub.log();
    assert!(
        log.contains(&"PUT bots/bot_acct1/apps/app-3".to_string()),
        "{log:?}"
    );
    assert!(
        log.contains(&"DELETE bots/bot_acct1/apps/app-1".to_string()),
        "{log:?}"
    );
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_message_sent_here_during_the_move_is_carried_over_before_the_chat_goes() {
    let w = world();
    let (store, direct) = (w.f.store.clone(), w.direct.clone());
    w.steps.before(MoveStep::ForgetConversations, 1, move || {
        send_text(&store, &direct, "late")
    });

    w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(
        w.hub.imported_ids(&w.direct),
        [vec!["m1", "m2", "m3", "m4"], vec!["late"]]
    );
    assert_eq!(w.hub.chat(&w.direct), ["m1", "m2", "m3", "m4", "late"]);
    assert!(!w.conversations().contains(&w.direct));
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_chat_started_here_during_the_move_moves_with_the_bot() {
    let w = world();
    let started = Arc::new(Mutex::new(None::<String>));
    let (store, ada, note) = (w.f.store.clone(), w.ada.id.clone(), started.clone());
    w.steps.before(MoveStep::ForgetConversations, 1, move || {
        let chat = store
            .create_conversation(conversation(
                json!({ "kind": "group", "title": "New", "members": [ada] }),
            ))
            .unwrap();
        send_text(&store, &chat.id, "n1");
        *note.lock().unwrap() = Some(chat.id);
    });

    w.move_bot(&w.ada.id).await.unwrap();

    let chat = started.lock().unwrap().clone().unwrap();
    assert_eq!(w.hub.imported_ids(&chat), [vec!["n1"]]);
    assert!(!w.conversations().contains(&chat));
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_bot_edited_here_during_the_move_is_updated_again_before_it_goes() {
    let w = world();
    let (store, ada) = (w.f.store.clone(), w.ada.id.clone());
    w.steps.before(MoveStep::Rebind, 1, move || {
        store
            .update_bot(&ada, patch(json!({ "name": "Ada 2" })))
            .unwrap();
    });

    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.bot["name"], "Ada 2");
    let patches = w
        .hub
        .log()
        .iter()
        .filter(|line| line.starts_with("PATCH"))
        .count();
    assert_eq!(patches, 2);
    assert_eq!(w.steps.count(MoveStep::ForgetBot), 2);
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_bot_that_keeps_changing_here_stops_after_five_passes_and_stays() {
    let w = world();
    let (store, direct) = (w.f.store.clone(), w.direct.clone());
    let mut sent = 0;
    w.steps
        .before(MoveStep::ForgetConversations, usize::MAX, move || {
            send_text(&store, &direct, &format!("busy{sent}"));
            sent += 1;
        });

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, codes::LOCAL_CHANGED);
    assert!(
        error
            .message
            .contains("it kept changing on this computer while it moved"),
        "{error}"
    );
    assert_eq!(w.steps.count(MoveStep::Check), 1);
    assert_eq!(w.steps.count(MoveStep::Verify), 5);
    assert_eq!(w.steps.count(MoveStep::Rebind), 0);
    assert_eq!(w.steps.count(MoveStep::ForgetBot), 0);
    assert!(w.here(&w.ada.id));
    assert!(w.conversations().contains(&w.direct));
    assert_eq!(
        error.conversations,
        [MovedConversation {
            local_id: w.team.clone(),
            account_id: "chat_acct2".into(),
        }],
        "the chat that did move is reported, so the caller can follow it"
    );
}

#[tokio::test]
async fn a_secret_that_cant_be_opened_when_it_uploads_stops_the_move_with_its_issue() {
    let w = world();
    let (path, ada) = (w.f.path.clone(), w.ada.id.clone());
    w.steps.before(MoveStep::Secrets, 1, move || {
        break_secret(&path, &ada, "API_KEY", "TOKEN")
    });

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(
        (error.code.as_str(), error.step),
        (BOT_SECRET_UNREADABLE, MoveStep::Secrets)
    );
    assert_eq!(error.issue.as_deref(), Some(&unreadable("API_KEY")));
    assert!(w.hub.account().imports.is_empty());
    assert!(
        w.hub.account().secrets.is_empty(),
        "no value leaves before every secret opens"
    );
    assert!(w.here(&w.ada.id));
}

#[tokio::test]
async fn a_move_interrupted_while_removing_the_local_copy_completes_when_repeated() {
    let w = world();
    let path = w.f.path.clone();
    w.steps.before(MoveStep::ForgetBot, 1, move || {
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER refuse_forget BEFORE DELETE ON bots
                 BEGIN SELECT RAISE(ABORT, 'database is locked'); END;",
            )
            .unwrap();
    });

    let error = w.move_bot(&w.ada.id).await.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.step),
        (codes::STORAGE, MoveStep::ForgetBot)
    );
    assert!(
        error.message.starts_with(&format!(
            "Could not remove local bot {} from this computer: ",
            w.ada.id
        )) && error.message.ends_with("database is locked"),
        "{error}"
    );
    assert_eq!(w.conversations(), set(&[&w.waiting]));

    w.f.raw()
        .execute_batch("DROP TRIGGER refuse_forget;")
        .unwrap();
    w.hub.clear_log();
    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.bot["id"], "bot_acct1");
    assert_eq!(
        w.hub.log(),
        [
            "POST bots/import/check",
            "GET bots",
            "POST bots/import",
            "PATCH bots/bot_acct1",
            "GET bots/bot_acct1/apps",
            "PUT bots/bot_acct1/secrets/API_KEY",
            "PUT bots/bot_acct1/secrets/TOKEN",
        ]
    );
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn moves_run_one_at_a_time_so_a_later_bot_sees_the_earlier_one_in_the_account() {
    let w = world();
    let (first, second) = tokio::join!(w.move_bot(&w.ada.id), w.move_bot(&w.cy.id));

    assert_eq!(
        [&first.unwrap().bot["id"], &second.unwrap().bot["id"]],
        [&json!("bot_acct1"), &json!("bot_acct2")]
    );
    let seen = w.steps.seen.lock().unwrap().clone();
    let ada_gone = seen
        .iter()
        .position(|progress| progress.bot_id == w.ada.id && progress.step == MoveStep::Done)
        .unwrap();
    let cy_checked = seen
        .iter()
        .position(|progress| progress.bot_id == w.cy.id && progress.step == MoveStep::Check)
        .unwrap();
    assert!(ada_gone < cy_checked);
    let last = w.hub.account().imports.last().cloned().unwrap();
    assert_eq!(last["local_id"], json!(w.waiting));
    assert_eq!(
        members(&last),
        (SUB.to_string(), set(&["bot_acct1", "bot_acct2"]))
    );
    assert_eq!(message_ids(&last["entries"]), ["w1"]);
    assert_eq!(last["entries"][0]["sender_id"], "bot_acct2");
    assert!(w.conversations().is_empty());
}

#[tokio::test]
async fn an_empty_chat_moves_and_goes_only_while_it_stays_empty() {
    let w = world();
    let quiet = w.f.bot("Quiet");
    let still = w.f.direct(&quiet).id;
    w.move_bot(&quiet.id).await.unwrap();
    assert_eq!(w.hub.imported_ids(&still), [Vec::<String>::new()]);
    assert!(!w.conversations().contains(&still));

    let busy = w.f.bot("Busy");
    let chat = w.f.direct(&busy).id;
    let (store, written) = (w.f.store.clone(), chat.clone());
    w.steps.before(MoveStep::ForgetConversations, 1, move || {
        send_text(&store, &written, "first")
    });
    w.move_bot(&busy.id).await.unwrap();

    assert_eq!(
        w.hub.imported_ids(&chat),
        [Vec::<String>::new(), vec!["first".to_string()]],
        "a chat exported empty is forgotten only up to 0:0, so a first message keeps it"
    );
    assert_eq!(w.hub.chat(&chat), ["first"]);
    assert!(!w.conversations().contains(&chat));
}

#[tokio::test]
async fn a_long_chat_moves_in_batches_and_goes_after_its_last_batch() {
    let f = fixture();
    let (hub, mover, steps) = (FakeHub::new(), BotMover::default(), Steps::default());
    let bot = f.bot("Talker");
    let chat = f.direct(&bot).id;
    bulk_messages(&f, &chat, LOCAL_OWNER_ID, "l", 2_100, &json!({}));

    mover
        .move_bot(&f.store, &hub, &bot.id, &|progress| steps.record(progress))
        .await
        .unwrap();

    let sizes: Vec<usize> = hub.imported_ids(&chat).iter().map(Vec::len).collect();
    assert_eq!(sizes, [1_000, 1_000, 100]);
    assert_eq!(hub.chat(&chat).len(), 2_100);
    assert!(f.store.conversation(&chat).unwrap().is_none());
}

/// This device's Assistant or Flux, beside an account whose own runs where `location` says.
struct BuiltInWorld {
    f: Fixture,
    hub: FakeHub,
    mover: BotMover,
    steps: Steps,
    bot: Bot,
}

fn built_in_world(kind: BotSystemKind, location: &str) -> BuiltInWorld {
    let f = fixture();
    let hub = FakeHub::new();
    hub.add_built_in(&format!("bot_{}", kind.as_str()), kind.as_str(), location);
    f.store.ensure_built_ins().unwrap();
    let bot = f
        .store
        .list_bots()
        .unwrap()
        .into_iter()
        .find(|bot| bot.system == Some(kind))
        .unwrap();
    BuiltInWorld {
        f,
        hub,
        mover: BotMover::default(),
        steps: Steps::default(),
        bot,
    }
}

impl BuiltInWorld {
    fn edit(&self, body: Value) {
        self.f.store.update_bot(&self.bot.id, patch(body)).unwrap();
    }

    async fn move_bot(&self) -> Result<MovedBot, MoveError> {
        self.mover
            .move_bot(&self.f.store, &self.hub, &self.bot.id, &|progress| {
                self.steps.record(progress)
            })
            .await
    }
}

#[tokio::test]
async fn an_edited_assistant_gives_its_edits_apps_and_chats_to_the_accounts_own() {
    let w = built_in_world(BotSystemKind::General, "server");
    w.edit(json!({ "name": "Ally", "description": "Helps" }));
    w.f.store.attach_app(&w.bot.id, "app-1").unwrap();
    let chat = w.f.direct(&w.bot).id;
    w.f.text(&chat, LOCAL_OWNER_ID, "hi");

    let report = w.mover.check(&w.f.store, &w.hub, &w.bot.id).await.unwrap();
    assert_eq!(
        (
            report.issues.len(),
            report.apps,
            report.conversations,
            report.messages
        ),
        (0, 1, 1, 1)
    );
    let moved = w.move_bot().await.unwrap();

    assert_eq!(moved.bot["id"], "bot_general");
    let log = w.hub.log();
    assert!(
        !log.iter()
            .any(|line| line.contains("import/check") || line == "POST bots/import"),
        "{log:?}"
    );
    assert_eq!(
        w.hub.account().patches,
        [json!({ "name": "Ally", "description": "Helps" })],
        "only what was changed here"
    );
    let (settings, apps) = w.hub.bot("bot_general");
    assert_eq!(
        (&settings["instructions"], &settings["location"]),
        (&json!("From the web"), &json!("server")),
        "the rest stays the account's"
    );
    assert_eq!(apps, ["app-web", "app-1"], "its own apps stay");
    let [request] = w.hub.imports(&chat).try_into().unwrap();
    assert_eq!(request["members"], json!([SUB, "bot_general"]));
    assert_eq!(request["entries"][0]["sender_id"], SUB);
    assert!(
        !w.steps
            .of(&w.bot.id)
            .iter()
            .any(|(step, _)| *step == MoveStep::ImportBot)
    );
    assert_eq!(w.f.store.get_bot(&w.bot.id).unwrap(), None);
    assert_eq!(
        moved.conversations,
        [MovedConversation {
            local_id: chat,
            account_id: "chat_acct1".into(),
        }]
    );
}

#[tokio::test]
async fn a_built_in_used_only_in_a_chat_changes_nothing_on_the_account_but_its_chat_moves() {
    let w = built_in_world(BotSystemKind::General, "server");
    let chat = w.f.direct(&w.bot).id;
    w.f.text(&chat, LOCAL_OWNER_ID, "hi");

    w.move_bot().await.unwrap();

    assert!(w.hub.account().patches.is_empty());
    assert_eq!(w.hub.imported_ids(&chat), [vec!["hi"]]);
    assert_eq!(w.f.store.get_bot(&w.bot.id).unwrap(), None);
}

#[tokio::test]
async fn an_assistant_that_uses_the_computer_waits_while_the_account_runs_it_on_the_server() {
    let w = built_in_world(BotSystemKind::General, "server");
    w.edit(json!({ "capabilities": ["internet", "computer_use"] }));

    let report = w.mover.check(&w.f.store, &w.hub, &w.bot.id).await.unwrap();
    assert_eq!(
        report.issues,
        [MoveIssue {
            part: "capability".into(),
            code: "BOT_CAPABILITY_LOCATION".into(),
            message: "Your account runs this bot on the server, which can't use 'computer_use'"
                .into(),
            ref_id: Some("computer_use".into()),
        }]
    );
    let error = w.move_bot().await.unwrap_err();
    assert_eq!(error.code, "BOT_CAPABILITY_LOCATION");
    assert!(w.hub.account().patches.is_empty());
    assert!(w.f.store.get_bot(&w.bot.id).unwrap().is_some());

    let desktop = built_in_world(BotSystemKind::General, "desktop");
    desktop.edit(json!({ "capabilities": ["internet", "computer_use"] }));
    desktop.move_bot().await.unwrap();
    assert_eq!(
        desktop.hub.account().patches,
        [json!({ "capabilities": ["internet", "computer_use"] })]
    );
}

#[tokio::test]
async fn fluxs_edits_move_without_its_capabilities() {
    let w = built_in_world(BotSystemKind::Flux, "server");
    w.edit(json!({ "name": "Builder", "embedding_model": "bit-embed" }));

    let moved = w.move_bot().await.unwrap();

    assert_eq!(
        w.hub.account().patches,
        [json!({ "name": "Builder", "embedding_model": "bit-embed" })]
    );
    assert_eq!(moved.bot["name"], "Builder");
}

fn exported<'a>(bundle: &'a crate::LocalBotBundle, id: &str) -> &'a crate::ExportedConversation {
    bundle
        .conversations
        .iter()
        .find(|item| item.conversation.id == id)
        .unwrap()
}

fn requests<'a>(
    item: &'a crate::ExportedConversation,
    ids: &AccountIds,
) -> Vec<ImportConversationRequest<'a>> {
    let entries: Vec<&crate::InboxEntry> = item.entries.iter().collect();
    import_conversation_requests(item, &entries, ids)
}

#[test]
fn import_batches_stay_within_1000_entries_and_2000_copies() {
    assert_eq!(
        [1, 2, 3, 50].map(import_batch_size),
        [1_000, 1_000, 666, 40]
    );
    let f = fixture();
    let (a, b) = (f.bot("A"), f.bot("B"));
    let direct = f.direct(&a).id;
    bulk_messages(&f, &direct, LOCAL_OWNER_ID, "d", 2_500, &json!({}));
    let crowd = group(&f, "Crowd", &[&a, &b]);
    bulk_messages(&f, &crowd, &a.id, "g", 1_500, &json!({}));
    let bundle = f.store.export_bot(&a.id).unwrap();
    let ids = AccountIds::new(SUB, []);

    let sizes = |id: &str| -> Vec<usize> {
        requests(exported(&bundle, id), &ids)
            .iter()
            .map(|request| request.entries.len())
            .collect()
    };
    assert_eq!(sizes(&direct), [1_000, 1_000, 500]);
    assert_eq!(sizes(&crowd), [666, 666, 168]);
    for id in [&direct, &crowd] {
        let all = requests(exported(&bundle, id), &ids);
        for request in &all {
            assert!(request.entries.len() <= 1_000);
            assert!(request.entries.len() * request.members.len() <= 2_000);
        }
        let order: Vec<&str> = all
            .iter()
            .flat_map(|request| request.entries.iter().map(|entry| entry.message_id))
            .collect();
        let local: Vec<&str> = exported(&bundle, id)
            .entries
            .iter()
            .filter(|entry| entry.kind.as_str() == "message")
            .map(|entry| entry.message_id.as_str())
            .collect();
        assert_eq!(order, local, "oldest first across batches");
    }
}

#[test]
fn import_batches_keep_each_body_under_4_mib_and_its_copies_under_7_mb() {
    let text = "ü".repeat(30_000);
    for bots in [1, 3] {
        let f = fixture();
        let members: Vec<Bot> = (0..bots).map(|n| f.bot(&format!("B{n}"))).collect();
        let chat = match members.as_slice() {
            [only] => f.direct(only).id,
            many => group(&f, "Big", &many.iter().collect::<Vec<_>>()),
        };
        bulk_messages(
            &f,
            &chat,
            LOCAL_OWNER_ID,
            "b",
            200,
            &json!({ "text": text }),
        );
        let bundle = f.store.export_bot(&members[0].id).unwrap();
        let all = requests(exported(&bundle, &chat), &AccountIds::new(SUB, []));

        assert!(all.len() > 1, "{bots} bots");
        for request in &all {
            assert!(serde_json::to_vec(request).unwrap().len() <= 4 * 1024 * 1024);
            let copies =
                serde_json::to_vec(&request.entries).unwrap().len() * request.members.len();
            assert!(copies <= 7_000_000, "{copies}");
        }
        let order: Vec<String> = all
            .iter()
            .flat_map(|request| {
                request
                    .entries
                    .iter()
                    .map(|entry| entry.message_id.to_string())
            })
            .collect();
        let expected: Vec<String> = (0..200).map(|n| format!("b{n}")).collect();
        assert_eq!(order, expected);
    }
}

#[test]
fn an_empty_chat_still_makes_one_request_and_join_events_stay_with_the_server() {
    let f = fixture();
    let bot = f.bot("A");
    let crew = f.group(&[&bot]).id;
    let bundle = f.store.export_bot(&bot.id).unwrap();
    let item = exported(&bundle, &crew);
    assert_eq!(item.entries.len(), 1, "the join event");

    let ids = AccountIds::new(SUB, [(bot.id.clone(), "bot_acct1".to_string())]);
    assert_eq!(
        serde_json::to_value(requests(item, &ids)).unwrap(),
        json!([{
            "local_id": crew,
            "kind": "group",
            "title": "Crew",
            "members": [SUB, "bot_acct1"],
            "entries": [],
        }])
    );
}

#[test]
fn a_secret_travels_as_its_value_and_description_or_null() {
    let secret = crate::SecretValue {
        name: "API_KEY".into(),
        value: zeroize::Zeroizing::new("sk-1".into()),
        description: None,
    };
    let bytes = HubBody::Secret(&secret).to_json().unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({ "value": "sk-1", "description": null })
    );
    assert!(HubBody::Empty.to_json().unwrap().is_none());
    let call = HubCall::new(HubMethod::Put, &["bots", "bot_1", "secrets", "API_KEY"])
        .with(HubBody::Secret(&secret));
    assert_eq!(call.to_string(), "PUT bots/bot_1/secrets/API_KEY");
    assert!(!format!("{call:?}").contains("sk-1"), "{call:?}");
}

#[test]
fn errors_reach_callers_as_code_message_step_and_what_moved() {
    let mut error = MoveError::new(
        MoveStep::Conversations,
        codes::LOCAL_CHANGED,
        "Kept changing",
    );
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({ "code": "LOCAL_CHANGED", "message": "Kept changing", "step": "conversations" })
    );
    error.issue = Some(Box::new(unreadable("TOKEN")));
    error.conversations = vec![MovedConversation {
        local_id: "c-1".into(),
        account_id: "chat_1".into(),
    }];
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({
            "code": "LOCAL_CHANGED",
            "message": "Kept changing",
            "step": "conversations",
            "issue": {
                "part": "secret",
                "code": "BOT_SECRET_UNREADABLE",
                "message": "Secret TOKEN can't be read on this computer anymore",
                "ref_id": "TOKEN",
            },
            "conversations": [{ "local_id": "c-1", "account_id": "chat_1" }],
        })
    );
}

#[test]
fn a_check_and_a_move_can_run_on_any_runtime_thread() {
    fn sendable<T: Send>(_: &T) {}
    let f = fixture();
    let (hub, mover) = (FakeHub::new(), BotMover::default());
    let listener = |_: MoveProgress| {};
    sendable(&mover.check(&f.store, &hub, "bot_x"));
    sendable(&mover.move_bot(&f.store, &hub, "bot_x", &listener));
}

#[test]
fn a_deleted_message_moves_empty_and_its_edits_stay_behind() {
    use flow_like_types::{bot_message::MessageBody, inbox::is_tombstone};

    let f = fixture();
    let bot = f.bot("A");
    let direct = f.direct(&bot).id;
    let person = LOCAL_OWNER_ID;
    say(
        &f,
        &direct,
        person,
        "msg_1",
        "message",
        json!({ "text": "the secret plan" }),
    );
    say(
        &f,
        &direct,
        person,
        "msg_2",
        "message",
        json!({ "text": "keep me" }),
    );
    say(
        &f,
        &direct,
        person,
        "e_1",
        "edit",
        json!({ "target": "msg_1", "text": "the secret plan v2" }),
    );
    say(
        &f,
        &direct,
        &bot.id,
        "r_1",
        "reaction",
        json!({ "target": "msg_1" }),
    );
    say(
        &f,
        &direct,
        person,
        "d_1",
        "delete",
        json!({ "target": "msg_1" }),
    );
    let bundle = f.store.export_bot(&bot.id).unwrap();
    let ids = AccountIds::new(SUB, []);

    let sent = requests(exported(&bundle, &direct), &ids);
    let [request] = sent.as_slice() else {
        panic!("one call carries the chat")
    };
    let body = serde_json::to_value(request).unwrap();
    assert_eq!(
        message_ids(&body["entries"]),
        ["msg_1", "msg_2", "r_1", "d_1"],
        "the edit of a deleted message stays behind, the deletion goes"
    );
    assert_eq!(
        body["entries"][0]["envelope"],
        plain_envelope(json!({ "text": "" }))
    );
    assert_eq!(body["entries"][1]["envelope"]["body"]["text"], "keep me");
    assert!(!body.to_string().contains("secret"));
    for entry in body["entries"].as_array().unwrap() {
        let envelope = &entry["envelope"];
        assert!(!is_tombstone(envelope), "{entry}");
        if matches!(entry["kind"].as_str(), Some("message" | "edit")) {
            MessageBody::parse(&envelope["body"])
                .unwrap_or_else(|error| panic!("the account refuses {entry}: {error}"));
        }
    }
}

const WORKSPACE: &str = "app-ws-ada";

fn give_workspace(w: &World) {
    w.f.raw()
        .execute(
            "UPDATE bots SET workspace_app_id = ?2 WHERE id = ?1",
            [&w.ada.id, WORKSPACE],
        )
        .unwrap();
    w.hub.local_files().lock().unwrap().insert(
        WORKSPACE.into(),
        BTreeMap::from([
            ("upload/a.txt".to_string(), 10),
            ("storage/db/t.lance/x".to_string(), 300),
            ("storage/notes.md".to_string(), 5),
        ]),
    );
}

fn account_files(w: &World, upload_path: &str) -> BTreeMap<String, u64> {
    w.hub
        .account()
        .remote_files
        .get(upload_path)
        .cloned()
        .unwrap_or_default()
}

fn log_of_workspace(w: &World) -> Vec<String> {
    w.hub
        .log()
        .into_iter()
        .filter(|line| {
            line.contains("workspace")
                || line.starts_with("UPLOAD")
                || line.starts_with("COMPARE")
                || line.starts_with("DELETE")
        })
        .collect()
}

fn position(steps: &[(MoveStep, u8)], step: MoveStep) -> usize {
    steps.iter().position(|(seen, _)| *seen == step).unwrap()
}

#[tokio::test]
async fn a_workspace_moves_with_the_bot_and_the_local_copy_goes_after_the_account_has_it_all() {
    let w = world();
    give_workspace(&w);

    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.bot["id"], "bot_acct1");
    let account = account_files(&w, "apps/ws_bot_acct1");
    assert_eq!(account.len(), 3);
    assert_eq!(account["storage/db/t.lance/x"], 300);
    assert_eq!(
        log_of_workspace(&w),
        [
            "POST bots/bot_acct1/workspace",
            "POST bots/bot_acct1/workspace/upload",
            "UPLOAD app-ws-ada",
            "COMPARE app-ws-ada",
            "POST bots/bot_acct1/workspace/upload",
            "COMPARE app-ws-ada",
            "DELETE app-ws-ada",
        ]
    );
    assert_eq!(
        w.hub.account().upload_requests,
        [
            json!({ "summary": { "total_size_bytes": 315, "total_object_count": 3 } }),
            json!({ "summary": { "total_size_bytes": 315, "total_object_count": 3 } }),
        ]
    );
    assert_eq!(w.hub.account().deleted_workspaces, [WORKSPACE]);
    assert!(w.hub.local_files().lock().unwrap().is_empty());
    let steps = w.steps.of(&w.ada.id);
    assert!(position(&steps, MoveStep::Workspace) < position(&steps, MoveStep::Conversations));
    assert!(position(&steps, MoveStep::Verify) < position(&steps, MoveStep::ForgetWorkspace));
    assert!(position(&steps, MoveStep::ForgetWorkspace) < position(&steps, MoveStep::Rebind));
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn files_written_while_the_bot_moves_are_sent_before_the_local_copy_goes() {
    let w = world();
    give_workspace(&w);
    let files = w.hub.local_files();
    w.steps.before(MoveStep::ForgetWorkspace, 1, move || {
        files
            .lock()
            .unwrap()
            .get_mut(WORKSPACE)
            .unwrap()
            .insert("upload/late.txt".into(), 7);
    });

    w.move_bot(&w.ada.id).await.unwrap();

    assert!(account_files(&w, "apps/ws_bot_acct1").contains_key("upload/late.txt"));
    assert_eq!(w.hub.account().uploads, 2);
    assert_eq!(w.hub.account().deleted_workspaces, [WORKSPACE]);
}

#[tokio::test]
async fn an_account_copy_that_lacks_files_stops_the_move_before_any_chat_and_keeps_everything() {
    let w = world();
    give_workspace(&w);
    w.hub.account().lose_a_file_from_upload = Some(1);

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, WORKSPACE_NOT_VERIFIED);
    assert_eq!(error.step, MoveStep::Workspace);
    assert!(w.here(&w.ada.id));
    assert!(w.hub.local_files().lock().unwrap().contains_key(WORKSPACE));
    assert!(w.hub.account().deleted_workspaces.is_empty());
    assert!(w.hub.imports(&w.direct).is_empty());
    assert_eq!(w.conversations().len(), 3);
}

#[tokio::test]
async fn a_file_lost_on_the_final_upload_keeps_the_local_copy_and_a_repeat_finishes() {
    let w = world();
    give_workspace(&w);
    let files = w.hub.local_files();
    w.steps.before(MoveStep::ForgetWorkspace, 1, move || {
        files
            .lock()
            .unwrap()
            .get_mut(WORKSPACE)
            .unwrap()
            .insert("upload/zzz.txt".into(), 7);
    });
    w.hub.account().lose_a_file_from_upload = Some(2);

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, WORKSPACE_NOT_VERIFIED);
    assert_eq!(error.step, MoveStep::ForgetWorkspace);
    assert!(w.here(&w.ada.id));
    assert!(w.hub.local_files().lock().unwrap().contains_key(WORKSPACE));
    assert!(w.hub.account().deleted_workspaces.is_empty());
    assert_eq!(w.conversations(), set(&[&w.waiting]), "the chats moved");

    w.hub.account().lose_a_file_from_upload = None;
    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.bot["id"], "bot_acct1");
    assert!(account_files(&w, "apps/ws_bot_acct1").contains_key("upload/zzz.txt"));
    assert_eq!(w.hub.account().deleted_workspaces, [WORKSPACE]);
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_move_interrupted_while_the_chats_import_repeats_with_the_same_workspace() {
    let w = world();
    give_workspace(&w);
    w.hub.account().fail_next_import_of = Some(w.direct.clone());

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, HUB_UNAVAILABLE);
    assert_eq!(error.step, MoveStep::Conversations);
    assert_eq!(account_files(&w, "apps/ws_bot_acct1").len(), 3);
    assert!(w.hub.account().deleted_workspaces.is_empty());
    assert!(w.hub.local_files().lock().unwrap().contains_key(WORKSPACE));

    w.move_bot(&w.ada.id).await.unwrap();

    let workspaces = w
        .hub
        .log()
        .iter()
        .filter(|line| line.as_str() == "POST bots/bot_acct1/workspace")
        .count();
    assert_eq!(workspaces, 2, "asking again returns the same workspace");
    assert_eq!(w.hub.account().deleted_workspaces, [WORKSPACE]);
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_local_copy_that_cannot_be_deleted_stays_and_the_repeat_deletes_it() {
    let w = world();
    give_workspace(&w);
    w.hub.account().fail_delete = true;

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, "STORAGE");
    assert_eq!(error.step, MoveStep::ForgetWorkspace);
    assert!(w.here(&w.ada.id));
    assert!(w.hub.local_files().lock().unwrap().contains_key(WORKSPACE));

    w.hub.account().fail_delete = false;
    w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(w.hub.account().deleted_workspaces, [WORKSPACE]);
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_workspace_already_deleted_here_moves_without_an_upload() {
    let w = world();
    give_workspace(&w);
    w.hub.local_files().lock().unwrap().clear();

    w.move_bot(&w.ada.id).await.unwrap();

    assert!(w.hub.log().iter().all(|line| !line.starts_with("UPLOAD")));
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_bot_without_a_workspace_neither_asks_for_one_nor_tells_the_steps() {
    let w = world();

    w.move_bot(&w.ada.id).await.unwrap();

    assert!(log_of_workspace(&w).is_empty());
    assert_eq!(w.steps.count(MoveStep::Workspace), 0);
    assert_eq!(w.steps.count(MoveStep::ForgetWorkspace), 0);
    assert_eq!(w.steps.count(MoveStep::Segments), 0);
}

fn handle(app: &str, path: &str) -> Value {
    json!({
        "app_id": app,
        "path": path,
        "bytes": 12,
        "sha256": "a".repeat(64),
        "mime": "text/plain",
    })
}

fn retarget_foreign_handles(w: &World, message_id: &str) {
    let db = w.f.raw();
    let stored: String = db
        .query_row(
            "SELECT envelope FROM entries WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut envelope: Value = serde_json::from_str(&stored).unwrap();
    for part in envelope["body"]["parts"].as_array_mut().unwrap() {
        let foreign = part["handle"]["path"]
            .as_str()
            .is_some_and(|path| path.starts_with("foreign/"));
        if foreign {
            part["handle"]["app_id"] = json!("app-other");
        }
    }
    db.execute(
        "UPDATE entries SET envelope = ?2 WHERE message_id = ?1",
        [message_id, &envelope.to_string()],
    )
    .unwrap();
}

fn message_with_files(w: &World) {
    say(
        &w.f,
        &w.direct,
        &w.ada.id,
        "files1",
        "message",
        json!({
            "text": "here",
            "parts": [
                { "kind": "attachment", "id": "a1", "name": "report.pdf",
                  "handle": handle(WORKSPACE, "files/report.pdf") },
                { "kind": "tool", "id": "t1", "call_id": "c1", "name": "search",
                  "summary": "found it", "handle": handle(WORKSPACE, "tool-results/c1.json") },
                { "kind": "attachment", "id": "a2", "name": "gone.txt",
                  "handle": handle(WORKSPACE, "foreign/gone.txt") },
                { "kind": "tool", "id": "t2", "call_id": "c2", "name": "read",
                  "summary": "read it", "handle": handle(WORKSPACE, "foreign/c2.json") },
            ],
        }),
    );
    retarget_foreign_handles(w, "files1");
}

#[tokio::test]
async fn handles_to_the_local_workspace_name_the_account_workspace_and_others_are_stripped() {
    let w = world();
    give_workspace(&w);
    message_with_files(&w);

    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!(report.warnings, 2);

    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.warnings, 2);
    let [direct] = w.hub.imports(&w.direct).try_into().unwrap();
    let sent = direct["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["message_id"] == "files1")
        .unwrap();
    let parts = sent["envelope"]["body"]["parts"].as_array().unwrap();
    assert_eq!(
        parts,
        &[
            json!({ "kind": "attachment", "id": "a1", "name": "report.pdf",
                    "handle": handle("ws_bot_acct1", "files/report.pdf") }),
            json!({ "kind": "tool", "id": "t1", "call_id": "c1", "name": "search",
                    "summary": "found it", "handle": handle("ws_bot_acct1", "tool-results/c1.json") }),
            json!({ "kind": "tool", "id": "t2", "call_id": "c2", "name": "read",
                    "summary": "read it" }),
        ]
    );
    MessageBody::parse(&sent["envelope"]["body"]).unwrap();
    assert_eq!(sent["envelope"]["body"]["text"], "here");
}

#[tokio::test]
async fn without_a_workspace_every_handle_is_stripped() {
    let w = world();
    give_workspace(&w);
    message_with_files(&w);
    w.f.raw()
        .execute(
            "UPDATE bots SET workspace_app_id = NULL WHERE id = ?1",
            [&w.ada.id],
        )
        .unwrap();

    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.warnings, 4);
    assert!(
        !w.hub.imports(&w.direct)[0]
            .to_string()
            .contains("\"handle\"")
    );
}

fn seq_of(w: &World, message_id: &str) -> i64 {
    w.f.all_entries(&w.direct)
        .iter()
        .find(|entry| entry.message_id == message_id)
        .and_then(|entry| entry.seq)
        .unwrap()
}

fn local_segment(w: &World, id: &str, level: i32, span: (i64, i64), parent: Option<&str>) {
    w.f.store
        .write_segment(
            &w.direct,
            &w.ada.id,
            WriteSegmentRequest {
                id: id.into(),
                level,
                seq_from: span.0,
                seq_to: span.1,
                title: Some(format!("About {id}")),
                summary: format!("Summary {id}"),
                parent_id: parent.map(str::to_string),
            },
        )
        .unwrap();
}

fn segment_world() -> World {
    let w = world();
    let (m1, m2, m3, m4) = (
        seq_of(&w, "m1"),
        seq_of(&w, "m2"),
        seq_of(&w, "m3"),
        seq_of(&w, "m4"),
    );
    local_segment(&w, "top", 1, (m1, m4), None);
    local_segment(&w, "early", 0, (m1, m2), Some("top"));
    local_segment(&w, "late", 0, (m3, m4), Some("top"));
    w
}

fn written_segments(w: &World) -> Vec<(String, String, i64, i64, Option<String>)> {
    w.hub
        .account()
        .segments
        .iter()
        .map(|(chat, bot, body)| {
            (
                chat.clone(),
                bot.clone(),
                body["seq_from"].as_i64().unwrap(),
                body["seq_to"].as_i64().unwrap(),
                body["parent_id"].as_str().map(str::to_string),
            )
        })
        .collect()
}

#[tokio::test]
async fn segments_move_with_their_chat_and_their_ranges_follow_the_accounts_positions() {
    let w = segment_world();
    w.hub.account().page_cap = 2;

    let report = w.check(&w.ada.id).await.unwrap();
    assert_eq!((report.segments, report.warnings), (3, 0));

    let first_here = seq_of(&w, "m1");
    let moved = w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(moved.warnings, 0);
    let chat = w.hub.account().chat_keys[&w.direct].clone();
    assert_ne!(first_here, 5, "the account numbers the chat differently");
    assert_eq!(
        written_segments(&w),
        [
            (chat.clone(), "bot_acct1".into(), 5, 8, None),
            (chat.clone(), "bot_acct1".into(), 5, 6, Some("top".into())),
            (chat, "bot_acct1".into(), 7, 8, Some("top".into())),
        ]
    );
    let reads = w
        .hub
        .log()
        .iter()
        .filter(|line| line.ends_with("/entries"))
        .count();
    assert_eq!(reads, 2, "four entries in pages of two");
    let steps = w.steps.of(&w.ada.id);
    assert!(position(&steps, MoveStep::Conversations) < position(&steps, MoveStep::Segments));
    assert!(position(&steps, MoveStep::Segments) < position(&steps, MoveStep::ForgetConversations));
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_summary_pointing_at_the_second_and_third_messages_still_does_after_the_move() {
    let w = world();
    local_segment(&w, "mid", 0, (seq_of(&w, "m2"), seq_of(&w, "m3")), None);

    w.move_bot(&w.ada.id).await.unwrap();

    let account = w.hub.account();
    let chat = &account.chat_keys[&w.direct];
    let seq_there = |message_id: &str| {
        let index = account.chats[chat]
            .iter()
            .position(|entry| entry["message_id"] == message_id)
            .unwrap();
        account.seqs[chat][index]
    };
    let (_, _, body) = &account.segments[0];
    assert_eq!(body["seq_from"], seq_there("m2"));
    assert_eq!(body["seq_to"], seq_there("m3"));
    assert_eq!(body["summary"], "Summary mid");
}

#[tokio::test]
async fn segments_interrupted_in_the_account_are_written_once_when_the_move_repeats() {
    let w = segment_world();
    w.hub.account().fail_segments_once = true;

    let error = w.move_bot(&w.ada.id).await.unwrap_err();

    assert_eq!(error.code, HUB_UNAVAILABLE);
    assert_eq!(error.step, MoveStep::Segments);
    assert_eq!(
        w.conversations().len(),
        3,
        "no chat is forgotten before its summaries moved"
    );
    assert!(w.here(&w.ada.id));

    w.move_bot(&w.ada.id).await.unwrap();

    assert_eq!(written_segments(&w).len(), 3);
    assert!(!w.here(&w.ada.id));
}

#[tokio::test]
async fn a_chat_without_summaries_is_never_read_back() {
    let w = world();

    w.move_bot(&w.ada.id).await.unwrap();

    assert!(w.hub.log().iter().all(|line| !line.ends_with("/entries")));
}

#[test]
fn a_summary_none_of_whose_entries_moved_is_dropped_and_counted() {
    let w = world();
    local_segment(&w, "mid", 0, (seq_of(&w, "m2"), seq_of(&w, "m3")), None);
    let bundle = w.f.store.export_bot(&w.ada.id).unwrap();
    let item = exported(&bundle, &w.direct);
    let ids = AccountIds::new(SUB, []);

    let nowhere = PositionMap::new(&[], &ids, &HashMap::new());
    let plan = segment_requests(&item.segments, &nowhere);
    assert_eq!((plan.requests.len(), plan.dropped), (0, 1));

    let moved: Vec<&crate::InboxEntry> = item.entries.iter().collect();
    assert_eq!(unmovable_segments(item, &[]), 1);
    assert_eq!(unmovable_segments(item, &moved), 0);
}

#[tokio::test]
async fn a_group_deleted_during_move_retries_its_account_deletion_before_finishing() {
    let w = world();
    let group = w.f.group(&[&w.ada]);
    w.f.text(&group.id, &w.ada.id, "group-message");
    let (store, id) = (w.f.store.clone(), group.id.clone());
    w.steps.before(MoveStep::ForgetConversations, 1, move || {
        store.delete_conversation(&id).unwrap();
    });
    w.hub.account().fail_chat_delete_once = true;
    assert!(w.move_bot(&w.ada.id).await.is_err());
    assert!(
        w.here(&w.ada.id),
        "the local bot stays until its group deletion reaches the account"
    );
    let account_chat = w.hub.account().chat_keys.get(&group.id).unwrap().clone();
    assert!(w.hub.account().deleted_chats.is_empty());
    let pending: i64 =
        w.f.raw()
            .query_row(
                "SELECT COUNT(*) FROM conversation_transfers WHERE conversation_id = ?1",
                [&group.id],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(pending, 1);
    w.move_bot(&w.ada.id).await.unwrap();
    assert!(w.hub.account().deleted_chats.contains(&account_chat));
    assert!(!w.here(&w.ada.id));
}
