use super::{
    host::{
        BotHost, CODE_CANCELLED, CODE_STORE_CHANGED, CODE_TIMEOUT, HostConfig, RunError,
        TurnEvents, TurnInfo, TurnJob, TurnRunner,
    },
    rule::{Waiting, waiting},
    store::{HostStore, WINDOW},
};
use async_trait::async_trait;
use chrono::{DateTime, Duration as Age, FixedOffset, Utc};
use flow_like_local_bots::{
    Bot, ChangeNotifier, Conversation, ConversationSegmentList, EntriesQuery, InboxEntriesPage,
    InboxEntry, KeyError, LOCAL_OWNER_ID, LocalBotStore, PositionQuery, SecretKeyProvider,
    SegmentsQuery, SendEntryRequest, WriteSegmentRequest,
};
use flow_like_types::{
    bot_turn::{TurnStatus, failure_message_id, reply_message_id, turn_id},
    inbox::{ConversationKind, InboxEntryKind},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub(super) struct NoKey;

impl SecretKeyProvider for NoKey {
    fn key(&self, _create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        Err(KeyError::Missing("no key in these tests".into()))
    }
}

pub(super) struct Quiet;

impl ChangeNotifier for Quiet {
    fn bots_changed(&self, _bot_id: Option<&str>) {}
    fn inbox_changed(&self, _conversation_id: Option<&str>) {}
}

pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flow-like-bot-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct Direct(pub(super) Arc<LocalBotStore>);

#[async_trait]
impl HostStore for Direct {
    async fn conversations(&self) -> Result<Vec<Conversation>, String> {
        self.0.host_conversations().map_err(|e| e.to_string())
    }

    async fn conversation(&self, id: &str) -> Result<Option<Conversation>, String> {
        self.0.conversation(id).map_err(|e| e.to_string())
    }

    async fn newest(&self, id: &str) -> Result<Vec<InboxEntry>, String> {
        let Some(bot_id) = self
            .0
            .conversation(id)
            .map_err(|e| e.to_string())?
            .and_then(|conversation| {
                conversation.members.into_iter().find_map(|member| {
                    (member.kind == flow_like_types::inbox::PrincipalKind::Bot)
                        .then_some(member.principal_id)
                })
            })
        else {
            return Ok(Vec::new());
        };
        self.0
            .entries_as(
                id,
                &bot_id,
                &EntriesQuery {
                    limit: Some(WINDOW),
                    ..Default::default()
                },
            )
            .map(|page| page.entries)
            .map_err(|e| e.to_string())
    }

    async fn bot(&self, id: &str) -> Result<Option<Bot>, String> {
        self.0.get_bot(id).map_err(|e| e.to_string())
    }

    async fn room_limits(
        &self,
        conversation_id: &str,
        bot_id: &str,
    ) -> Result<Option<flow_like_types::bot_effective::BotRoomLimits>, String> {
        self.0
            .member_limits(conversation_id, bot_id)
            .map_err(|e| e.to_string())
    }

    async fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: SendEntryRequest,
    ) -> Result<InboxEntry, String> {
        self.0
            .send(conversation_id, sender_id, request)
            .map_err(|e| e.to_string())
    }

    async fn positions(
        &self,
        conversation_id: &str,
        bot_id: &str,
        query: PositionQuery,
    ) -> Result<InboxEntriesPage, String> {
        self.0
            .entries_by_position_as(conversation_id, bot_id, &query)
            .map_err(|e| e.to_string())
    }

    async fn segments(
        &self,
        conversation_id: &str,
        bot_id: &str,
        query: SegmentsQuery,
    ) -> Result<ConversationSegmentList, String> {
        self.0
            .segments_as(conversation_id, bot_id, &query)
            .map_err(|e| e.to_string())
    }

    async fn write_segment(
        &self,
        conversation_id: &str,
        author_id: &str,
        request: WriteSegmentRequest,
    ) -> Result<(), String> {
        self.0
            .write_segment(conversation_id, author_id, request)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn tool_keys(&self, bot_id: &str) -> Result<Vec<(String, String)>, String> {
        self.0.tool_keys(bot_id).map_err(|e| e.to_string())
    }

    async fn put_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<(), String> {
        self.0
            .put_tool_key(bot_id, tool, scope)
            .map_err(|e| e.to_string())
    }

    async fn delete_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<bool, String> {
        self.0
            .delete_tool_key(bot_id, tool, scope)
            .map_err(|e| e.to_string())
    }

    async fn park(
        &self,
        turn_id: &str,
        bot_id: &str,
        conversation_id: &str,
        trigger_message_id: &str,
        card: &flow_like_types::bot_message::ApprovalRequest,
    ) -> Result<(), String> {
        self.0
            .park_turn(turn_id, bot_id, conversation_id, trigger_message_id, card)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn approval_card(
        &self,
        approval_id: &str,
    ) -> Result<Option<flow_like_types::bot_message::ApprovalRequest>, String> {
        self.0.approval_card(approval_id).map_err(|e| e.to_string())
    }

    async fn resolve_parked(
        &self,
        bot_id: &str,
        turn_id: &str,
        response: &flow_like_types::bot_message::ApprovalResponse,
    ) -> Result<bool, String> {
        self.0
            .resolve_parked(bot_id, turn_id, response)
            .map(|released| released.is_some())
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Reply,
    Fail,
    Blank,
    /// Answers with the silent reply.
    Silent,
    /// Calls a tool that asks the owner through the turn's approval gate.
    Ask,
    /// Asks another bot through the turn's approval gate.
    AskBot,
    /// Calls a connector tool that core marked as asking, through the turn's approval gate.
    AskConnector,
}

pub(super) struct FakeRunner {
    pub(super) mode: Mutex<Mode>,
    pub(super) gate: Semaphore,
    pub(super) jobs: Mutex<Vec<TurnJob>>,
    pub(super) cancelled: Mutex<Vec<String>>,
    pub(super) running: Mutex<HashMap<(String, String), usize>>,
    pub(super) max_per_key: Mutex<usize>,
    pub(super) active: Mutex<usize>,
    pub(super) max_active: Mutex<usize>,
}

impl FakeRunner {
    pub(super) fn new(open: bool) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(Mode::Reply),
            gate: Semaphore::new(if open { 1_000_000 } else { 0 }),
            jobs: Mutex::default(),
            cancelled: Mutex::default(),
            running: Mutex::default(),
            max_per_key: Mutex::new(0),
            active: Mutex::new(0),
            max_active: Mutex::new(0),
        })
    }

    pub(super) fn started(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }

    pub(super) fn active(&self) -> usize {
        *self.active.lock().unwrap()
    }
}

