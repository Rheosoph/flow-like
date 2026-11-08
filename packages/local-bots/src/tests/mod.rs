mod bots;
mod connectors;
mod deletion;
mod files;
mod history;
mod inbox;
mod paging;
mod permissions;
mod pins;
mod recovery;
mod room_decisions;
mod secrets;
mod skills;
mod storage;
mod tasks;
mod tool_grants;
mod transfer;
mod user_context;
mod workspace;

use crate::{
    Bot, ChangeNotifier, Conversation, CreateBotRequest, CreateConversationRequest, EntriesQuery,
    InboxEntry, KeyError, LocalBotStore, Result, SecretKeyProvider, SendEntryRequest,
    UpdateBotRequest,
};
use flow_like_types::inbox::plain_envelope;
use serde_json::{Value, json};
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

/// A keychain in memory that records whether each call asked to create the key.
#[derive(Default)]
pub(super) struct TestKey {
    key: Mutex<Option<[u8; 32]>>,
    refuse: bool,
    calls: Mutex<Vec<bool>>,
}

impl TestKey {
    pub fn holding(key: [u8; 32]) -> Self {
        Self {
            key: Mutex::new(Some(key)),
            ..Self::default()
        }
    }

    pub fn refusing() -> Self {
        Self {
            refuse: true,
            ..Self::default()
        }
    }

    pub fn calls(&self) -> Vec<bool> {
        self.calls.lock().unwrap().clone()
    }
}

impl SecretKeyProvider for TestKey {
    fn key(&self, create: bool) -> std::result::Result<Zeroizing<[u8; 32]>, KeyError> {
        self.calls.lock().unwrap().push(create);
        if self.refuse {
            return Err(KeyError::Unavailable("the keychain is locked".into()));
        }
        let mut key = self.key.lock().unwrap();
        if key.is_none() && create {
            *key = Some([7; 32]);
        }
        key.map(Zeroizing::new)
            .ok_or_else(|| KeyError::Missing("no key in this keychain".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Change {
    Bots(Option<String>),
    Inbox(Option<String>),
}

/// Rows a separate connection sees when a change is announced, which proves the write
/// committed before the announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Committed {
    pub bots: i64,
    pub conversations: i64,
    pub entries: i64,
}

pub(super) fn committed(path: &Path) -> Committed {
    let db = rusqlite::Connection::open(path).unwrap();
    let count = |table: &str| -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    Committed {
        bots: count("bots"),
        conversations: count("conversations"),
        entries: count("entries"),
    }
}

pub(super) struct Recorder {
    path: PathBuf,
    seen: Mutex<Vec<(Change, Committed)>>,
}

impl Recorder {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, change: Change) {
        let state = committed(&self.path);
        self.seen.lock().unwrap().push((change, state));
    }

    /// The announcements since the last call, with what was committed at each.
    pub fn take_with_state(&self) -> Vec<(Change, Committed)> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }

    pub fn take(&self) -> Vec<Change> {
        self.take_with_state()
            .into_iter()
            .map(|(change, _)| change)
            .collect()
    }
}

impl ChangeNotifier for Recorder {
    fn bots_changed(&self, bot_id: Option<&str>) {
        self.record(Change::Bots(bot_id.map(str::to_string)));
    }

    fn inbox_changed(&self, conversation_id: Option<&str>) {
        self.record(Change::Inbox(conversation_id.map(str::to_string)));
    }
}

pub(super) struct Fixture {
    pub store: Arc<LocalBotStore>,
    pub key: Arc<TestKey>,
    pub changes: Arc<Recorder>,
    pub path: PathBuf,
    _dir: TempDir,
}

pub(super) fn fixture() -> Fixture {
    fixture_with(TestKey::default())
}

pub(super) fn fixture_with(key: TestKey) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flow-like").join("local_bots.db");
    let key = Arc::new(key);
    let changes = Arc::new(Recorder::new(&path));
    let store = Arc::new(LocalBotStore::open(&path, key.clone(), changes.clone()).unwrap());
    Fixture {
        store,
        key,
        changes,
        path,
        _dir: dir,
    }
}

