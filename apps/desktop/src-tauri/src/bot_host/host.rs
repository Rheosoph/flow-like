//! Runs a local bot's turn when a message waits for it. Waiting work is found by [`rule`] after
//! every change of the local inbox and at startup; each bot answers one conversation at a time
//! and a few turns run at once. Messages that arrive while a bot is busy or queued are answered
//! by one later turn, and the reply is posted as the bot under an id derived from the message it
//! answers, so a turn run twice replies once.

use super::{
    gate::ApprovalGate,
    memory::{self, JobMemory},
    room_updates::LocalRoomUpdates,
    rule::{self, Waiting},
    store::{HostStore, StoreGrants},
    transcript::{self, Unreadable},
};
use chrono::{Duration as Age, Utc};
use flow_like::flow::copilot::{
    decision::{self, Decision, DecisionParticipant},
    decision_event::{self, EventDecision},
    memory_open::follow_up,
    platform::RoomUpdates,
    tool_grants::{GrantStore, clear_tokens, resume_note},
};
use flow_like_local_bots::{Bot, Conversation, InboxEntry, LOCAL_OWNER_ID, SendEntryRequest};
use flow_like_types::{
    bot::BotSystemKind,
    bot_turn::{
        DEFAULT_TURN_DEADLINE_SECS, TurnStatus, failure_message_id, reply_message_id, turn_id,
    },
    inbox::{ConversationKind, InboxEntryKind, PrincipalKind, plain_envelope},
};
use flow_like_types::{bot_effective::BotRoomLimits, bot_message::ApprovalRequest};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc, watch};

pub use flow_like_bot_host::host::*;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub slots: usize,
    pub deadline: Duration,
    pub max_age: Age,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            slots: 4,
            deadline: Duration::from_secs(DEFAULT_TURN_DEADLINE_SECS),
            max_age: Age::hours(24),
        }
    }
}