#[async_trait]
impl TurnRunner for FakeRunner {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        let key = (job.bot_id.clone(), job.conversation_id.clone());
        {
            let mut running = self.running.lock().unwrap();
            let count = running.entry(key.clone()).or_default();
            *count += 1;
            let mut max = self.max_per_key.lock().unwrap();
            *max = (*max).max(*count);
            let mut active = self.active.lock().unwrap();
            *active += 1;
            let mut max_active = self.max_active.lock().unwrap();
            *max_active = (*max_active).max(*active);
        }
        self.jobs.lock().unwrap().push(job.clone());
        struct Leave<'a>(&'a FakeRunner, (String, String));
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                *self.0.running.lock().unwrap().get_mut(&self.1).unwrap() -= 1;
                *self.0.active.lock().unwrap() -= 1;
            }
        }
        let _leave = Leave(self, key);
        self.gate.acquire().await.unwrap().forget();
        let mode = *self.mode.lock().unwrap();
        match mode {
            Mode::Reply => Ok(format!("answer to: {}", job.prompt)),
            Mode::Blank => Ok("  ".into()),
            Mode::Silent => Ok(flow_like::flow::copilot::SILENT_REPLY.into()),
            Mode::Ask => {
                let args = json!({ "index": "notes" });
                match job
                    .gate
                    .clear("knowledge_drop", &args, None, "Drops the index 'notes'.")
                    .await
                {
                    Ok(()) => Ok("ran the drop".into()),
                    Err(_) => Ok("could not run the drop".into()),
                }
            }
            Mode::AskConnector => {
                let args = json!({
                    "connector": "c-docs",
                    "tool": "create",
                    "policy": "ask",
                    "connector_label": "Docs",
                });
                match job
                    .gate
                    .clear("connector_call", &args, None, "Calls create on Docs.")
                    .await
                {
                    Ok(()) => Ok("ran the connector call".into()),
                    Err(_) => Ok("could not run the connector call".into()),
                }
            }
            Mode::AskBot => {
                use flow_like::flow::copilot::bot_talk::{CrewMember, HeadlessPermission};
                let member = |id: &str, name: &str| CrewMember {
                    id: id.into(),
                    name: name.into(),
                    ..CrewMember::default()
                };
                let (pip, flux) = (member(&job.bot_id, "Pip"), member("flux-id", "Flux"));
                match job.gate.ask_bot(&pip, &flux, "what is new?").await {
                    HeadlessPermission::Allowed => Ok("asked Flux".into()),
                    _ => Ok("could not ask Flux".into()),
                }
            }
            Mode::Fail => Err(RunError::Failed {
                code: "BOT_TURN_FAILED".into(),
                message: "the model fell over".into(),
            }),
        }
    }

    fn cancel(&self, run_id: &str) {
        self.cancelled.lock().unwrap().push(run_id.to_owned());
    }
}

#[derive(Default)]
pub(super) struct Recorded(
    pub(super) Mutex<Vec<TurnInfo>>,
    pub(super) Mutex<Vec<super::notify::Alert>>,
);

impl TurnEvents for Recorded {
    fn turn(&self, info: &TurnInfo) {
        self.0.lock().unwrap().push(info.clone());
    }

    fn alert(&self, alert: &super::notify::Alert) {
        self.1.lock().unwrap().push(alert.clone());
    }
}

struct Fixture {
    _scratch: Scratch,
    store: Arc<LocalBotStore>,
    runner: Arc<FakeRunner>,
    events: Arc<Recorded>,
    host: BotHost,
    dispatcher: Option<super::host::Dispatcher>,
}

fn far_past() -> DateTime<FixedOffset> {
    (Utc::now() - Age::days(3650)).fixed_offset()
}

impl Fixture {
    fn new(open: bool, config: HostConfig) -> Self {
        let scratch = Scratch::new();
        let store = Arc::new(
            LocalBotStore::open(
                scratch.0.join("local_bots.db"),
                Arc::new(NoKey),
                Arc::new(Quiet),
            )
            .unwrap(),
        );
        let runner = FakeRunner::new(open);
        let events = Arc::new(Recorded::default());
        let (host, dispatcher) = BotHost::new(
            Arc::new(Direct(store.clone())),
            runner.clone(),
            events.clone(),
            config,
        );
        Self {
            _scratch: scratch,
            store,
            runner,
            events,
            host,
            dispatcher: Some(dispatcher),
        }
    }

    fn open() -> Self {
        Self::new(true, HostConfig::default())
    }

    fn gated() -> Self {
        Self::new(false, HostConfig::default())
    }

    fn start(&mut self) {
        tokio::spawn(self.dispatcher.take().unwrap().run());
    }

    fn bot(&self, name: &str) -> Bot {
        self.store
            .create_bot(
                serde_json::from_value(json!({ "name": name, "appearance": { "kind": "pip" } }))
                    .unwrap(),
            )
            .unwrap()
    }

    fn direct(&self, bot: &Bot) -> Conversation {
        self.conversation("direct", &[&bot.id])
    }

    fn conversation(&self, kind: &str, members: &[&str]) -> Conversation {
        self.store
            .create_conversation(
                serde_json::from_value(json!({
                    "kind": kind,
                    "title": (kind == "group").then_some("Team"),
                    "members": members,
                }))
                .unwrap(),
            )
            .unwrap()
    }

    fn send(
        &self,
        conversation: &str,
        sender: &str,
        id: &str,
        kind: &str,
        body: Value,
    ) -> InboxEntry {
        self.store
            .send(
                conversation,
                sender,
                serde_json::from_value(json!({
                    "message_id": id,
                    "kind": kind,
                    "envelope": { "v": 1, "enc": "none", "body": body },
                }))
                .unwrap(),
            )
            .unwrap()
    }

    fn say(&self, conversation: &str, id: &str, text: &str) -> InboxEntry {
        self.send(
            conversation,
            LOCAL_OWNER_ID,
            id,
            "message",
            json!({ "text": text }),
        )
    }

    fn newest(&self, conversation: &str) -> Vec<InboxEntry> {
        self.store
            .entries(
                conversation,
                &EntriesQuery {
                    limit: Some(WINDOW),
                    ..Default::default()
                },
            )
            .unwrap()
            .entries
    }

    fn waiting(&self, conversation: &Conversation, bot: &str) -> Option<Waiting> {
        waiting(
            conversation.kind,
            bot,
            &self.newest(&conversation.id),
            far_past(),
            false,
        )
    }

    fn answer_only_when_mentioned(&self, conversation: &Conversation, bots: &[&Bot]) {
        let limits = flow_like_types::bot_effective::BotRoomLimits {
            mentioned_only: true,
            ..Default::default()
        };
        for bot in bots {
            self.store
                .set_member_limits(&conversation.id, &bot.id, Some(limits.clone()))
                .unwrap();
        }
    }

    fn waiting_mentioned_only(&self, conversation: &Conversation, bot: &str) -> Option<Waiting> {
        waiting(
            conversation.kind,
            bot,
            &self.newest(&conversation.id),
            far_past(),
            true,
        )
    }

    fn messages_of(&self, conversation: &str, sender: &str) -> Vec<InboxEntry> {
        let mut own: Vec<_> = self
            .newest(conversation)
            .into_iter()
            .filter(|entry| entry.sender_id == sender)
            .collect();
        own.reverse();
        own
    }

    fn status_of(&self, bot: &str, trigger: &str) -> Option<TurnStatus> {
        let id = turn_id(bot, trigger);
        self.host
            .turns()
            .into_iter()
            .find(|turn| turn.id == id)
            .map(|turn| turn.status)
    }
}

pub(super) async fn until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}");
}

pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

pub(super) fn body(entry: &InboxEntry) -> &Value {
    &entry.envelope["body"]
}

#[test]
fn a_direct_message_of_the_person_waits_until_the_bot_has_spoken() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    assert_eq!(f.waiting(&chat, &bot.id), None);

    f.say(&chat.id, "m1", "hello");
    let waiting = f.waiting(&chat, &bot.id).unwrap();
    assert_eq!((waiting.message_id.as_str(), waiting.hops), ("m1", 0));

    f.send(&chat.id, &bot.id, "b1", "message", json!({ "text": "hi" }));
    assert_eq!(f.waiting(&chat, &bot.id), None);

    f.say(&chat.id, "m2", "again");
    assert_eq!(f.waiting(&chat, &bot.id).unwrap().message_id, "m2");
}

#[test]
fn a_persons_message_in_a_group_wakes_every_bot_but_only_a_mention_or_reply_addresses_it() {
    let f = Fixture::open();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);

    f.say(&group.id, "m1", "anyone?");
    for bot in [&ada, &bob] {
        let waiting = f.waiting(&group, &bot.id).unwrap();
        assert_eq!(waiting.message_id, "m1");
        assert!(!waiting.addressed, "{:?} reads along", bot.name);
    }

    f.send(
        &group.id,
        LOCAL_OWNER_ID,
        "m2",
        "message",
        json!({ "text": "ada?", "mentions": [ada.id] }),
    );
    assert!(f.waiting(&group, &ada.id).unwrap().addressed);
    assert!(!f.waiting(&group, &bob.id).unwrap().addressed);

    f.send(
        &group.id,
        &ada.id,
        "a1",
        "message",
        json!({ "text": "yes" }),
    );
    assert_eq!(f.waiting(&group, &ada.id), None);
    assert_eq!(
        f.waiting(&group, &bob.id).unwrap().message_id,
        "m2",
        "a bot's message wakes a bot only when it names it"
    );
    f.send(
        &group.id,
        LOCAL_OWNER_ID,
        "m3",
        "message",
        json!({ "text": "thanks", "reply_to": "a1" }),
    );
    assert!(f.waiting(&group, &ada.id).unwrap().addressed);
    assert!(!f.waiting(&group, &bob.id).unwrap().addressed);
}

#[test]
fn answer_only_when_mentioned_stops_reading_along_but_not_a_mention_or_a_direct_chat() {
    let f = Fixture::open();
    let ada = f.bot("Ada");
    let group = f.conversation("group", &[&ada.id]);
    f.say(&group.id, "m1", "anyone?");
    assert_eq!(f.waiting_mentioned_only(&group, &ada.id), None);
    f.send(
        &group.id,
        LOCAL_OWNER_ID,
        "m2",
        "message",
        json!({ "text": "ada?", "mentions": [ada.id] }),
    );
    assert_eq!(
        f.waiting_mentioned_only(&group, &ada.id).unwrap().message_id,
        "m2"
    );
    let chat = f.direct(&ada);
    f.say(&chat.id, "d1", "hello");
    assert_eq!(
        f.waiting_mentioned_only(&chat, &ada.id).unwrap().message_id,
        "d1"
    );
}

#[test]
fn a_persons_reaction_wakes_only_the_bot_whose_message_it_reacts_to() {
    let f = Fixture::open();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    let react = |id: &str, sender: &str, target: &str, op: &str| {
        f.send(
            &group.id,
            sender,
            id,
            "reaction",
            json!({ "target": target, "emoji": "👎", "op": op }),
        );
    };
    f.send(&group.id, &ada.id, "a1", "message", json!({ "text": "my plan" }));
    f.say(&group.id, "m1", "hm");
    f.send(&group.id, &ada.id, "a2", "message", json!({ "text": "ok" }));

    react("r1", LOCAL_OWNER_ID, "a1", "add");
    let waiting = f.waiting_mentioned_only(&group, &ada.id).unwrap();
    assert_eq!(waiting.message_id, "r1");
    assert!(waiting.addressed && waiting.hops == 0);
    assert_eq!(f.waiting_mentioned_only(&group, &bob.id), None, "other bots are not woken");

    react("r2", LOCAL_OWNER_ID, "m1", "add");
    assert_eq!(
        f.waiting_mentioned_only(&group, &ada.id).unwrap().message_id,
        "r1",
        "a reaction to a person's message wakes nobody"
    );
    react("r3", &bob.id, "a1", "add");
    assert_eq!(
        f.waiting_mentioned_only(&group, &ada.id).unwrap().message_id,
        "r1",
        "a bot's reaction wakes nobody"
    );
    react("r4", LOCAL_OWNER_ID, "a1", "remove");
    assert_eq!(f.waiting_mentioned_only(&group, &ada.id), None, "taken back before the turn");

    react("r5", LOCAL_OWNER_ID, "a1", "add");
    f.send(&group.id, &ada.id, "a3", "message", json!({ "text": "sorry" }));
    assert_eq!(
        f.waiting_mentioned_only(&group, &ada.id),
        None,
        "only reactions newer than the bot's last message count"
    );
}

#[test]
fn a_bot_never_waits_for_its_own_message_and_hops_are_capped() {
    let f = Fixture::open();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    let mention = |hops: Option<u32>| json!({ "text": "ping", "mentions": [bob.id], "hops": hops });

    f.send(&group.id, &ada.id, "a1", "message", mention(Some(3)));
    assert_eq!(f.waiting(&group, &ada.id), None);
    let waiting = f.waiting(&group, &bob.id).unwrap();
    assert_eq!((waiting.message_id.as_str(), waiting.hops), ("a1", 3));

    f.send(
        &group.id,
        &bob.id,
        "b1",
        "message",
        json!({ "text": "pong" }),
    );
    f.send(&group.id, &ada.id, "a2", "message", mention(Some(4)));
    assert_eq!(f.waiting(&group, &bob.id), None);

    f.send(&group.id, &ada.id, "a3", "message", mention(None));
    let waiting = f.waiting(&group, &bob.id).unwrap();
    assert_eq!((waiting.message_id.as_str(), waiting.hops), ("a3", 1));
}

#[test]
fn topics_tombstones_edits_and_old_messages_are_no_trigger() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);

    f.say(&chat.id, "m1", "hello");
    f.send(
        &chat.id,
        LOCAL_OWNER_ID,
        "m1d",
        "delete",
        json!({ "target": "m1" }),
    );
    assert_eq!(f.waiting(&chat, &bot.id), None);

    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "topic", json!({}));
    assert_eq!(f.waiting(&chat, &bot.id), None);

    f.say(&chat.id, "m2", "fresh");
    f.send(&chat.id, &bot.id, "b1", "message", json!({ "text": "ok" }));
    f.send(
        &chat.id,
        LOCAL_OWNER_ID,
        "e1",
        "edit",
        json!({ "text": "fresher", "target": "m2" }),
    );
    assert_eq!(f.waiting(&chat, &bot.id), None);

    f.say(&chat.id, "m3", "late");
    let entries = f.newest(&chat.id);
    let tomorrow = (Utc::now() + Age::days(1)).fixed_offset();
    assert_eq!(waiting(chat.kind, &bot.id, &entries, tomorrow, false), None);
    assert_eq!(chat.kind, ConversationKind::Direct);
}