impl Fixture {
    /// Another store on the same file with its own key.
    pub fn open_again(&self, key: TestKey) -> Result<LocalBotStore> {
        LocalBotStore::open(
            &self.path,
            Arc::new(key),
            Arc::new(Recorder::new(&self.path)),
        )
    }

    pub fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }

    /// Whether the database file or its WAL holds `bytes` anywhere.
    pub fn on_disk(&self, bytes: &[u8]) -> bool {
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        [self.path.clone(), wal].iter().any(|file| {
            std::fs::read(file)
                .is_ok_and(|content| content.windows(bytes.len()).any(|window| window == bytes))
        })
    }

    pub fn bot(&self, name: &str) -> Bot {
        self.store
            .create_bot(create(
                json!({ "name": name, "appearance": { "kind": "pip" } }),
            ))
            .unwrap()
    }

    pub fn direct(&self, bot: &Bot) -> Conversation {
        self.store
            .create_conversation(conversation(
                json!({ "kind": "direct", "members": [bot.id] }),
            ))
            .unwrap()
    }

    pub fn group(&self, bots: &[&Bot]) -> Conversation {
        let members: Vec<&str> = bots.iter().map(|bot| bot.id.as_str()).collect();
        self.store
            .create_conversation(conversation(
                json!({ "kind": "group", "title": "Crew", "members": members }),
            ))
            .unwrap()
    }

    pub fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        message_id: &str,
        kind: &str,
        body: Value,
    ) -> Result<InboxEntry> {
        self.store.send(
            conversation_id,
            sender_id,
            entry_request(json!({
                "message_id": message_id,
                "kind": kind,
                "envelope": plain_envelope(body),
            })),
        )
    }

    pub fn text(&self, conversation_id: &str, sender_id: &str, message_id: &str) -> InboxEntry {
        self.send(
            conversation_id,
            sender_id,
            message_id,
            "message",
            json!({ "text": message_id }),
        )
        .unwrap()
    }

    /// A conversation the person is in.
    pub fn visible(&self, conversation_id: &str) -> Conversation {
        self.store
            .conversation(conversation_id)
            .unwrap()
            .unwrap_or_else(|| panic!("conversation {conversation_id} is not visible"))
    }

    /// Every event of a conversation, newest first.
    pub fn all_entries(&self, conversation_id: &str) -> Vec<InboxEntry> {
        self.store
            .entries(
                conversation_id,
                &EntriesQuery {
                    limit: Some(100),
                    ..EntriesQuery::default()
                },
            )
            .unwrap()
            .entries
    }

    /// The bodies of the conversation's member events, oldest first.
    pub fn member_bodies(&self, conversation_id: &str) -> Vec<Value> {
        let mut bodies: Vec<Value> = self
            .all_entries(conversation_id)
            .into_iter()
            .filter(|entry| entry.kind.as_str() == "member")
            .map(|entry| entry.envelope["body"].clone())
            .collect();
        bodies.reverse();
        bodies
    }
}

pub(super) fn create(body: Value) -> CreateBotRequest {
    serde_json::from_value(body).unwrap()
}

pub(super) fn patch(body: Value) -> UpdateBotRequest {
    serde_json::from_value(body).unwrap()
}

pub(super) fn conversation(body: Value) -> CreateConversationRequest {
    serde_json::from_value(body).unwrap()
}

pub(super) fn entry_request(body: Value) -> SendEntryRequest {
    serde_json::from_value(body).unwrap()
}

pub(super) fn code<T: Debug>(result: Result<T>) -> &'static str {
    result.unwrap_err().code()
}

/// Lets the wall clock move past the current millisecond.
pub(super) fn later() {
    std::thread::sleep(Duration::from_millis(3));
}

pub(super) fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}