type Key = (String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cancel {
    User,
    Store,
}

struct Live {
    info: TurnInfo,
    trigger_seq: Option<i64>,
    cancel: watch::Sender<Option<Cancel>>,
    rerun: bool,
    posting: bool,
}

#[derive(Default)]
struct State {
    live: HashMap<Key, Live>,
    finished: VecDeque<TurnInfo>,
    abandoned: HashSet<Key>,
}

enum Wake {
    Conversation(String),
    All,
}

struct End {
    status: TurnStatus,
    error_code: Option<String>,
    reply: Option<String>,
    abandon: bool,
    rerun: bool,
}

impl End {
    fn cancelled(code: &'static str) -> Self {
        Self {
            status: TurnStatus::Cancelled,
            error_code: Some(code.to_owned()),
            reply: None,
            abandon: false,
            rerun: false,
        }
    }

    /// A turn nobody asked to answer that stayed silent: done, nothing posted, and the message
    /// is not run again.
    fn silent() -> Self {
        Self {
            status: TurnStatus::Done,
            error_code: Some(flow_like_types::bot_turn::ERROR_SILENT.to_owned()),
            abandon: true,
            ..Self::cancelled(CODE_CANCELLED)
        }
    }

    fn store_changed() -> Self {
        Self {
            rerun: true,
            ..Self::cancelled(CODE_STORE_CHANGED)
        }
    }

    fn by(cancel: Cancel) -> Self {
        match cancel {
            Cancel::User => Self {
                abandon: true,
                ..Self::cancelled(CODE_CANCELLED)
            },
            Cancel::Store => Self::store_changed(),
        }
    }

    fn abandoned(code: &'static str) -> Self {
        Self {
            abandon: true,
            ..Self::cancelled(code)
        }
    }

    fn waiting() -> Self {
        Self {
            status: TurnStatus::Waiting,
            error_code: None,
            reply: None,
            abandon: false,
            rerun: false,
        }
    }

    fn posted(message_id: String, failure: Option<&str>) -> Self {
        Self {
            status: if failure.is_some() {
                TurnStatus::Failed
            } else {
                TurnStatus::Done
            },
            error_code: failure.map(str::to_owned),
            reply: Some(message_id),
            abandon: false,
            rerun: false,
        }
    }

    fn refused() -> Self {
        Self {
            status: TurnStatus::Failed,
            abandon: true,
            ..Self::cancelled(CODE_REPLY_REFUSED)
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn cancelled(cancel: &mut watch::Receiver<Option<Cancel>>) -> Cancel {
    loop {
        if let Some(reason) = *cancel.borrow_and_update() {
            return reason;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

struct Inner {
    store: Arc<dyn HostStore>,
    runner: Arc<dyn TurnRunner>,
    events: Arc<dyn TurnEvents>,
    config: HostConfig,
    slots: Arc<Semaphore>,
    state: Mutex<State>,
    wake: mpsc::UnboundedSender<Wake>,
}

#[derive(Clone)]
pub struct BotHost {
    inner: Arc<Inner>,
}

pub struct Dispatcher {
    host: BotHost,
    wake: mpsc::UnboundedReceiver<Wake>,
}

impl BotHost {
    pub fn new(
        store: Arc<dyn HostStore>,
        runner: Arc<dyn TurnRunner>,
        events: Arc<dyn TurnEvents>,
        config: HostConfig,
    ) -> (Self, Dispatcher) {
        let (wake, receiver) = mpsc::unbounded_channel();
        let host = Self {
            inner: Arc::new(Inner {
                store,
                runner,
                events,
                slots: Arc::new(Semaphore::new(config.slots)),
                config,
                state: Mutex::default(),
                wake,
            }),
        };
        let dispatcher = Dispatcher {
            host: host.clone(),
            wake: receiver,
        };
        (host, dispatcher)
    }

    pub(super) fn slots(&self) -> Arc<Semaphore> {
        self.inner.slots.clone()
    }

    /// Something changed in the inbox (`None`: several conversations). Never blocks.
    pub fn poke(&self, conversation_id: Option<&str>) {
        self.inner.poke(conversation_id);
    }

    /// Running and queued turns, then the latest finished ones, newest first.
    pub fn turns(&self) -> Vec<TurnInfo> {
        let state = lock(&self.inner.state);
        let mut live: Vec<TurnInfo> = state.live.values().map(|live| live.info.clone()).collect();
        live.sort_by(|a, b| b.queued_at.cmp(&a.queued_at));
        live.extend(state.finished.iter().rev().cloned());
        live
    }

    /// Cancels a queued or running turn without a reply; false when it isn't live or its reply is
    /// already being posted. The message stays unanswered until the next one arrives.
    pub async fn cancel(&self, turn_id: &str) -> Result<bool, String> {
        let (bot, room, trigger) = {
            let mut state = lock(&self.inner.state);
            let Some(live) = state.live.values().find(|live| live.info.id == turn_id) else {
                return Ok(false);
            };
            if live.posting {
                return Ok(false);
            }
            live.cancel.send_replace(Some(Cancel::User));
            let ids = (
                live.info.bot_id.clone(),
                live.info.conversation_id.clone(),
                live.info.trigger_message_id.clone(),
            );
            state.abandoned.insert((ids.0.clone(), ids.2.clone()));
            ids
        };
        self.inner.store.cancel_run(&bot, &room, &trigger).await?;
        Ok(true)
    }
}

impl Dispatcher {
    pub async fn run(mut self) {
        self.host.poke(None);
        while let Some(first) = self.wake.recv().await {
            let mut all = false;
            let mut conversations = BTreeSet::new();
            let mut absorb = |wake: Wake| match wake {
                Wake::All => all = true,
                Wake::Conversation(id) => {
                    conversations.insert(id);
                }
            };
            absorb(first);
            while let Ok(wake) = self.wake.try_recv() {
                absorb(wake);
            }
            let inner = &self.host.inner;
            if all {
                inner.scan_all().await;
            } else {
                for id in conversations {
                    inner.scan(&id).await;
                }
            }
        }
    }
}

impl Inner {
    fn poke(&self, conversation_id: Option<&str>) {
        let wake = conversation_id.map_or(Wake::All, |id| Wake::Conversation(id.to_owned()));
        let _ = self.wake.send(wake);
    }

    async fn scan_all(self: &Arc<Self>) {
        match self.store.conversations().await {
            Ok(conversations) => {
                for conversation in conversations {
                    self.scan(&conversation.id).await;
                }
            }
            Err(error) => tracing::warn!(%error, "Bot host: listing local conversations failed"),
        }
    }

    async fn read(&self, conversation_id: &str) -> Option<(Conversation, Vec<InboxEntry>)> {
        let conversation = match self.store.conversation(conversation_id).await {
            Ok(conversation) => conversation?,
            Err(error) => {
                tracing::warn!(%error, conversation_id, "Bot host: reading a conversation failed");
                return None;
            }
        };
        match self.store.newest(conversation_id).await {
            Ok(entries) => Some((conversation, entries)),
            Err(error) => {
                tracing::warn!(%error, conversation_id, "Bot host: reading entries failed");
                None
            }
        }
    }

    async fn mentioned_only(&self, conversation: &Conversation, bot_id: &str) -> bool {
        if conversation.kind != ConversationKind::Group {
            return false;
        }
        match self.store.room_limits(&conversation.id, bot_id).await {
            Ok(limits) => !BotRoomLimits::reads_along(limits.as_ref()),
            Err(error) => {
                tracing::warn!(%error, bot_id, conversation_id = %conversation.id, "Bot host: room limits unreadable; the bot answers only when mentioned");
                true
            }
        }
    }

    async fn waiting_for(
        &self,
        conversation: &Conversation,
        entries: &[InboxEntry],
        bot_id: &str,
    ) -> Option<(Bot, Waiting)> {
        let member = conversation
            .members
            .iter()
            .any(|member| member.principal_id == bot_id && member.kind == PrincipalKind::Bot);
        if !member {
            return None;
        }
        let bot = self.store.bot(bot_id).await.ok().flatten()?;
        if bot.system == Some(BotSystemKind::Flux) {
            return None;
        }
        let recovery = match self.store.recovery_trigger(bot_id, &conversation.id).await {
            Ok(trigger) => trigger,
            Err(error) => {
                tracing::warn!(%error, bot_id, "Bot host: recovery state could not be read");
                return None;
            }
        };
        if let Some(trigger) = recovery {
            let prior: Vec<_> = entries
                .iter()
                .filter(|entry| entry.seq <= trigger.seq)
                .cloned()
                .collect();
            let waiting = rule::waiting_held(
                conversation.kind,
                bot_id,
                &prior,
                trigger.created_at - Age::seconds(1),
                Some(super::interactive::hold_now(bot_id, &conversation.id)),
                false,
            )?;
            if waiting.message_id == trigger.message_id {
                return Some((bot, waiting));
            }
            return None;
        }
        let not_before = (Utc::now() - self.config.max_age).fixed_offset();
        let hold = super::interactive::hold_now(bot_id, &conversation.id);
        let mentioned_only = self.mentioned_only(conversation, bot_id).await;
        let decision_state = {
            if bot.model.decision.is_none() {
                if let Err(error) = self.store.clear_room_wait(bot_id, &conversation.id).await {
                    tracing::warn!(%error, bot_id, "Bot host: participant wait could not be cleared");
                    return None;
                }
            }
            match self
                .store
                .room_decision_state(bot_id, &conversation.id)
                .await
            {
                Ok(state) => Some(state),
                Err(error) => {
                    tracing::warn!(%error, bot_id, "Bot host: participation state could not be read");
                    return None;
                }
            }
        };
        let waiting_pending = decision_state
            .as_ref()
            .is_some_and(|state| state.participant_id.is_some() && !state.ready);
        let normal = rule::waiting_held(
            conversation.kind,
            bot_id,
            entries,
            not_before,
            Some(hold),
            mentioned_only || waiting_pending,
        );
        let resumed = decision_state
            .as_ref()
            .filter(|state| state.ready)
            .and_then(|state| {
                rule::waiting_participant(
                    conversation.kind,
                    bot_id,
                    state.participant_id.as_deref()?,
                    state.handled_through,
                    entries,
                    not_before,
                    Some(hold),
                )
            });
        let spoken = (conversation.kind == ConversationKind::Group
            && bot.model.decision.is_some()
            && !mentioned_only)
            .then(|| {
                rule::waiting_participant(
                    conversation.kind,
                    bot_id,
                    LOCAL_OWNER_ID,
                    decision_state
                        .as_ref()
                        .map_or(0, |state| state.handled_through),
                    entries,
                    not_before,
                    Some(hold),
                )
            })
            .flatten()
            .filter(|waiting| {
                entries.iter().any(|entry| {
                    entry.message_id == waiting.message_id
                        && entry.kind == InboxEntryKind::Transcript
                })
            });
        let waiting = normal
            .into_iter()
            .chain(resumed)
            .chain(spoken)
            .max_by_key(|waiting| waiting.seq)?;
        match self.store.run_cancelled(bot_id, &waiting.message_id).await {
            Ok(false) => {}
            Ok(true) => return None,
            Err(error) => {
                tracing::warn!(%error, bot_id, "Bot host: cancellation state could not be read");
                return None;
            }
        }
        if let Some(state) = decision_state {
            if waiting.seq.is_none_or(|seq| seq <= state.handled_through)
                || (state.participant_id.is_some() && !state.ready && !waiting.addressed)
            {
                return None;
            }
        }
        Some((bot, waiting))
    }

    async fn scan(self: &Arc<Self>, conversation_id: &str) {
        let Some((conversation, entries)) = self.read(conversation_id).await else {
            self.cancel_conversation(conversation_id, &HashSet::new());
            return;
        };
        let mut seen = HashSet::new();
        for member in &conversation.members {
            if member.kind != PrincipalKind::Bot {
                continue;
            }
            seen.insert(member.principal_id.clone());
            let waiting = self
                .waiting_for(&conversation, &entries, &member.principal_id)
                .await
                .map(|(_, waiting)| waiting);
            self.apply(&member.principal_id, conversation_id, waiting);
        }
        self.cancel_conversation(conversation_id, &seen);
    }

    fn cancel_conversation(&self, conversation_id: &str, keep: &HashSet<String>) {
        let state = lock(&self.state);
        for ((bot_id, conversation), live) in &state.live {
            if conversation == conversation_id && !keep.contains(bot_id) && !live.posting {
                live.cancel.send_replace(Some(Cancel::Store));
            }
        }
    }

    fn apply(self: &Arc<Self>, bot_id: &str, conversation_id: &str, waiting: Option<Waiting>) {
        let key = (bot_id.to_owned(), conversation_id.to_owned());
        let mut publish = None;
        let mut start = None;
        {
            let mut state = lock(&self.state);
            if let Some(live) = state.live.get_mut(&key) {
                match waiting {
                    None => {
                        if !live.posting {
                            live.cancel.send_replace(Some(Cancel::Store));
                        }
                    }
                    Some(waiting) if waiting.message_id == live.info.trigger_message_id => {}
                    Some(waiting) if live.info.status == TurnStatus::Queued => {
                        retarget(live, bot_id, &waiting);
                        live.info.batched += 1;
                        publish = Some(live.info.clone());
                    }
                    Some(waiting) => {
                        let older = matches!(
                            (waiting.seq, live.trigger_seq),
                            (Some(new), Some(running)) if new < running
                        );
                        if older && !live.posting {
                            live.cancel.send_replace(Some(Cancel::Store));
                        } else {
                            live.rerun = true;
                        }
                    }
                }
            } else if let Some(waiting) = waiting
                && !state
                    .abandoned
                    .contains(&(bot_id.to_owned(), waiting.message_id.clone()))
            {
                let (sender, receiver) = watch::channel(None);
                let mut live = Live {
                    info: TurnInfo {
                        id: String::new(),
                        bot_id: bot_id.to_owned(),
                        conversation_id: conversation_id.to_owned(),
                        trigger_message_id: String::new(),
                        status: TurnStatus::Queued,
                        hops: 0,
                        requester_id: String::new(),
                        audience: Default::default(),
                        batched: 0,
                        queued_at: now(),
                        started_at: None,
                        finished_at: None,
                        error_code: None,
                        reply_message_id: None,
                    },
                    trigger_seq: None,
                    cancel: sender,
                    rerun: false,
                    posting: false,
                };
                retarget(&mut live, bot_id, &waiting);
                publish = Some(live.info.clone());
                state.live.insert(key.clone(), live);
                start = Some(receiver);
            }
        }
        if let Some(info) = publish {
            self.events.turn(&info);
        }
        if let Some(receiver) = start {
            tokio::spawn(self.clone().run_turn(key, receiver));
        }
    }

    async fn run_turn(self: Arc<Self>, key: Key, mut cancel: watch::Receiver<Option<Cancel>>) {
        let permit = tokio::select! {
            biased;
            reason = cancelled(&mut cancel) => {
                self.finish(&key, End::by(reason));
                return;
            }
            permit = self.slots.clone().acquire_owned() => permit.ok(),
        };
        let Some(_permit) = permit else {
            self.finish(&key, End::cancelled(CODE_CANCELLED));
            return;
        };
        let end = self.execute(&key, &mut cancel).await;
        self.finish(&key, end);
    }

    async fn execute(&self, key: &Key, cancel: &mut watch::Receiver<Option<Cancel>>) -> End {
        let (bot_id, conversation_id) = key;
        let Some((conversation, entries)) = self.read(conversation_id).await else {
            return End::store_changed();
        };
        let Some((bot, waiting)) = self.waiting_for(&conversation, &entries, bot_id).await else {
            return End::cancelled(CODE_NOT_WAITING);
        };
        let run_id = self.start(key, &waiting);
        let names = self.names(&conversation).await;
        let ephemeral_user_context = entries.iter().any(|entry| {
            entry.message_id == waiting.message_id
                && entry.envelope.get("body").is_some_and(|body| {
                    flow_like_types::bot_user_context::response_for(body, &entry.sender_id, bot_id)
                })
        });
        let memory = if ephemeral_user_context {
            None
        } else {
            memory::source_for(
                &self.store,
                &conversation,
                bot_id,
                entries
                    .iter()
                    .find(|entry| entry.message_id == waiting.message_id),
            )
            .await
            .map(JobMemory)
        };
        let mut oldest_first = entries;
        oldest_first.reverse();
        let reacting = oldest_first.iter().any(|entry| {
            entry.message_id == waiting.message_id && entry.kind == InboxEntryKind::Reaction
        });
        let mut reactions = Vec::new();
        let transcript = if reacting {
            let answered = self.reactions_answered(key, &oldest_first);
            let found = transcript::reaction_turn(
                bot_id,
                conversation.kind,
                &answered,
                &oldest_first,
                &names,
                flow_like_local_bots::LOCAL_OWNER_ID,
            );
            reactions = found.reactions;
            found.transcript
        } else {
            match transcript::transcript(
                bot_id,
                conversation.kind,
                &waiting.message_id,
                &oldest_first,
                &names,
                flow_like_local_bots::LOCAL_OWNER_ID,
            ) {
                Ok(transcript) => transcript,
                Err(Unreadable::Missing | Unreadable::Deleted) => {
                    return End::abandoned(CODE_UNREADABLE);
                }
            }
        };
        if reacting && transcript.prompt.is_empty() {
            return End::silent();
        }
        let requester =
            (conversation.kind == ConversationKind::Group || reacting || !transcript.addressed)
                .then(|| {
                    let requester_id = oldest_first
                        .iter()
                        .find(|entry| entry.message_id == waiting.message_id)
                        .map_or(LOCAL_OWNER_ID, |entry| entry.sender_id.as_str())
                        .to_owned();
                    flow_like::flow::copilot::TurnRequester {
                        requester_name: names.get(&requester_id).cloned(),
                        requester_id,
                        owner_id: LOCAL_OWNER_ID.to_owned(),
                        owner_name: names.get(LOCAL_OWNER_ID).cloned(),
                        audience: waiting.audience,
                        room: conversation.title.clone(),
                        addressed: transcript.addressed,
                        reactions,
                    }
                });
        let grants: Arc<dyn GrantStore> = Arc::new(StoreGrants {
            store: self.store.clone(),
            bot_id: bot_id.clone(),
        });
        let seen = grants.list().await.unwrap_or_default();
        let note = resume_note(grants.as_ref()).await;
        let resumed = note.is_some();
        let waiting = Waiting {
            proactive: waiting.proactive || note.is_some(),
            ..waiting
        };
        let prompt = match note {
            Some(note) => format!("{}\n\n{note}", transcript.prompt),
            None => transcript.prompt,
        };
        let (runner, stop_id) = (self.runner.clone(), run_id.clone());
        let gate = ApprovalGate::new(grants.clone(), &run_id, move || runner.cancel(&stop_id));
        let room_updates = (conversation.kind == ConversationKind::Group)
            .then(|| {
                let through = if reacting {
                    oldest_first.last()
                } else {
                    oldest_first
                        .iter()
                        .find(|entry| entry.message_id == waiting.message_id)
                };
                through.map(|trigger| {
                    Arc::new(LocalRoomUpdates::new(
                        self.store.clone(),
                        bot_id,
                        trigger,
                        names.clone(),
                    )) as Arc<dyn RoomUpdates>
                })
            })
            .flatten();
        let recovery: Option<Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>> =
            (self.store.supports_recovery() && !ephemeral_user_context).then(|| {
                Arc::new(super::recovery::LocalRecovery::new(
                    self.store.clone(),
                    bot_id,
                    conversation_id,
                    &waiting.message_id,
                    chrono::Utc::now().timestamp_millis().saturating_add(
                        self.config.deadline.as_millis().min(i64::MAX as u128) as i64,
                    ),
                ))
                    as Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>
            });
        let checkpoint = if let Some(recovery) = &recovery {
            match recovery.load().await {
                Ok(checkpoint) => checkpoint,
                Err(error) => {
                    tracing::warn!(%error, bot_id, "Bot host: saved execution state could not be read");
                    return End::abandoned(CODE_SETUP);
                }
            }
        } else {
            None
        };
        let recovering = checkpoint.is_some();
        let saved_delivery = match checkpoint
            .as_ref()
            .map(decision_event::saved_delivery)
            .transpose()
        {
            Ok(delivery) => delivery.flatten(),
            Err(error) => {
                tracing::warn!(%error, bot_id, "Bot host: saved event delivery could not be read");
                return End::abandoned(CODE_SETUP);
            }
        };
        let mut job = TurnJob {
            run_id: run_id.clone(),
            bot_id: bot_id.clone(),
            conversation_id: conversation_id.clone(),
            profile_id: bot.profile_id.clone(),
            prompt,
            acknowledgement: false,
            ephemeral_user_context,
            suppress_owner_notifications: false,
            history: transcript.history,
            memory: memory.clone(),
            room_updates,
            gate: gate.clone(),
            requester: requester.clone(),
            hub_token: None,
            answered_round: None,
            claim: None,
            recovery,
        };
        let call_utterance = oldest_first.iter().any(|entry| {
            entry.message_id == waiting.message_id && entry.kind == InboxEntryKind::Transcript
        });
        let event = (saved_delivery.is_some()
            || (!resumed
                && !recovering
                && !explicitly_delegated(bot_id, &waiting.message_id, &oldest_first)))
        .then(|| flow_like_bot_host::event::trigger(&oldest_first, &waiting.message_id))
        .flatten();
        if event.as_ref().is_some_and(decision_event::monitoring)
            && saved_delivery.is_none_or(|delivery| delivery.choice.is_none())
        {
            let previous = match self
                .store
                .previous_monitor_result(bot_id, conversation_id, &waiting.message_id)
                .await
            {
                Ok(previous) => previous,
                Err(error) => {
                    tracing::warn!(%error, bot_id, "Bot host: previous monitoring result is unavailable");
                    None
                }
            };
            job.prompt = format!(
                "{}\n\nCurrent scheduled check:\n{}",
                flow_like_bot_host::event::baseline_note(previous.as_deref()),
                job.prompt
            );
        }
        let mut event_delivery = saved_delivery.and_then(|delivery| delivery.choice);
        if let Some(event) = &event
            && bot.model.decision.is_some()
            && saved_delivery.is_none()
            && !resumed
            && !recovering
        {
            let classified = tokio::select! {
                biased;
                reason = cancelled(cancel) => return End::by(reason),
                result = tokio::time::timeout(Duration::from_secs(decision::TIMEOUT_SECS), flow_like_bot_host::event::classify(self.runner.as_ref(), &job, event, None)) => result,
            };
            match classified {
                Ok(Ok(Some(EventDecision::Ignore))) => {
                    return self.quiet_event(key, &waiting).await;
                }
                Ok(Ok(choice)) => event_delivery = choice,
                Ok(Err(RunError::SessionChanged)) => return End::cancelled(CODE_SESSION_CHANGED),
                Ok(Err(_)) | Err(_) => tracing::warn!(
                    bot_id,
                    "Bot host: event decision unavailable; processing normally"
                ),
            }
            let instruction = if decision_event::monitoring(event) {
                decision_event::MONITOR_INSTRUCTION
            } else {
                event_delivery
                    .map(EventDecision::instruction)
                    .unwrap_or_default()
            };
            if !instruction.is_empty() {
                job.prompt
                    .push_str(&format!("\n\nHost delivery instruction: {instruction}"));
                job.suppress_owner_notifications = true;
            }
        }
        let delivery_state = saved_delivery.or_else(|| {
            job.suppress_owner_notifications
                .then(|| decision_event::EventDelivery {
                    choice: event_delivery,
                    monitoring: event.as_ref().is_some_and(decision_event::monitoring),
                })
        });
        let event_recovery = delivery_state
            .and_then(|delivery| flow_like_bot_host::event::attach_delivery(&mut job, delivery));
        if (conversation.kind == ConversationKind::Group || call_utterance)
            && event.is_none()
            && bot.model.decision.is_some()
            && !resumed
            && !recovering
            && !explicitly_delegated(bot_id, &waiting.message_id, &oldest_first)
        {
            let participants = conversation
                .members
                .iter()
                .map(|member| DecisionParticipant {
                    id: member.principal_id.clone(),
                    name: names
                        .get(&member.principal_id)
                        .cloned()
                        .unwrap_or_else(|| member.principal_id.clone()),
                    kind: member.kind.as_str().to_owned(),
                })
                .collect::<Vec<_>>();
            if let Ok(request) =
                participation_request(&job, &waiting.message_id, &oldest_first, &participants)
            {
                let decision = tokio::select! {
                    biased;
                    reason = cancelled(cancel) => return End::by(reason),
                    result = tokio::time::timeout(Duration::from_secs(decision::TIMEOUT_SECS), self.runner.decide(&job, &request)) => result,
                };
                match decision {
                    Ok(Ok(Decision::NoAnswer)) => {
                        return match self
                            .store
                            .settle_room_decision(
                                bot_id,
                                conversation_id,
                                &waiting.message_id,
                                None,
                            )
                            .await
                        {
                            Ok(_) => End::silent(),
                            Err(error) => {
                                tracing::warn!(%error, bot_id, "Bot host: participation decision could not be saved");
                                End::abandoned(CODE_SETUP)
                            }
                        };
                    }
                    Ok(Ok(Decision::WaitFor(participant))) => {
                        let participant = (conversation.kind == ConversationKind::Group)
                            .then_some(participant.as_str());
                        match self
                            .store
                            .settle_room_decision(
                                bot_id,
                                conversation_id,
                                &waiting.message_id,
                                participant,
                            )
                            .await
                        {
                            Ok(flow_like_local_bots::RoomWaitOutcome::Parked)
                                if participant.is_none() =>
                            {
                                return End::silent();
                            }
                            Ok(flow_like_local_bots::RoomWaitOutcome::Parked) => {
                                return End::waiting();
                            }
                            Ok(flow_like_local_bots::RoomWaitOutcome::AlreadyAnswered) => {}
                            Ok(flow_like_local_bots::RoomWaitOutcome::Unavailable) => {}
                            Ok(flow_like_local_bots::RoomWaitOutcome::Superseded) => {
                                return End::silent();
                            }
                            Err(error) => {
                                tracing::warn!(%error, bot_id, "Bot host: participant wait could not be saved");
                                return End::abandoned(CODE_SETUP);
                            }
                        }
                    }
                    Ok(Err(RunError::SessionChanged)) => {
                        return End::cancelled(CODE_SESSION_CHANGED);
                    }
                    Ok(Err(_)) | Err(_) => tracing::warn!(
                        bot_id,
                        "Bot host: decision model unavailable; using normal response behavior"
                    ),
                    Ok(Ok(Decision::Acknowledge)) => job.acknowledgement = call_utterance,
                    Ok(Ok(Decision::AnswerNow)) => {}
                }
            }
        }
        if (conversation.kind == ConversationKind::Group || call_utterance)
            && bot.model.decision.is_some()
        {
            if let Err(error) = self.store.clear_room_wait(bot_id, conversation_id).await {
                tracing::warn!(%error, bot_id, "Bot host: previous participant wait could not be cleared");
                return End::abandoned(CODE_SETUP);
            }
        }
        let deadline = flow_like_bot_host::recovery::remaining_deadline(
            job.recovery.as_ref(),
            self.config.deadline,
        );
        let approval_recovery = job.recovery.clone();
        let acknowledgement = job.acknowledgement;
        let event_job = delivery_state.map(|_| job.clone());
        let outcome = tokio::select! {
            biased;
            reason = cancelled(cancel) => {
                self.runner.cancel(&run_id);
                return End::by(reason);
            }
            outcome = tokio::time::timeout(deadline, self.runner.run(job)) => outcome,
        };
        if let Some(card) = gate.parked() {
            if let Err(error) = flow_like_bot_host::recovery::preserve_unexecuted_approval(
                approval_recovery.as_ref(),
                &gate,
            )
            .await
            {
                return self
                    .fail(
                        key,
                        &waiting,
                        CODE_SETUP,
                        &format!("The pending approval could not be saved: {error}"),
                        cancel,
                    )
                    .await;
            }
            return self.park(key, &waiting, &card, cancel).await;
        }
        let failed_execution = if let Some(job) = &event_job {
            flow_like_bot_host::event::failed_execution(job).await
        } else {
            false
        };
        if failed_execution {
            event_delivery = None;
        }
        let pending_monitor = delivery_state.is_some_and(|saved| saved.monitoring)
            && event_delivery.is_none()
            && !failed_execution
            && matches!(&outcome, Ok(Ok(text)) if !text.trim().is_empty());
        if pending_monitor {
            event_delivery = Some(EventDecision::Notify);
        }
        if let (Some(event), Some(job), Ok(Ok(text))) = (&event, &event_job, &outcome)
            && decision_event::monitoring(event)
            && pending_monitor
            && !text.trim().is_empty()
        {
            let classified = tokio::select! {
                biased;
                reason = cancelled(cancel) => return End::by(reason),
                result = tokio::time::timeout(Duration::from_secs(decision::TIMEOUT_SECS), flow_like_bot_host::event::classify(self.runner.as_ref(), job, event, Some(text))) => result,
            };
            match classified {
                Ok(Ok(choice)) => event_delivery = Some(choice.unwrap_or(EventDecision::Notify)),
                Ok(Err(RunError::SessionChanged)) => return End::cancelled(CODE_SESSION_CHANGED),
                Ok(Err(_)) | Err(_) => {
                    event_delivery = Some(EventDecision::Notify);
                    tracing::warn!(
                        bot_id,
                        "Bot host: monitoring decision unavailable; delivering the result"
                    );
                }
            }
        }
        if let Some(recovery) = &event_recovery
            && !failed_execution
            && matches!(&outcome, Ok(Ok(_)))
            && delivery_state.is_some_and(|saved| saved.choice != event_delivery)
            && let Err(error) = recovery.record(event_delivery).await
        {
            return self
                .post(
                    key,
                    &waiting,
                    &format!(
                        "The event completed, but its delivery state could not be saved: {error}"
                    ),
                    Some(CODE_SETUP),
                    cancel,
                )
                .await;
        }
        if matches!(&outcome, Ok(Ok(_))) && event_delivery.is_some_and(EventDecision::quiet) {
            clear_tokens(grants.as_ref(), &seen).await;
            if let Some(JobMemory(source)) = memory {
                follow_up(bot_id, source);
            }
            return self.quiet_event(key, &waiting).await;
        }
        let quiet = event.is_none() && requester.as_ref().is_some_and(|r| r.may_stay_silent());
        let output_waiting = Waiting {
            proactive: waiting.proactive && event_delivery != Some(EventDecision::Respond),
            ..waiting.clone()
        };
        match outcome {
            Ok(Ok(text))
                if requester
                    .as_ref()
                    .is_some_and(|r| r.answer_is_silent(&text))
                    || (acknowledgement
                        && flow_like::flow::copilot::bot_run::is_silent_reply(&text)) =>
            {
                End::silent()
            }
            Ok(Err(RunError::SessionChanged)) => End::cancelled(CODE_SESSION_CHANGED),
            Ok(Err(_)) if quiet => End::silent(),
            Err(_) if quiet => {
                self.runner.cancel(&run_id);
                End::silent()
            }
            Ok(Ok(text)) if !text.trim().is_empty() => {
                let end = self
                    .post_reply(
                        key,
                        &output_waiting,
                        &clipped(&text, MAX_REPLY_CHARS),
                        None,
                        acknowledgement,
                        cancel,
                    )
                    .await;
                if end.status == TurnStatus::Done {
                    clear_tokens(grants.as_ref(), &seen).await;
                }
                if !acknowledgement
                    && let (Some(JobMemory(source)), TurnStatus::Done) = (memory, &end.status)
                {
                    follow_up(bot_id, source);
                }
                end
            }
            Ok(Ok(_)) => {
                self.fail(
                    key,
                    &waiting,
                    CODE_MODEL,
                    "The language model returned no answer. You can ask again.",
                    cancel,
                )
                .await
            }
            Ok(Err(RunError::Failed { code, message } | RunError::Retry { code, message })) => {
                let text = format!(
                    "I couldn't answer this: {}",
                    clipped(&message, MAX_FAILURE_DETAIL_CHARS)
                );
                self.post(key, &waiting, &text, Some(&code), cancel).await
            }
            Err(_) => {
                self.runner.cancel(&run_id);
                self.fail(
                    key,
                    &waiting,
                    CODE_TIMEOUT,
                    "I ran out of time before I could answer. You can ask again.",
                    cancel,
                )
                .await
            }
        }
    }

    /// Posts the approval card and parks the turn; the person's answer lets the rule run the
    /// message again.
    async fn park(
        &self,
        key: &Key,
        waiting: &Waiting,
        card: &ApprovalRequest,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        if let Some(Cancel::User) = *cancel.borrow() {
            return End::by(Cancel::User);
        }
        let (bot_id, conversation_id) = key;
        let turn = turn_id(bot_id, &waiting.message_id);
        match self
            .store
            .park(&turn, bot_id, conversation_id, &waiting.message_id, card)
            .await
        {
            Ok(()) => End::waiting(),
            Err(error) => {
                tracing::warn!(%error, bot_id, "Bot host: parking a turn failed");
                let text = format!(
                    "I needed your approval to run {} but could not ask you: {}",
                    card.tool,
                    clipped(&error, MAX_FAILURE_DETAIL_CHARS)
                );
                self.post(key, waiting, &text, Some(CODE_SETUP), cancel)
                    .await
            }
        }
    }

    async fn fail(
        &self,
        key: &Key,
        waiting: &Waiting,
        code: &'static str,
        text: &str,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        self.post(key, waiting, text, Some(code), cancel).await
    }

    /// The reaction entries a local turn answers: those after the bot's last message and after
    /// the trigger of its last finished turn here (entries oldest first).
    fn reactions_answered(&self, key: &Key, entries: &[InboxEntry]) -> Vec<String> {
        let (bot_id, conversation_id) = key;
        let finished: HashSet<String> = lock(&self.state)
            .finished
            .iter()
            .filter(|info| &info.bot_id == bot_id && &info.conversation_id == conversation_id)
            .map(|info| info.trigger_message_id.clone())
            .collect();
        let last_own = entries
            .iter()
            .rposition(|entry| &entry.sender_id == bot_id && entry.kind == InboxEntryKind::Message);
        let last_turn = entries
            .iter()
            .rposition(|entry| finished.contains(&entry.message_id));
        let after = last_own.into_iter().chain(last_turn).max();
        entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                entry.kind == InboxEntryKind::Reaction && after.is_none_or(|after| *index > after)
            })
            .map(|(_, entry)| entry.message_id.clone())
            .collect()
    }

    async fn names(&self, conversation: &Conversation) -> HashMap<String, String> {
        let mut names = HashMap::new();
        names.insert(LOCAL_OWNER_ID.to_owned(), "Owner".to_owned());
        for member in &conversation.members {
            if member.kind != PrincipalKind::Bot {
                continue;
            }
            if let Ok(Some(bot)) = self.store.bot(&member.principal_id).await
                && let Some(name) = bot.name.filter(|name| !name.trim().is_empty())
            {
                names.insert(member.principal_id.clone(), name);
            }
        }
        names
    }

    async fn quiet_event(&self, key: &Key, waiting: &Waiting) -> End {
        match self
            .store
            .settle_room_decision(&key.0, &key.1, &waiting.message_id, None)
            .await
        {
            Ok(
                flow_like_local_bots::RoomWaitOutcome::Parked
                | flow_like_local_bots::RoomWaitOutcome::Superseded,
            ) => End::silent(),
            Ok(_) => End::abandoned(CODE_NOT_WAITING),
            Err(error) => {
                tracing::warn!(%error, "Bot host: quiet background result could not be recorded");
                End::abandoned(CODE_SETUP)
            }
        }
    }

    /// Posts the bot's reply, unless the turn was cancelled first; from here on it can't be.
    async fn post(
        &self,
        key: &Key,
        waiting: &Waiting,
        text: &str,
        error_code: Option<&str>,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        self.post_reply(key, waiting, text, error_code, false, cancel)
            .await
    }

    async fn post_reply(
        &self,
        key: &Key,
        waiting: &Waiting,
        text: &str,
        error_code: Option<&str>,
        acknowledgement: bool,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        {
            let mut state = lock(&self.state);
            let Some(live) = state.live.get_mut(key) else {
                return End::store_changed();
            };
            if let Some(reason) = *cancel.borrow() {
                return End::by(reason);
            }
            live.posting = true;
        }
        let (bot_id, conversation_id) = key;
        let message_id = if error_code.is_some() {
            failure_message_id(bot_id, &waiting.message_id)
        } else {
            reply_message_id(bot_id, &waiting.message_id)
        };
        let mut request = reply_request(message_id.clone(), waiting, text, error_code);
        if error_code.is_none()
            && acknowledgement
            && let Some(envelope) = request.envelope.as_mut()
        {
            envelope["body"][decision::ACKNOWLEDGEMENT_FIELD] = serde_json::json!(true);
        }
        match self.store.send(conversation_id, bot_id, request).await {
            Ok(_) => {
                if error_code.is_none() && !acknowledgement && waiting.proactive {
                    self.alert_owner(bot_id, &message_id, text).await;
                }
                End::posted(message_id, error_code)
            }
            Err(error) => {
                tracing::warn!(%error, bot_id, conversation_id, "Bot host: posting a reply failed");
                End::refused()
            }
        }
    }

    async fn alert_owner(&self, bot_id: &str, message_id: &str, text: &str) {
        let Ok(Some(bot)) = self.store.bot(bot_id).await else {
            return;
        };
        if let Some(alert) = super::notify::reply_alert(&bot, true, message_id, text) {
            self.events.alert(&alert);
        }
    }

    fn start(&self, key: &Key, waiting: &Waiting) -> String {
        let info = {
            let mut state = lock(&self.state);
            let Some(live) = state.live.get_mut(key) else {
                return String::new();
            };
            retarget(live, &key.0, waiting);
            live.info.status = TurnStatus::Running;
            live.info.started_at = Some(now());
            live.info.clone()
        };
        self.events.turn(&info);
        info.id
    }

    fn finish(&self, key: &Key, end: End) {
        let (info, rerun) = {
            let mut state = lock(&self.state);
            let Some(mut live) = state.live.remove(key) else {
                return;
            };
            live.info.status = end.status;
            live.info.finished_at = Some(now());
            live.info.error_code = end.error_code;
            live.info.reply_message_id = end.reply;
            if end.abandon {
                state
                    .abandoned
                    .insert((key.0.clone(), live.info.trigger_message_id.clone()));
            }
            state.finished.push_back(live.info.clone());
            while state.finished.len() > FINISHED_KEPT {
                state.finished.pop_front();
            }
            (live.info, live.rerun || end.rerun)
        };
        self.events.turn(&info);
        if rerun {
            self.poke(Some(&key.1));
        }
    }
}

fn explicitly_delegated(bot_id: &str, trigger: &str, entries: &[InboxEntry]) -> bool {
    entries.iter().any(|entry| {
        entry.message_id == trigger
            && entry.kind == InboxEntryKind::Message
            && entry.sender_id != bot_id
            && entry.sender_id != LOCAL_OWNER_ID
            && rule::is_plain(entry)
            && entry
                .envelope
                .get("body")
                .and_then(|body| flow_like_types::bot_message::MessageBody::parse(body).ok())
                .is_some_and(|body| {
                    body.mentions.iter().any(|id| id == bot_id)
                        && body.ask.is_some_and(|ask| {
                            ask.id == entry.message_id
                                && ask.chain.last().is_none_or(|id| id == &entry.sender_id)
                        })
                })
    })
}

#[cfg(test)]
mod participation_guards {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_valid_addressed_bot_request_bypasses_participation() {
        let mut entry = InboxEntry {
            id: "entry".into(),
            message_id: "question".into(),
            conversation_id: "room".into(),
            sender_id: "peer".into(),
            kind: InboxEntryKind::Message,
            envelope: plain_envelope(json!({
                "text": "What is the total?", "mentions": ["bot"],
                "ask": {
                    "id": "question", "origin_conversation": "origin", "hops": 1,
                    "reply_by": (Utc::now() + Age::days(1)).to_rfc3339(),
                    "chain": ["peer"], "wait_secs": 30, "round": 1,
                },
            })),
            created_at: Utc::now().fixed_offset(),
            seq: Some(1),
            cursor: String::new(),
        };
        let delegated = |entry: &InboxEntry| {
            explicitly_delegated("bot", "question", std::slice::from_ref(entry))
        };
        assert!(delegated(&entry));
        entry.envelope["body"]["ask"]["chain"] = json!(["another-bot"]);
        assert!(!delegated(&entry));
        entry.envelope["body"]["ask"]["chain"] = json!(["peer"]);
        entry.sender_id = LOCAL_OWNER_ID.into();
        assert!(!delegated(&entry));
        entry.sender_id = "peer".into();
        entry.envelope["body"]["ask"]["id"] = json!("");
        assert!(!delegated(&entry));
    }
}

fn retarget(live: &mut Live, bot_id: &str, waiting: &Waiting) {
    live.info.id = turn_id(bot_id, &waiting.message_id);
    live.info.trigger_message_id = waiting.message_id.clone();
    live.info.hops = waiting.hops;
    live.info.requester_id.clone_from(&waiting.requester_id);
    live.info.audience = waiting.audience;
    live.trigger_seq = waiting.seq;
}

fn reply_request(
    message_id: String,
    waiting: &Waiting,
    text: &str,
    error_code: Option<&str>,
) -> SendEntryRequest {
    SendEntryRequest {
        message_id,
        kind: InboxEntryKind::Message,
        envelope: Some(plain_envelope(reply_body(
            &waiting.message_id,
            waiting.hops,
            text,
            error_code,
        ))),
        envelopes: None,
    }
}