#[test]
fn a_host_reply_answers_up_to_the_message_it_replies_to() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.say(&chat.id, "m1", "first");
    f.say(&chat.id, "m2", "second, while the turn runs");
    f.send(
        &chat.id,
        &bot.id,
        &reply_message_id(&bot.id, "m1"),
        "message",
        json!({ "text": "re", "reply_to": "m1", "hops": 1 }),
    );
    assert_eq!(f.waiting(&chat, &bot.id).unwrap().message_id, "m2");

    f.send(
        &chat.id,
        &bot.id,
        "b9",
        "message",
        json!({ "text": "unprompted", "reply_to": "m1" }),
    );
    assert_eq!(f.waiting(&chat, &bot.id), None);
}

#[tokio::test]
async fn a_message_gets_one_reply_posted_as_the_bot() {
    let mut f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    settle().await;
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));

    let id = reply_message_id(&bot.id, "m1");
    until("the reply", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Done)
    })
    .await;
    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].message_id, id);
    assert_eq!(body(&replies[0])["text"], "answer to: hello");
    assert_eq!(body(&replies[0])["reply_to"], "m1");
    assert_eq!(body(&replies[0])["hops"], 1);
    assert!(body(&replies[0]).get("error").is_none());

    let job = f.runner.jobs.lock().unwrap()[0].clone();
    assert_eq!(job.run_id, turn_id(&bot.id, "m1"));
    assert_eq!(job.prompt, "hello");
    assert!(job.room_updates.is_none());

    settle().await;
    assert_eq!(f.runner.started(), 1);
    let states: Vec<_> = f
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|t| t.status)
        .collect();
    assert_eq!(
        states,
        [TurnStatus::Queued, TurnStatus::Running, TurnStatus::Done]
    );
}

#[tokio::test]
async fn an_addressed_group_turn_keeps_the_actual_requester_and_room() {
    let mut f = Fixture::gated();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    f.answer_only_when_mentioned(&group, &[&ada, &bob]);
    f.send(
        &group.id,
        &bob.id,
        "b1",
        "message",
        json!({ "text": "Please check the manual", "mentions": [ada.id], "hops": 1 }),
    );
    f.start();
    until("the group turn", || f.runner.started() == 1).await;
    let jobs = f.runner.jobs.lock().unwrap();
    let requester = jobs[0].requester.as_ref().expect("group requester");
    assert_eq!(requester.requester_id, bob.id);
    assert_eq!(requester.requester_name.as_deref(), Some("Bob"));
    assert_eq!(requester.owner_id, LOCAL_OWNER_ID);
    assert_eq!(requester.audience, flow_like_types::bot_turn::TurnAudience::Room);
    assert_eq!(requester.room.as_deref(), Some("Team"));
    assert!(requester.addressed);
}

#[tokio::test]
async fn the_startup_scan_answers_what_waited_and_leaves_the_rest() {
    let mut f = Fixture::open();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let waiting_chat = f.direct(&ada);
    let answered_chat = f.direct(&bob);
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    f.answer_only_when_mentioned(&group, &[&ada, &bob]);
    f.say(&waiting_chat.id, "w1", "are you there");
    f.say(&answered_chat.id, "a1", "hi");
    f.send(
        &answered_chat.id,
        &bob.id,
        "a2",
        "message",
        json!({ "text": "hello" }),
    );
    f.say(&group.id, "g1", "not addressed to anyone");

    f.start();
    until("the startup reply", || {
        f.status_of(&ada.id, "w1") == Some(TurnStatus::Done)
    })
    .await;
    settle().await;
    assert_eq!(f.runner.started(), 1);
    assert_eq!(f.messages_of(&answered_chat.id, &bob.id).len(), 1);
    assert_eq!(f.messages_of(&group.id, &ada.id).len(), 0);
}

#[tokio::test]
async fn messages_that_arrive_during_a_turn_are_answered_by_one_later_turn() {
    let mut f = Fixture::gated();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "one");
    f.host.poke(Some(&chat.id));
    until("the first run", || f.runner.started() == 1).await;

    f.say(&chat.id, "m2", "two");
    f.host.poke(Some(&chat.id));
    f.say(&chat.id, "m3", "three");
    f.host.poke(Some(&chat.id));
    settle().await;
    assert_eq!(f.runner.started(), 1);
    assert_eq!(f.host.turns().len(), 1);

    f.runner.gate.add_permits(1);
    until("the second run", || f.runner.started() == 2).await;
    f.runner.gate.add_permits(1);
    until("both replies", || {
        f.status_of(&bot.id, "m3") == Some(TurnStatus::Done)
    })
    .await;
    settle().await;

    assert_eq!(f.runner.started(), 2);
    assert_eq!(f.runner.jobs.lock().unwrap()[1].prompt, "three");
    assert_eq!(*f.runner.max_per_key.lock().unwrap(), 1);
    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(
        replies
            .iter()
            .map(|r| r.message_id.clone())
            .collect::<Vec<_>>(),
        [
            reply_message_id(&bot.id, "m1"),
            reply_message_id(&bot.id, "m3")
        ]
    );
}

#[tokio::test]
async fn a_running_room_turn_reads_new_participant_messages_once() {
    let mut f = Fixture::gated();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let room = f.conversation("group", &[&ada.id, &bob.id]);
    f.answer_only_when_mentioned(&room, &[&ada, &bob]);
    f.start();
    f.send(
        &room.id,
        LOCAL_OWNER_ID,
        "introduce",
        "message",
        json!({ "text": "Ada, introduce yourself", "mentions": [ada.id] }),
    );
    f.host.poke(Some(&room.id));
    until("the room turn", || f.runner.started() == 1).await;
    let updates = f.runner.jobs.lock().unwrap()[0]
        .room_updates
        .clone()
        .expect("room turns read new activity");
    assert!(updates.drain().await.is_empty());

    f.send(
        &room.id,
        &bob.id,
        "bob-introduction",
        "message",
        json!({ "text": "I'm Bob and I already introduced myself." }),
    );
    f.say(&room.id, "shorter", "Keep your introduction brief.");
    let fresh = updates.drain().await.join("\n");
    assert!(fresh.contains("Bob"));
    assert!(fresh.contains("I already introduced myself."));
    assert!(fresh.contains("Keep your introduction brief."));
    assert!(!fresh.contains("Ada, introduce yourself"));
    assert!(updates.drain().await.is_empty());

    f.store.remove_member(&room.id, &ada.id).unwrap();
    f.say(&room.id, "after-leaving", "This is for the remaining members.");
    assert!(updates.drain().await.is_empty());
    f.host.poke(Some(&room.id));
    until("the removed bot's cancellation", || {
        f.status_of(&ada.id, "introduce") == Some(TurnStatus::Cancelled)
    })
    .await;
}

#[tokio::test]
async fn turns_queued_behind_a_slot_batch_into_the_newest_message() {
    let mut f = Fixture::new(
        false,
        HostConfig {
            slots: 1,
            ..HostConfig::default()
        },
    );
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let (first, second) = (f.direct(&ada), f.direct(&bob));
    f.start();
    f.say(&first.id, "m1", "holds the slot");
    f.host.poke(Some(&first.id));
    until("the slot taken", || f.runner.started() == 1).await;

    f.say(&second.id, "q1", "queued one");
    f.host.poke(Some(&second.id));
    until("the turn queued", || {
        f.status_of(&bob.id, "q1") == Some(TurnStatus::Queued)
    })
    .await;
    f.say(&second.id, "q2", "queued two");
    f.host.poke(Some(&second.id));
    settle().await;
    let queued: Vec<_> = f
        .host
        .turns()
        .into_iter()
        .filter(|turn| turn.status == TurnStatus::Queued)
        .collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].trigger_message_id, "q2");
    assert_eq!(queued[0].batched, 1);

    f.runner.gate.add_permits(2);
    until("the batched reply", || {
        f.status_of(&bob.id, "q2") == Some(TurnStatus::Done)
    })
    .await;
    assert_eq!(f.runner.started(), 2);
    assert_eq!(f.runner.jobs.lock().unwrap()[1].prompt, "queued two");
    assert_eq!(f.messages_of(&second.id, &bob.id).len(), 1);
}

#[tokio::test]
async fn no_more_turns_run_at_once_than_there_are_slots() {
    let mut f = Fixture::new(
        false,
        HostConfig {
            slots: 2,
            ..HostConfig::default()
        },
    );
    f.start();
    let chats: Vec<_> = (0..4)
        .map(|n| {
            let bot = f.bot(&format!("Bot {n}"));
            let chat = f.direct(&bot);
            f.say(&chat.id, &format!("m{n}"), "hello");
            f.host.poke(Some(&chat.id));
            (bot, chat)
        })
        .collect();
    until("two runs", || f.runner.active() == 2).await;
    settle().await;
    assert_eq!(f.runner.active(), 2);
    let queued = f
        .host
        .turns()
        .iter()
        .filter(|turn| turn.status == TurnStatus::Queued)
        .count();
    assert_eq!(queued, 2);

    f.runner.gate.add_permits(4);
    until("every reply", || {
        chats
            .iter()
            .all(|(bot, chat)| f.messages_of(&chat.id, &bot.id).len() == 1)
    })
    .await;
    assert_eq!(*f.runner.max_active.lock().unwrap(), 2);
}

#[test]
fn sending_the_same_reply_twice_stores_it_once() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.say(&chat.id, "m1", "hello");
    let id = reply_message_id(&bot.id, "m1");
    let first = f.send(&chat.id, &bot.id, &id, "message", json!({ "text": "re" }));
    let again = f.send(&chat.id, &bot.id, &id, "message", json!({ "text": "re" }));
    assert_eq!(first.id, again.id);
    assert_eq!(f.messages_of(&chat.id, &bot.id).len(), 1);
}

#[tokio::test]
async fn a_reply_that_exists_is_not_asked_for_again_after_a_restart() {
    let mut f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.say(&chat.id, "m1", "hello");
    f.send(
        &chat.id,
        &bot.id,
        &reply_message_id(&bot.id, "m1"),
        "message",
        json!({ "text": "re", "reply_to": "m1", "hops": 1 }),
    );
    f.start();
    settle().await;
    assert_eq!(f.runner.started(), 0);
    assert_eq!(f.messages_of(&chat.id, &bot.id).len(), 1);
}

#[tokio::test]
async fn a_failed_turn_posts_the_reason_with_its_error_code() {
    let mut f = Fixture::open();
    *f.runner.mode.lock().unwrap() = Mode::Fail;
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));
    until("the failure", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Failed)
    })
    .await;

    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].message_id, failure_message_id(&bot.id, "m1"));
    assert_eq!(body(&replies[0])["error"]["code"], "BOT_TURN_FAILED");
    assert_eq!(body(&replies[0])["reply_to"], "m1");
    assert!(
        body(&replies[0])["text"]
            .as_str()
            .unwrap()
            .contains("the model fell over")
    );
    settle().await;
    assert_eq!(f.runner.started(), 1);
}

#[tokio::test]
async fn an_empty_answer_counts_as_a_failure() {
    let mut f = Fixture::open();
    *f.runner.mode.lock().unwrap() = Mode::Blank;
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));
    until("the failure", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Failed)
    })
    .await;
    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(body(&replies[0])["error"]["code"], "BOT_TURN_MODEL_ERROR");
}

#[tokio::test]
async fn a_turn_past_its_deadline_is_cancelled_and_reported() {
    let mut f = Fixture::new(
        false,
        HostConfig {
            deadline: Duration::from_millis(60),
            ..HostConfig::default()
        },
    );
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));
    until("the timeout", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Failed)
    })
    .await;
    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(body(&replies[0])["error"]["code"], CODE_TIMEOUT);
    assert_eq!(
        *f.runner.cancelled.lock().unwrap(),
        [turn_id(&bot.id, "m1")]
    );
}

#[tokio::test]
async fn cancelling_a_turn_posts_nothing_and_does_not_start_it_again() {
    let mut f = Fixture::gated();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));
    until("the run", || f.runner.started() == 1).await;

    let id = turn_id(&bot.id, "m1");
    assert!(f.host.cancel(&id));
    until("the cancellation", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Cancelled)
    })
    .await;
    f.host.poke(Some(&chat.id));
    settle().await;
    assert_eq!(f.runner.started(), 1);
    assert_eq!(*f.runner.cancelled.lock().unwrap(), [id.clone()]);
    assert!(f.messages_of(&chat.id, &bot.id).is_empty());
    let turn = f.host.turns().into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(turn.error_code.as_deref(), Some(CODE_CANCELLED));
    assert!(!f.host.cancel(&id));
}

#[tokio::test]
async fn a_turn_whose_message_was_deleted_meanwhile_is_cancelled() {
    let mut f = Fixture::gated();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    f.say(&chat.id, "m1", "hello");
    f.host.poke(Some(&chat.id));
    until("the run", || f.runner.started() == 1).await;

    f.send(
        &chat.id,
        LOCAL_OWNER_ID,
        "m1d",
        "delete",
        json!({ "target": "m1" }),
    );
    f.host.poke(Some(&chat.id));
    until("the cancellation", || {
        f.status_of(&bot.id, "m1") == Some(TurnStatus::Cancelled)
    })
    .await;
    let turn = f.host.turns().into_iter().next().unwrap();
    assert_eq!(turn.error_code.as_deref(), Some(CODE_STORE_CHANGED));
    assert!(f.messages_of(&chat.id, &bot.id).is_empty());
}

#[tokio::test]
async fn a_turn_whose_bot_left_the_conversation_is_cancelled() {
    let mut f = Fixture::gated();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    f.answer_only_when_mentioned(&group, &[&ada, &bob]);
    f.start();
    f.send(
        &group.id,
        LOCAL_OWNER_ID,
        "m1",
        "message",
        json!({ "text": "ada?", "mentions": [ada.id] }),
    );
    f.host.poke(Some(&group.id));
    until("the run", || f.runner.started() == 1).await;

    f.store.remove_member(&group.id, &ada.id).unwrap();
    f.host.poke(Some(&group.id));
    until("the cancellation", || {
        f.status_of(&ada.id, "m1") == Some(TurnStatus::Cancelled)
    })
    .await;
    assert!(f.messages_of(&group.id, &ada.id).is_empty());
}

mod grants {
    use super::*;
    use crate::bot_host::approvals::answer_local;
    use flow_like_types::tool_grant::{ApprovalDecision, ToolGrantKey};

    fn asking() -> Fixture {
        let mut f = Fixture::open();
        *f.runner.mode.lock().unwrap() = Mode::Ask;
        f.start();
        f
    }

    fn asking_a_connector() -> Fixture {
        let mut f = Fixture::open();
        *f.runner.mode.lock().unwrap() = Mode::AskConnector;
        f.start();
        f
    }

    #[tokio::test]
    async fn a_connector_call_that_asks_parks_resumes_on_approval_and_runs() {
        let f = asking_a_connector();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        let approval = &body(&card)["approval"];
        assert_eq!(approval["tool"], "connector_call");
        assert_eq!(approval["scope"], "connector:c-docs:create");
        assert!(f.runner.cancelled.lock().unwrap().contains(&turn_id(&bot.id, "m1")));
        settle().await;
        assert!(replies(&f, &bot, &chat).is_empty());
        answer(&f, &bot, "m1", &card, ApprovalDecision::Always).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["ran the connector call"]);
        let grants = f.store.tool_grants(&bot.id).unwrap();
        assert_eq!(
            (grants[0].tool.as_str(), grants[0].scope.as_str()),
            ("connector_call", "connector:c-docs:create")
        );
        f.say(&chat.id, "m2", "and again");
        f.host.poke(Some(&chat.id));
        until("the second reply", || replies(&f, &bot, &chat).len() == 2).await;
    }

    #[tokio::test]
    async fn a_refused_connector_call_resumes_with_the_refusal_and_runs_nothing() {
        let f = asking_a_connector();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Deny).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["could not run the connector call"]);
        assert!(f.store.tool_grants(&bot.id).unwrap().is_empty());
    }

    fn asking_a_bot() -> Fixture {
        let mut f = Fixture::open();
        *f.runner.mode.lock().unwrap() = Mode::AskBot;
        f.start();
        f
    }

    #[tokio::test]
    async fn a_headless_ask_of_another_bot_parks_with_the_ask_card() {
        let f = asking_a_bot();
        let bot = f.bot("Pip");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        let approval = &body(&card)["approval"];
        assert_eq!(approval["tool"], "ask_bot");
        assert_eq!(approval["scope"], "bot:flux-id");
        assert_eq!(approval["summary"], "Pip wants to ask Flux: what is new?");
        assert!(f.runner.cancelled.lock().unwrap().contains(&turn_id(&bot.id, "m1")));
    }

    #[tokio::test]
    async fn always_on_an_ask_card_lets_the_next_ask_run_without_a_card() {
        let f = asking_a_bot();
        let bot = f.bot("Pip");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Always).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        f.say(&chat.id, "m2", "ask again");
        f.host.poke(Some(&chat.id));
        until("the second reply", || replies(&f, &bot, &chat).len() == 2).await;
        assert_eq!(replies(&f, &bot, &chat), ["asked Flux", "asked Flux"]);
    }

    #[tokio::test]
    async fn a_trigger_started_turn_parks_its_first_ask_resumes_and_asks() {
        let f = asking_a_bot();
        let bot = f.bot("Pip");
        let chat = f.direct(&bot);
        let trigger = flow_like_types::inbox::TriggerBody {
            source: flow_like_types::inbox::TriggerSource::Webhook,
            ref_id: "hook".into(),
            title: "Hook".into(),
            text: "Check with Flux".into(),
            payload: None,
            payload_handle: None,
            bot_id: None,
        };
        f.store.post_trigger(&bot.id, "tk-1", &trigger).unwrap();
        f.host.poke(Some(&chat.id));
        until("the turn parks", || {
            f.status_of(&bot.id, "tk-1") == Some(TurnStatus::Waiting)
        })
        .await;
        let card = f
            .messages_of(&chat.id, &bot.id)
            .into_iter()
            .find(|entry| body(entry).get("approval").is_some())
            .expect("an approval card");
        assert_eq!(body(&card)["approval"]["tool"], "ask_bot");
        assert_eq!(body(&card)["approval"]["scope"], "bot:flux-id");
        answer(&f, &bot, "tk-1", &card, ApprovalDecision::Always).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["asked Flux"]);
    }

    #[tokio::test]
    async fn deny_on_an_ask_card_refuses_the_ask() {
        let f = asking_a_bot();
        let bot = f.bot("Pip");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Deny).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["could not ask Flux"]);
    }

    fn dyn_store(f: &Fixture) -> Arc<dyn HostStore> {
        Arc::new(Direct(f.store.clone()))
    }

    fn drop_key() -> ToolGrantKey {
        ToolGrantKey {
            tool: "knowledge_drop".into(),
            scope: "table:notes".into(),
        }
    }

    async fn parked(f: &Fixture, bot: &Bot, chat: &Conversation, trigger: &str) -> InboxEntry {
        f.say(&chat.id, trigger, "drop my notes index");
        f.host.poke(Some(&chat.id));
        until("the turn parks", || {
            f.status_of(&bot.id, trigger) == Some(TurnStatus::Waiting)
        })
        .await;
        f.messages_of(&chat.id, &bot.id)
            .into_iter()
            .find(|entry| body(entry).get("approval").is_some())
            .expect("an approval card")
    }

    async fn answer(f: &Fixture, bot: &Bot, trigger: &str, card: &InboxEntry, d: ApprovalDecision) {
        let card_id = card.message_id.clone();
        answer_local(&dyn_store(f), &bot.id, &turn_id(&bot.id, trigger), &card_id, d)
            .await
            .unwrap();
        f.host.poke(None);
    }

    fn replies(f: &Fixture, bot: &Bot, chat: &Conversation) -> Vec<String> {
        f.messages_of(&chat.id, &bot.id)
            .iter()
            .filter(|entry| entry.message_id.starts_with("re-"))
            .map(|entry| body(entry)["text"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_headless_turn_with_a_grant_runs_without_asking() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        f.store.grant_tool(&bot.id, &drop_key()).unwrap();
        f.say(&chat.id, "m1", "drop my notes index");
        until("the reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["ran the drop"]);
        assert!(f.store.parked_turns().unwrap().is_empty());
    }

    #[tokio::test]
    async fn without_a_grant_the_turn_parks_posts_a_card_and_stops_the_run() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        let approval = &body(&card)["approval"];
        assert_eq!(approval["tool"], "knowledge_drop");
        assert_eq!(approval["scope"], "table:notes");
        assert_eq!(approval["turn_id"], turn_id(&bot.id, "m1"));
        assert!(f.runner.cancelled.lock().unwrap().contains(&turn_id(&bot.id, "m1")));
        assert_eq!(f.store.parked_turns().unwrap().len(), 1);
        settle().await;
        assert!(replies(&f, &bot, &chat).is_empty());
        assert_eq!(f.runner.started(), 1);
        assert_eq!(f.waiting(&chat, &bot.id), None);
    }

    #[tokio::test]
    async fn once_runs_the_resumed_call_and_asks_again_next_time() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Once).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["ran the drop"]);
        let resumed = f.runner.jobs.lock().unwrap().last().unwrap().prompt.clone();
        assert!(resumed.contains("approved: knowledge_drop"), "{resumed}");
        assert!(f.store.tool_grants(&bot.id).unwrap().is_empty());
        assert!(f.store.tool_keys(&bot.id).unwrap().is_empty());
        let _ = parked(&f, &bot, &chat, "m2").await;
    }

    #[tokio::test]
    async fn always_stores_a_grant_the_next_turn_uses() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Always).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        let grants = f.store.tool_grants(&bot.id).unwrap();
        assert_eq!(
            (grants[0].tool.as_str(), grants[0].scope.as_str()),
            ("knowledge_drop", "table:notes")
        );
        f.say(&chat.id, "m2", "and again");
        f.host.poke(Some(&chat.id));
        until("the second reply", || replies(&f, &bot, &chat).len() == 2).await;
        assert_eq!(replies(&f, &bot, &chat), ["ran the drop", "ran the drop"]);
    }

    #[tokio::test]
    async fn deny_resumes_with_the_refusal_told_to_the_model_and_runs_nothing() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        answer(&f, &bot, "m1", &card, ApprovalDecision::Deny).await;
        until("the resumed reply", || !replies(&f, &bot, &chat).is_empty()).await;
        assert_eq!(replies(&f, &bot, &chat), ["could not run the drop"]);
        let resumed = f.runner.jobs.lock().unwrap().last().unwrap().prompt.clone();
        assert!(resumed.contains("refused: knowledge_drop"), "{resumed}");
        assert!(f.store.tool_grants(&bot.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_answer_for_another_card_or_turn_is_refused() {
        let f = asking();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        let card = parked(&f, &bot, &chat, "m1").await;
        let store = dyn_store(&f);
        let turn = turn_id(&bot.id, "m1");
        assert!(answer_local(&store, &bot.id, &turn, "ap-other", ApprovalDecision::Always).await.is_err());
        assert!(answer_local(&store, &bot.id, "bt_other", &card.message_id, ApprovalDecision::Always).await.is_err());
        assert!(f.store.tool_grants(&bot.id).unwrap().is_empty());
        assert_eq!(f.store.parked_turns().unwrap().len(), 1);
    }
}

async fn reply_to(f: &mut Fixture, bot: &Bot, chat: &Conversation, id: &str) {
    f.host.poke(Some(&chat.id));
    until("the reply", || {
        f.status_of(&bot.id, id) == Some(TurnStatus::Done)
    })
    .await;
}

fn trigger_body() -> Value {
    json!({ "source": "schedule", "ref_id": "r1", "title": "Daily", "text": "Summarize the news" })
}

#[tokio::test]
async fn a_reply_to_a_persons_message_raises_no_alert() {
    let mut f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    settle().await;
    f.say(&chat.id, "m1", "hello");
    reply_to(&mut f, &bot, &chat, "m1").await;
    settle().await;
    assert!(f.events.1.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_reply_to_a_trigger_alerts_under_the_bots_name_once() {
    let mut f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.start();
    settle().await;
    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "trigger", trigger_body());
    reply_to(&mut f, &bot, &chat, "t1").await;
    settle().await;
    let alerts = f.events.1.lock().unwrap().clone();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].title, "Ada");
    assert_eq!(alerts[0].link, format!("/bots?bot={}", bot.id));
    assert_eq!(alerts[0].key, reply_message_id(&bot.id, "t1"));
    assert_eq!(alerts[0].body, "answer to: Summarize the news");
}

#[tokio::test]
async fn a_muted_bot_raises_no_alert_for_a_trigger() {
    let mut f = Fixture::open();
    let bot = f.bot("Ada");
    f.store
        .update_bot(
            &bot.id,
            serde_json::from_value(json!({
                "appearance": { "kind": "pip", "mute_notifications": true }
            }))
            .unwrap(),
        )
        .unwrap();
    let chat = f.direct(&bot);
    f.start();
    settle().await;
    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "trigger", trigger_body());
    reply_to(&mut f, &bot, &chat, "t1").await;
    settle().await;
    assert!(f.events.1.lock().unwrap().is_empty());
}

#[test]
fn only_a_message_of_the_person_is_asked_for() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.say(&chat.id, "m1", "hello");
    assert!(!f.waiting(&chat, &bot.id).unwrap().proactive);
    f.send(&chat.id, &bot.id, "b1", "message", json!({ "text": "hi" }));
    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "trigger", trigger_body());
    assert!(f.waiting(&chat, &bot.id).unwrap().proactive);
}

fn scheduling_bot(f: &Fixture, name: &str) -> Bot {
    f.store
        .create_bot(
            serde_json::from_value(json!({
                "name": name,
                "appearance": { "kind": "pip" },
                "capabilities": ["scheduling"],
            }))
            .unwrap(),
        )
        .unwrap()
}

#[test]
fn a_trigger_of_the_person_waits_like_a_message_with_no_hops() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "trigger", trigger_body());
    let waiting = f.waiting(&chat, &bot.id).unwrap();
    assert_eq!((waiting.message_id.as_str(), waiting.hops), ("t1", 0));
    f.send(&chat.id, &bot.id, "b1", "message", json!({ "text": "done" }));
    assert_eq!(f.waiting(&chat, &bot.id), None);
}

#[test]
fn a_trigger_in_a_group_wakes_only_the_bot_it_names() {
    let f = Fixture::open();
    let (ada, bob) = (f.bot("Ada"), f.bot("Bob"));
    let group = f.conversation("group", &[&ada.id, &bob.id]);
    f.send(&group.id, LOCAL_OWNER_ID, "t1", "trigger", trigger_body());
    assert_eq!(f.waiting(&group, &ada.id), None);
    let mut named = trigger_body();
    named["bot_id"] = json!(ada.id);
    f.send(&group.id, LOCAL_OWNER_ID, "t2", "trigger", named);
    assert_eq!(f.waiting(&group, &ada.id).unwrap().message_id, "t2");
    assert_eq!(f.waiting(&group, &bob.id), None);
}

#[test]
fn the_prompt_of_a_trigger_is_its_text_and_a_payload_is_marked() {
    let f = Fixture::open();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    let mut body = trigger_body();
    body["payload"] = json!({ "order": 7 });
    f.send(&chat.id, LOCAL_OWNER_ID, "t1", "trigger", body);
    let mut entries = f.newest(&chat.id);
    entries.reverse();
    let names = std::collections::HashMap::new();
    let transcript =
        super::transcript::transcript(&bot.id, chat.kind, "t1", &entries, &names, flow_like_local_bots::LOCAL_OWNER_ID).unwrap();
    assert_eq!(
        transcript.prompt,
        "Summarize the news\n\nPayload (schedule): {\"order\":7}"
    );
}

#[tokio::test]
async fn a_due_task_is_answered_once_and_a_week_of_missed_runs_fire_once() {
    let mut f = Fixture::open();
    let bot = scheduling_bot(&f, "Clock");
    let chat = f.direct(&bot);
    let task = f
        .store
        .create_task(
            &bot.id,
            serde_json::from_value(json!({
                "kind": "cron", "cron": "0 * * * *", "tz": "UTC", "instruction": "Hourly sweep",
            }))
            .unwrap(),
            flow_like_types::bot_task::TaskCreator::Owner,
        )
        .unwrap();
    f.start();
    settle().await;
    let next = chrono::DateTime::parse_from_rfc3339(task.next_run_at.as_deref().unwrap())
        .unwrap()
        .timestamp_micros();
    let a_week_later = next + Age::days(7).num_microseconds().unwrap();
    assert_eq!(f.store.fire_due_tasks(a_week_later).unwrap().len(), 1);
    assert!(f.store.fire_due_tasks(a_week_later).unwrap().is_empty());

    let triggers: Vec<_> = f
        .newest(&chat.id)
        .into_iter()
        .filter(|entry| entry.kind == InboxEntryKind::Trigger)
        .collect();
    assert_eq!(triggers.len(), 1);
    let trigger_id = triggers[0].message_id.clone();
    reply_to(&mut f, &bot, &chat, &trigger_id).await;
    let replies = f.messages_of(&chat.id, &bot.id);
    assert_eq!(replies.len(), 1);
    assert_eq!(body(&replies[0])["text"], "answer to: Hourly sweep");
    settle().await;
    assert_eq!(f.runner.started(), 1);
}

mod interactive {
    use super::*;
    use crate::bot_host::{
        interactive::{self, InteractiveArgs, InteractiveRun},
        rule::{Hold, waiting_held},
    };

    fn marked(f: &Fixture, conversation: &str, id: &str, text: &str) -> InboxEntry {
        f.send(
            conversation,
            LOCAL_OWNER_ID,
            id,
            "message",
            json!({ "text": text, "interactive": { "client_run_id": "run_1" } }),
        )
    }

    fn hold(live: bool, minutes_from_now: i64) -> Hold {
        Hold {
            now: (Utc::now() + Age::minutes(minutes_from_now)).fixed_offset(),
            grace: Age::seconds(flow_like_types::bot_turn::INTERACTIVE_GRACE_SECS),
            live,
        }
    }

    fn waiting_with(f: &Fixture, conversation: &Conversation, bot: &str, hold: Hold) -> Option<Waiting> {
        waiting_held(
            conversation.kind,
            bot,
            &f.newest(&conversation.id),
            far_past(),
            Some(hold),
            false,
        )
    }

    #[test]
    fn a_marked_message_is_left_to_its_chat_while_it_runs_and_inside_the_grace() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        marked(&f, &chat.id, "m1", "plan my week");

        assert_eq!(waiting_with(&f, &chat, &bot.id, hold(true, 10)), None);
        assert_eq!(waiting_with(&f, &chat, &bot.id, hold(false, 0)), None);
        assert_eq!(waiting_with(&f, &chat, &bot.id, hold(false, 1)), None);
    }

    #[test]
    fn the_host_takes_a_marked_message_over_once_the_chat_is_gone_and_the_grace_passed() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        marked(&f, &chat.id, "m1", "plan my week");

        let taken = waiting_with(&f, &chat, &bot.id, hold(false, 3)).expect("picked up");
        assert_eq!(taken.message_id, "m1");
    }

    #[test]
    fn an_unmarked_message_is_never_held() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        f.say(&chat.id, "m1", "hello");
        assert_eq!(
            waiting_with(&f, &chat, &bot.id, hold(true, 0)).map(|w| w.message_id),
            Some("m1".into())
        );
    }

    #[test]
    fn a_registered_run_holds_its_conversation_until_the_guard_drops() {
        let guard = interactive::begin(None, "bot_hold", "conv_hold");
        assert!(interactive::is_live("bot_hold", "conv_hold"));
        assert!(interactive::hold_now("bot_hold", "conv_hold").live);
        assert!(!interactive::is_live("bot_hold", "other"));
        drop(guard);
        assert!(!interactive::is_live("bot_hold", "conv_hold"));
    }

    async fn run_for(f: &Fixture, bot: &Bot, chat: &Conversation) -> InteractiveRun {
        InteractiveRun::open(
            Arc::new(Direct(f.store.clone())),
            None,
            &bot.id,
            InteractiveArgs {
                conversation_id: chat.id.clone(),
                trigger_message_id: "m1".into(),
                client_run_id: "run_1".into(),
            },
        )
        .await
        .expect("a local conversation opens")
    }

    #[tokio::test]
    async fn the_reply_is_stored_in_the_local_store_as_the_bot_under_the_hosts_id() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        marked(&f, &chat.id, "m1", "what day is it");
        let run = run_for(&f, &bot, &chat).await;

        assert!(run.store_reply("It is Tuesday.", None).await.unwrap());

        let replies = f.messages_of(&chat.id, &bot.id);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].message_id, reply_message_id(&bot.id, "m1"));
        assert_eq!(replies[0].envelope["body"]["text"], json!("It is Tuesday."));
        assert_eq!(replies[0].envelope["body"]["reply_to"], json!("m1"));
        assert_eq!(f.waiting(&chat, &bot.id), None, "nothing is left for the host");
    }

    #[tokio::test]
    async fn a_host_reply_and_the_chat_reply_are_one_message() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        marked(&f, &chat.id, "m1", "what day is it");
        let run = run_for(&f, &bot, &chat).await;

        run.store_reply("first", None).await.unwrap();
        run.store_reply("second", None).await.unwrap();

        let replies = f.messages_of(&chat.id, &bot.id);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].envelope["body"]["text"], json!("first"));
    }

    #[tokio::test]
    async fn interactive_room_detection_does_not_need_a_memory_trigger() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let direct = f.direct(&bot);
        let group = f.conversation("group", &[&bot.id]);
        let direct_run = run_for(&f, &bot, &direct).await;
        let group_run = run_for(&f, &bot, &group).await;
        assert!(direct_run.memory().await.is_none());
        assert!(group_run.memory().await.is_none());
        assert!(direct_run.owner_direct().await);
        assert!(!group_run.owner_direct().await);
    }

    #[tokio::test]
    async fn the_run_reads_its_memory_from_the_local_conversation() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let chat = f.direct(&bot);
        marked(&f, &chat.id, "m1", "remember my sister is Eva");
        let run = run_for(&f, &bot, &chat).await;
        let source = run.memory().await.expect("a direct chat remembers");
        assert_eq!(source.trigger_seq, 1);
    }

    #[tokio::test]
    async fn a_conversation_that_exists_nowhere_opens_no_run() {
        let f = Fixture::open();
        let bot = f.bot("Ada");
        let opened = InteractiveRun::open(
            Arc::new(Direct(f.store.clone())),
            None,
            &bot.id,
            InteractiveArgs {
                conversation_id: "nope".into(),
                trigger_message_id: "m1".into(),
                client_run_id: "run_1".into(),
            },
        )
        .await;
        assert!(opened.is_none());
    }
}
