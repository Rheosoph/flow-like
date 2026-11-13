//! Runs the turns of the signed-in account's bots that live on this desktop (`location:
//! desktop`). The hub queues a turn when a message wakes such a bot; this host lists them, claims
//! one with a lease it keeps alive by heartbeat, runs it through the same native path as a local
//! bot and answers as the bot with the turn it holds. A poll every 30 s finds new turns, and
//! [`AccountHost::poke`] brings a poll forward; the desktop pokes it when its realtime connection
//! reports inbox activity, so a turn is claimed within moments instead of at the next poll.

use crate::{
    authorization::{HubAuthority, RunnerInference},
    e2e::{self, E2eHost},
    gate::ApprovalGate,
    host::{
        CODE_CANCELLED, CODE_MODEL, CODE_TIMEOUT, CODE_UNREADABLE, FINISHED_KEPT, HubToken,
        MAX_FAILURE_DETAIL_CHARS, MAX_REPLY_CHARS, RunError, RunFuture, RunOutcome, TurnEvents,
        TurnInfo, TurnJob, TurnRunner, clipped, now, participation_request, reply_body,
    },
    hub::{Claimed, HubFailure, HubGrants, TurnHub, TurnSession, WaitOutcome},
    latency::{self, CallTrace},
    live_reply,
    memory::{self, JobMemory},
    notes::{HubNotes, StatusTurn},
    room_updates::{HubRoomUpdates, RoomUpdateTracker},
    transcript,
};
use flow_like::flow::copilot::{
    ReplyFiles, ToolTrail, TurnRequester,
    bot_notes::NotesHandle,
    bot_talk::AnsweredRound,
    decision::{self, Decision, DecisionParticipant},
    decision_event::{self, EventDecision},
    memory_open::follow_up,
    tool_gate::{ToolGateOpener, tool_gate},
    tool_grants::{GrantStore, clear_tokens, resume_note},
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::{ApprovalRequest, MessageBody},
    bot_turn::{
        BotTurn, DEFAULT_TURN_DEADLINE_SECS, LEASE_SECS, TurnAudience, TurnStatus,
        failure_message_id, reply_message_id,
    },
    inbox::{ConversationKind, InboxEntryKind, plain_envelope},
};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Notify, Semaphore, watch},
    task::JoinHandle,
};

pub const CODE_LEASE_LOST: &str = "BOT_TURN_LEASE_LOST";
pub const CODE_TAKEN: &str = "BOT_TURN_TAKEN";
pub const CODE_HUB: &str = "BOT_TURN_HUB";
pub const CODE_REPLY_REFUSED: &str = "BOT_TURN_REPLY_REFUSED";
const CODE_SETUP: &str = "BOT_TURN_SETUP";
const CODE_INTERNAL: &str = "BOT_TURN_INTERNAL";
const CODE_HOST_STOPPED: &str = "BOT_TURN_HOST_STOPPED";
const LEASE_GRACE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct AccountConfig {
    pub host_id: String,
    pub poll: Duration,
    pub max_backoff: Duration,
    pub heartbeat: Duration,
    pub deadline: Duration,
    pub send_retries: u32,
    pub send_retry_delay: Duration,
    pub presence: Duration,
    /// Fires the due tasks of the hub's desktop bots each poll; off for hosts whose tasks the
    /// server fires.
    pub fire_tasks: bool,
}

impl AccountConfig {
    pub fn new(host_id: String) -> Self {
        Self {
            host_id,
            poll: Duration::from_secs(30),
            max_backoff: Duration::from_secs(300),
            heartbeat: Duration::from_secs(15),
            deadline: Duration::from_secs(DEFAULT_TURN_DEADLINE_SECS),
            send_retries: 3,
            send_retry_delay: Duration::from_secs(2),
            presence: Duration::from_secs(60),
            fire_tasks: true,
        }
    }
}

/// The wait before the next poll after `failures` failed ones in a row: doubling, capped.
pub fn backoff(poll: Duration, max: Duration, failures: u32) -> Duration {
    poll.saturating_mul(1u32 << failures.min(10))
        .min(max.max(poll))
}

type Key = (String, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cancel {
    User,
    LeaseLost,
}

struct Live {
    usage: flow_like::flow::copilot::bot_usage::BotUsage,
    info: TurnInfo,
    cancel: Arc<watch::Sender<Option<Cancel>>>,
    posting: bool,
    notes: Option<NotesHandle>,
    trail: Option<ToolTrail>,
    reply_files: Option<ReplyFiles>,
}

#[derive(Default)]
struct State {
    live: HashMap<Key, Live>,
    finished: VecDeque<TurnInfo>,
}

struct End {
    status: TurnStatus,
    error_code: String,
    reply: Option<String>,
}

struct LateContinuation {
    conversation_id: String,
    kind: ConversationKind,
    transcript: transcript::Transcript,
    answered_round: Option<AnsweredRound>,
}

impl End {
    fn dropped(code: &str) -> Self {
        Self {
            status: TurnStatus::Cancelled,
            error_code: code.to_owned(),
            reply: None,
        }
    }

    fn lease_lost() -> Self {
        Self::dropped(CODE_LEASE_LOST)
    }

    fn by(cancel: Cancel) -> Self {
        match cancel {
            Cancel::User => Self::dropped(CODE_CANCELLED),
            Cancel::LeaseLost => Self::lease_lost(),
        }
    }

    fn failed(code: &str) -> Self {
        Self {
            status: TurnStatus::Failed,
            ..Self::dropped(code)
        }
    }
}

/// Ends the turn's local entry and its heartbeat however the turn task leaves: a panic or an
/// aborted task fails the turn here instead of keeping the lease alive.
struct TurnGuard {
    inner: Arc<Inner>,
    key: Key,
    beat: Option<JoinHandle<()>>,
    armed: bool,
}

impl TurnGuard {
    fn complete(mut self, end: End) {
        self.armed = false;
        self.inner.finish(&self.key, end);
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if let Some(beat) = self.beat.take() {
            beat.abort();
        }
        if self.armed {
            self.inner.finish(&self.key, End::failed(CODE_INTERNAL));
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
    hub: Arc<dyn TurnHub>,
    runner: Arc<dyn TurnRunner>,
    events: Arc<dyn TurnEvents>,
    config: AccountConfig,
    slots: Arc<Semaphore>,
    state: Mutex<State>,
    wake: Notify,
    claim_failures: AtomicU32,
    hosted_bots: AtomicUsize,
    reporting: AtomicBool,
    e2e: OnceLock<Arc<E2eHost>>,
}

#[derive(Clone)]
pub struct AccountHost {
    inner: Arc<Inner>,
}

impl AccountHost {
    pub fn new(
        hub: Arc<dyn TurnHub>,
        runner: Arc<dyn TurnRunner>,
        events: Arc<dyn TurnEvents>,
        slots: Arc<Semaphore>,
        config: AccountConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                hub,
                runner,
                events,
                config,
                slots,
                state: Mutex::default(),
                wake: Notify::new(),
                claim_failures: AtomicU32::new(0),
                hosted_bots: AtomicUsize::new(0),
                reporting: AtomicBool::new(false),
                e2e: OnceLock::new(),
            }),
        }
    }

    /// Lets this host's encryption bots take part in end-to-end encrypted conversations.
    pub fn enable_e2e(&self, e2e: Arc<E2eHost>) {
        let _ = self.inner.e2e.set(e2e);
    }

    /// How many account bots the last successful presence report placed on this host.
    pub fn hosted_bot_count(&self) -> usize {
        self.inner.hosted_bots.load(Ordering::SeqCst)
    }

    /// Brings the next poll forward, e.g. on an inbox invalidation.
    pub fn poke(&self) {
        self.inner.wake.notify_one();
    }

    pub fn turns(&self) -> Vec<TurnInfo> {
        let state = lock(&self.inner.state);
        let mut turns: Vec<TurnInfo> = state.live.values().map(|live| live.info.clone()).collect();
        turns.extend(state.finished.iter().rev().cloned());
        turns
    }

    /// Cancels a queued or running turn through the hub's owner cancel, so the turn ends instead
    /// of being queued again, and stops it here. False when it isn't live or its reply is being
    /// posted.
    pub async fn cancel(&self, turn_id: &str) -> Result<bool, HubFailure> {
        let (bot_id, id) = {
            let state = lock(&self.inner.state);
            let Some(live) = state
                .live
                .values()
                .find(|live| live.info.id == turn_id && !live.posting)
            else {
                return Ok(false);
            };
            live.cancel.send_replace(Some(Cancel::User));
            (live.info.bot_id.clone(), live.info.id.clone())
        };
        self.inner.hub.cancel_turn(&bot_id, &id).await?;
        Ok(true)
    }

    /// Stops reporting this host's bots; call on sign-out and when the app exits.
    pub async fn withdraw(&self) {
        let inner = &self.inner;
        inner.hosted_bots.store(0, Ordering::SeqCst);
        inner.release_running().await;
        if inner.reporting.swap(false, Ordering::SeqCst)
            && let Err(failure) = inner.hub.delete_host(&inner.config.host_id).await
        {
            tracing::debug!(%failure, "Bot host: withdrawing the host failed; it reads offline soon");
        }
    }

    pub async fn run(self) {
        tokio::spawn(self.clone().report_presence());
        let inner = &self.inner;
        let mut failures = 0u32;
        loop {
            if inner.config.fire_tasks
                && let Err(failure) = crate::tasks::fire_account_tasks(&*inner.hub).await
            {
                tracing::debug!(%failure, "Bot host: firing account tasks failed");
            }
            match inner.poll_once().await {
                Ok(started) => {
                    failures = 0;
                    if started == 0 {
                        inner.claim_failures.store(0, Ordering::SeqCst);
                    }
                }
                Err(failure) => {
                    failures = failures.saturating_add(1);
                    tracing::debug!(%failure, failures, "Bot host: listing account turns failed");
                }
            }
            let poll = inner.hub.next_poll().unwrap_or(inner.config.poll);
            let streak = failures.saturating_add(inner.claim_failures.load(Ordering::SeqCst));
            let delay = backoff(poll, inner.config.max_backoff, streak);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = inner.wake.notified(), if streak == 0 => {}
            }
        }
    }
}

impl AccountHost {
    async fn report_presence(self) {
        let inner = &self.inner;
        loop {
            match inner.report_once().await {
                Ok(()) => {}
                Err(failure) if failure.signed_out() => self.withdraw().await,
                Err(failure) => tracing::debug!(%failure, "Bot host: reporting presence failed"),
            }
            tokio::time::sleep(inner.config.presence).await;
        }
    }
}

impl Inner {
    async fn report_once(&self) -> Result<(), HubFailure> {
        if let Some(e2e) = self.e2e.get()
            && let Err(failure) = e2e.sync().await
        {
            tracing::debug!(%failure, "Bot host: publishing encryption keys failed");
        }
        let bots = self.hub.reported_bots().await?;
        self.hosted_bots.store(bots.len(), Ordering::SeqCst);
        if bots.is_empty() && !self.reporting.load(Ordering::SeqCst) {
            return Ok(());
        }
        let refused = self.hub.report_host(&self.config.host_id, &bots).await?;
        self.reporting.store(true, Ordering::SeqCst);
        if !refused.is_empty() {
            tracing::debug!(
                ?refused,
                "Bot host: the hub refused some bots as this host's"
            );
        }
        Ok(())
    }

    async fn poll_once(self: &Arc<Self>) -> Result<usize, HubFailure> {
        let turns = self.hub.list().await?;
        let mut started = Vec::new();
        {
            let mut state = lock(&self.state);
            let mut seen = HashSet::new();
            for turn in turns {
                let key = (turn.bot_id.clone(), turn.conversation_id.clone());
                if state.live.contains_key(&key) || !seen.insert(key.clone()) {
                    continue;
                }
                let (cancel, receiver) = watch::channel(None);
                let live = Live {
                    usage: Default::default(),
                    info: info_of(&turn, TurnStatus::Queued),
                    cancel: Arc::new(cancel),
                    posting: false,
                    notes: None,
                    trail: None,
                    reply_files: None,
                };
                started.push((live.info.clone(), key.clone(), turn.id, receiver));
                state.live.insert(key, live);
            }
        }
        let count = started.len();
        for (info, key, turn_id, receiver) in started {
            self.events.turn(&info);
            tokio::spawn(self.clone().run_turn(key, turn_id, receiver));
        }
        Ok(count)
    }

    async fn run_turn(
        self: Arc<Self>,
        key: Key,
        listed_id: String,
        cancel: watch::Receiver<Option<Cancel>>,
    ) {
        let mut guard = TurnGuard {
            inner: self.clone(),
            key: key.clone(),
            beat: None,
            armed: true,
        };
        let end = self.run_claimed(&key, listed_id, cancel, &mut guard).await;
        let claimed = guard.beat.is_some();
        guard.complete(end);
        if claimed {
            self.wake.notify_one();
        }
    }

    async fn run_claimed(
        self: &Arc<Self>,
        key: &Key,
        listed_id: String,
        mut cancel: watch::Receiver<Option<Cancel>>,
        guard: &mut TurnGuard,
    ) -> End {
        let permit = tokio::select! {
            biased;
            reason = cancelled(&mut cancel) => return End::by(reason),
            permit = self.slots.clone().acquire_owned() => permit.ok(),
        };
        let Some(_permit) = permit else {
            return End::dropped(CODE_CANCELLED);
        };
        let claimed = match self.hub.claim_turn(&listed_id, &self.config.host_id).await {
            Ok(claimed) => {
                self.claim_failures.store(0, Ordering::SeqCst);
                claimed
            }
            Err(failure) if failure.taken() => return End::dropped(CODE_TAKEN),
            Err(failure) => {
                tracing::debug!(%failure, "Bot host: claiming an account turn failed");
                self.claim_failures.fetch_add(1, Ordering::SeqCst);
                return End::dropped(CODE_HUB);
            }
        };
        let Claimed {
            turn: lead,
            session,
        } = claimed;
        let sender = self.start(key, &lead);
        guard.beat = Some(tokio::spawn(
            self.clone().keep_lease(lead.id.clone(), sender),
        ));
        self.execute(key, &lead, session, &mut cancel).await
    }

    async fn keep_lease(
        self: Arc<Self>,
        turn_id: String,
        cancel: Arc<watch::Sender<Option<Cancel>>>,
    ) {
        let limit = self.config.deadline + LEASE_GRACE;
        let lease = Duration::from_secs(LEASE_SECS.unsigned_abs());
        let started = Instant::now();
        let mut renewed = started;
        loop {
            tokio::time::sleep(self.config.heartbeat).await;
            if started.elapsed() > limit {
                tracing::warn!(
                    turn_id,
                    "Bot host: a turn outlived its deadline; the lease is no longer renewed"
                );
                return;
            }
            match self.hub.heartbeat(&turn_id, &self.config.host_id).await {
                Ok(_) => renewed = Instant::now(),
                Err(failure) if failure.lease_lost() || failure.signed_out() || failure.taken() => {
                    cancel.send_replace(Some(Cancel::LeaseLost));
                    return;
                }
                Err(failure) => {
                    tracing::debug!(%failure, turn_id, "Bot host: a heartbeat failed");
                    if renewed.elapsed() > lease {
                        tracing::warn!(
                            turn_id,
                            "Bot host: no heartbeat reached the hub within the lease; stopping the turn"
                        );
                        cancel.send_replace(Some(Cancel::LeaseLost));
                        return;
                    }
                }
            }
        }
    }

    /// Stops the turns this host runs and hands each back to the hub so another host or a later
    /// start picks it up without waiting for the lease to run out.
    async fn release_running(&self) {
        let running: Vec<String> = lock(&self.state)
            .live
            .values()
            .filter(|live| live.info.status == TurnStatus::Running && !live.posting)
            .map(|live| {
                live.cancel.send_replace(Some(Cancel::LeaseLost));
                live.info.id.clone()
            })
            .collect();
        for turn_id in running {
            if let Err(failure) = self
                .hub
                .release(&turn_id, &self.config.host_id, CODE_HOST_STOPPED)
                .await
            {
                tracing::debug!(%failure, turn_id, "Bot host: releasing a turn at shutdown failed");
            }
        }
    }

    async fn execute(
        &self,
        key: &Key,
        lead: &BotTurn,
        session: Option<TurnSession>,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        let mut trace = CallTrace::new(CallTrace::queued_at(&lead.created_at));
        trace.turn_id = lead.id.clone();
        let end = self
            .execute_traced(key, lead, session, cancel, &mut trace)
            .await;
        trace.log(&lead.bot_id, &format!("{:?}", end.status));
        end
    }

    async fn execute_traced(
        &self,
        key: &Key,
        lead: &BotTurn,
        session: Option<TurnSession>,
        cancel: &mut watch::Receiver<Option<Cancel>>,
        trace: &mut CallTrace,
    ) -> End {
        let (bot_id, conversation_id) = key;
        let minted_by_hub = session.is_some();
        let (hub_token, hub, claim) = match session {
            Some(session) => (
                session.token.map(HubToken),
                session.hub.unwrap_or_else(|| self.hub.clone()),
                session.run,
            ),
            None => {
                let hub_token = match self.hub.runtime_token(bot_id, &lead.id).await {
                    Ok(token) => token.map(HubToken),
                    Err(failure) if failure.lease_lost() => return End::lease_lost(),
                    Err(failure) => {
                        tracing::debug!(%failure, "Bot host: the hub minted no bot token for the turn");
                        return self.release_or_abandon(lead, CODE_HUB).await;
                    }
                };
                let hub = hub_token
                    .as_ref()
                    .and_then(|token| self.hub.with_runtime_token(&token.0))
                    .unwrap_or_else(|| self.hub.clone());
                (hub_token, hub, None)
            }
        };
        let continuation = match self.late_answer(lead, &hub).await {
            Ok(continuation) => continuation,
            Err(failure) if failure.lease_lost() => return End::lease_lost(),
            Err(failure) => {
                tracing::debug!(%failure, "Bot host: reading a delayed answer's context failed");
                return self.release_or_abandon(lead, CODE_HUB).await;
            }
        };
        let continued = continuation.is_some();
        // A resumed turn's token is scoped to its origin room. The ask-result endpoint supplies
        // the exchange's answer, so the host never needs to read that exchange with this token.
        let mut entries = if continued {
            Vec::new()
        } else {
            match hub
                .entries_for_turn(bot_id, conversation_id, &lead.trigger_message_id)
                .await
            {
                Ok(entries) => entries,
                Err(failure) if failure.lease_lost() => return End::lease_lost(),
                Err(failure) => {
                    tracing::debug!(%failure, "Bot host: reading an account conversation failed");
                    return self.release_or_abandon(lead, CODE_HUB).await;
                }
            }
        };
        entries.reverse();
        let sealed = entries
            .iter()
            .any(|entry| entry.message_id == lead.trigger_message_id && e2e::is_sealed(entry));
        if sealed {
            entries = match self
                .open_sealed(bot_id, &lead.trigger_message_id, entries)
                .await
            {
                Ok(entries) => entries,
                Err(failure) => {
                    tracing::debug!(code = failure.code, message = %failure.message, "Bot host: an encrypted turn can't run");
                    return self.abandon(lead, failure.code).await;
                }
            };
        }
        if let Some(entry) = entries.iter().find(|entry| {
            entry.message_id == lead.trigger_message_id
                && matches!(
                    entry.kind,
                    InboxEntryKind::Message | InboxEntryKind::Transcript
                )
                && entry
                    .envelope
                    .get("body")
                    .is_some_and(decision::is_acknowledgement_body)
        }) {
            let room = hub
                .read(
                    &format!("inbox/conversations/{conversation_id}"),
                    vec![("as", bot_id.clone())],
                )
                .await;
            let bot_sender = room
                .as_ref()
                .ok()
                .and_then(|room| room.get("members"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|members| {
                    members.iter().any(|member| {
                        member
                            .get("principal_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(entry.sender_id.as_str())
                            && member
                                .get("kind")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|kind| kind.eq_ignore_ascii_case("bot"))
                    })
                });
            if bot_sender {
                return self.stay_silent(lead).await;
            }
        }
        let kind = match lead.audience {
            TurnAudience::Room => ConversationKind::Group,
            TurnAudience::Owner => ConversationKind::Direct,
        };
        let room = if kind == ConversationKind::Group && !continued {
            hub.read(
                &format!("inbox/conversations/{conversation_id}"),
                vec![("as", bot_id.clone())],
            )
            .await
            .ok()
        } else {
            None
        };
        let mut names: HashMap<String, String> = room
            .as_ref()
            .and_then(|room| room.get("members"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|member| {
                let id = member.get("principal_id")?.as_str()?;
                let name = member.get("name")?.as_str()?;
                Some((id.to_owned(), name.to_owned()))
            })
            .collect();
        names.extend(
            lead.requester_id
                .iter()
                .zip(lead.requester_name.iter())
                .map(|(id, name)| (id.clone(), name.clone())),
        );
        let reacting = entries.iter().any(|entry| {
            entry.message_id == lead.trigger_message_id && entry.kind == InboxEntryKind::Reaction
        });
        let mut reactions = Vec::new();
        let mut chat_conversation = conversation_id.clone();
        let mut audience = lead.audience;
        let mut answered_round = None;
        let transcript = match continuation {
            Some(continuation) => {
                chat_conversation = continuation.conversation_id;
                audience = match continuation.kind {
                    ConversationKind::Direct => TurnAudience::Owner,
                    ConversationKind::Group => TurnAudience::Room,
                };
                answered_round = continuation.answered_round;
                continuation.transcript
            }
            None if reacting => {
                let answered: Vec<String> = std::iter::once(lead.trigger_message_id.clone())
                    .chain(lead.batched.iter().cloned())
                    .collect();
                let found = transcript::reaction_turn(
                    bot_id,
                    kind,
                    &answered,
                    &entries,
                    &names,
                    &lead.owner_id,
                );
                reactions = found.reactions;
                found.transcript
            }
            None => match transcript::transcript(
                bot_id,
                kind,
                &lead.trigger_message_id,
                &entries,
                &names,
                &lead.owner_id,
            ) {
                Ok(transcript) => transcript,
                Err(_) => return self.abandon(lead, CODE_UNREADABLE).await,
            },
        };
        // Keep the claimed turn's identity and trigger, while every reply (including failures)
        // goes to the conversation whose scope the runtime token permits.
        let mut execution_turn = lead.clone();
        execution_turn.conversation_id = chat_conversation.clone();
        execution_turn.audience = audience;
        let lead = &execution_turn;
        let trigger_seq = entries
            .iter()
            .find(|entry| entry.message_id == lead.trigger_message_id)
            .and_then(|entry| entry.seq);
        let ephemeral_user_context = entries.iter().any(|entry| {
            entry.message_id == lead.trigger_message_id
                && entry.envelope.get("body").is_some_and(|body| {
                    flow_like_types::bot_user_context::response_for(body, &entry.sender_id, bot_id)
                })
        });
        let owner_hub = if minted_by_hub { &hub } else { &self.hub };
        // The exchange's memory would replace the reconstructed origin conversation and the
        // delayed-answer prompt. A continuation keeps that explicit transcript, even on old hubs
        // whose ask results have no round counter.
        let memory = if sealed || continued {
            None
        } else {
            memory::hub_source_for(
                owner_hub,
                &hub,
                bot_id,
                conversation_id,
                trigger_seq,
                Some(&lead.owner_id),
            )
            .await
            .map(JobMemory)
        };
        let grants: Arc<dyn GrantStore> = Arc::new(HubGrants {
            hub: owner_hub.clone(),
            bot_id: bot_id.clone(),
        });
        let seen = grants.list().await.unwrap_or_default();
        let no_prompt = transcript.prompt.is_empty();
        let note = resume_note(grants.as_ref()).await;
        let resumed = note.is_some();
        let requester = TurnRequester {
            requester_id: lead
                .requester_id
                .clone()
                .unwrap_or_else(|| lead.owner_id.clone()),
            requester_name: lead.requester_name.clone(),
            owner_id: lead.owner_id.clone(),
            owner_name: None,
            audience,
            room: None,
            addressed: transcript.addressed,
            reactions,
        };
        if reacting && no_prompt {
            return self.stay_silent(lead).await;
        }
        let gate = ApprovalGate::new(grants.clone(), &lead.id);
        let room_updates = (kind == ConversationKind::Group && !continued)
            .then(|| {
                if reacting {
                    entries.last()
                } else {
                    entries
                        .iter()
                        .find(|entry| entry.message_id == lead.trigger_message_id)
                }
            })
            .flatten()
            .map(|trigger| {
                Arc::new(HubRoomUpdates::new(
                    hub.clone(),
                    bot_id,
                    conversation_id,
                    sealed.then(|| self.e2e.get().cloned()).flatten(),
                    RoomUpdateTracker::new(bot_id, &lead.owner_id, trigger, names.clone()),
                )) as Arc<dyn flow_like::flow::copilot::platform::RoomUpdates>
            });
        let recovery: Option<Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>> =
            (self.hub.supports_recovery() && !ephemeral_user_context).then(|| {
                let recovery = crate::recovery::HubRecovery::new(
                    self.hub.clone(),
                    lead,
                    &self.config.host_id,
                    chrono::Utc::now().timestamp_millis().saturating_add(
                        self.config.deadline.as_millis().min(i64::MAX as u128) as i64,
                    ),
                );
                let recovery = if sealed {
                    // Decrypting this turn already required the host's retained inbox keys.
                    recovery.with_encryption(
                        self.e2e
                            .get()
                            .expect("a sealed turn has an encryption host")
                            .clone(),
                        conversation_id.clone(),
                    )
                } else {
                    recovery
                };
                Arc::new(recovery)
                    as Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>
            });
        let checkpoint = if let Some(recovery) = &recovery {
            match recovery.load().await {
                Ok(checkpoint) => checkpoint,
                Err(error) => {
                    return self
                        .retry_or_fail(
                            key,
                            lead,
                            &hub,
                            CODE_SETUP,
                            &format!("The saved execution state could not be read: {error}"),
                            sealed,
                            cancel,
                        )
                        .await;
                }
            }
        } else {
            None
        };
        let recovering = checkpoint.is_some();
        let prompt = crate::recovery::resumed_prompt(transcript.prompt, note, checkpoint.as_ref());
        let saved_delivery = match checkpoint
            .as_ref()
            .map(decision_event::saved_delivery)
            .transpose()
        {
            Ok(delivery) => delivery.flatten(),
            Err(error) => {
                return self
                    .retry_or_fail(
                        key,
                        lead,
                        &hub,
                        CODE_SETUP,
                        &error.to_string(),
                        sealed,
                        cancel,
                    )
                    .await;
            }
        };
        let notes = (!sealed && !minted_by_hub && !ephemeral_user_context).then(|| {
            HubNotes::new(
                owner_hub.clone(),
                bot_id,
                &chat_conversation,
                Some(StatusTurn {
                    turn_id: lead.id.clone(),
                    host_id: self.config.host_id.clone(),
                }),
            )
            .handle(transcript.topic_at)
        });
        let owner_direct = audience == TurnAudience::Owner
            && !minted_by_hub
            && !continued
            && !ephemeral_user_context
            && lead
                .requester_id
                .as_deref()
                .is_none_or(|requester| requester == lead.owner_id);
        let trail = owner_direct.then(|| {
            let upto = entries
                .iter()
                .position(|entry| entry.message_id == lead.trigger_message_id)
                .map_or(entries.len(), |index| index + 1);
            ToolTrail::new(transcript::tool_trail(bot_id, &entries[..upto]))
        });
        let reply_files = (!ephemeral_user_context).then(ReplyFiles::new);
        let usage = flow_like::flow::copilot::bot_usage::BotUsage::default();
        if let Some(live) = lock(&self.state).live.get_mut(key) {
            live.notes = notes.clone();
            live.trail = trail.clone();
            live.reply_files = reply_files.clone();
            live.usage = usage.clone();
        }
        let mut job = TurnJob {
            usage,
            run_id: lead.id.clone(),
            bot_id: bot_id.clone(),
            conversation_id: chat_conversation,
            profile_id: None,
            prompt,
            acknowledgement: false,
            live_call: false,
            tool_gate: None,
            ephemeral_user_context,
            seed_taint: flow_like::flow::copilot::TurnReads {
                personal_context: ephemeral_user_context,
                ..crate::taint::reads_of(&entries, bot_id, &Default::default())
            }
            .seed(),
            shared_context: transcript::shared_context(&entries, &lead.trigger_message_id, bot_id),
            suppress_owner_notifications: false,
            history: transcript.history,
            room_updates,
            notes: notes.clone(),
            trail,
            reply_files,
            memory: memory.clone(),
            gate: gate.clone(),
            requester: Some(requester.clone()),
            hub_token,
            answered_round,
            claim,
            recovery,
            reply_stream: None,
        };
        gate.set_inference(Some(RunnerInference::new(
            self.runner.clone(),
            &job,
            grants.clone(),
            Arc::new(HubAuthority::new(
                owner_hub.clone(),
                bot_id,
                &lead.id,
                &lead.owner_id,
                self.e2e.get().cloned(),
            )),
        )));
        let call_utterance = entries.iter().any(|entry| {
            entry.message_id == lead.trigger_message_id && entry.kind == InboxEntryKind::Transcript
        });
        let explicit_ask = entries.iter().any(|entry| {
            entry.message_id == lead.trigger_message_id
                && entry.kind == InboxEntryKind::Message
                && entry.sender_id != lead.bot_id
                && entry.sender_id != lead.owner_id
                && crate::rule::is_plain(entry)
                && entry
                    .envelope
                    .get("body")
                    .and_then(|body| MessageBody::parse(body).ok())
                    .is_some_and(|body| {
                        body.mentions.contains(&lead.bot_id)
                            && body.ask.is_some_and(|ask| {
                                ask.id == entry.message_id
                                    && ask.chain.last().is_none_or(|id| id == &entry.sender_id)
                            })
                    })
        });
        let event = (saved_delivery.is_some()
            || (!resumed && !continued && !explicit_ask && !recovering))
            .then(|| crate::event::trigger(&entries, &lead.trigger_message_id))
            .flatten();
        if event.as_ref().is_some_and(decision_event::monitoring)
            && saved_delivery.is_none_or(|delivery| delivery.choice.is_none())
        {
            let previous = match self
                .hub
                .previous_monitor_result(bot_id, &lead.id, &self.config.host_id, lead.attempts)
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
                crate::event::baseline_note(previous.as_deref()),
                job.prompt
            );
        }
        let mut event_delivery = saved_delivery.and_then(|delivery| delivery.choice);
        if let Some(event) = &event
            && saved_delivery.is_none()
            && !resumed
            && !recovering
        {
            let classified = tokio::select! {
                biased;
                reason = cancelled(cancel) => return End::by(reason),
                result = tokio::time::timeout(Duration::from_secs(decision::TIMEOUT_SECS), crate::event::classify(self.runner.as_ref(), &job, event, None)) => result,
            };
            match classified {
                Ok(Ok(Some(EventDecision::Ignore))) => return self.stay_silent(lead).await,
                Ok(Ok(choice)) => event_delivery = choice,
                Ok(Err(RunError::SessionChanged)) => {
                    return End::dropped(crate::host::CODE_SESSION_CHANGED);
                }
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
        let event_recovery =
            delivery_state.and_then(|delivery| crate::event::attach_delivery(&mut job, delivery));
        // An explicit delegation already selected this bot to answer. Participation decisions
        // must not silently drop that answer or wait for the caller that is waiting on it.
        trace.transcript = call_utterance;
        job.live_call = call_utterance;
        let deadline =
            crate::recovery::remaining_deadline(job.recovery.as_ref(), self.config.deadline);
        let mut parallel: Option<(ToolGateOpener, RunFuture<'_>)> = None;
        let mut early: Option<RunOutcome> = None;
        let mut reply = None;
        if (kind == ConversationKind::Group || call_utterance)
            && event.is_none()
            && !continued
            && !explicit_ask
            && !resumed
            && !recovering
        {
            if call_utterance && requester.addressed {
                let (opener, tool_gate) = tool_gate();
                job.tool_gate = Some(tool_gate);
                trace.parallel = true;
                trace.run_started();
                reply = live_reply::open(self.events.as_ref(), &mut job, &lead.trigger_message_id);
                parallel = Some((
                    opener,
                    Box::pin(tokio::time::timeout(deadline, self.runner.run(job.clone()))),
                ));
            }
            trace.decision_started();
            let decide = async {
                let room = match room {
                    Some(room) => room,
                    None => hub
                        .read(
                            &format!("inbox/conversations/{conversation_id}"),
                            vec![("as", bot_id.clone())],
                        )
                        .await
                        .map_err(|error| RunError::Retry {
                            code: CODE_HUB.into(),
                            message: error.to_string(),
                        })?,
                };
                let participants = room
                    .get("members")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    // The hub cannot recognize an explicit override inside a sealed message.
                    // Omit participant waits until their routing can stay on the decrypting host.
                    .filter(|_| !sealed)
                    .filter(|member| {
                        member
                            .get("status")
                            .and_then(serde_json::Value::as_str)
                            .is_none_or(|status| status.eq_ignore_ascii_case("active"))
                    })
                    .filter_map(|member| {
                        let id = member.get("principal_id")?.as_str()?;
                        Some(DecisionParticipant {
                            id: id.to_owned(),
                            name: member
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .or_else(|| names.get(id).map(String::as_str))
                                .unwrap_or(id)
                                .to_owned(),
                            kind: member.get("kind")?.as_str()?.to_owned(),
                        })
                    })
                    .collect::<Vec<_>>();
                let request =
                    participation_request(&job, &lead.trigger_message_id, &entries, &participants)
                        .map_err(|error| RunError::Failed {
                            code: CODE_SETUP.into(),
                            message: error.to_string(),
                        })?;
                self.runner.decide(&job, &request).await
            };
            let mut decide = Box::pin(tokio::time::timeout(
                Duration::from_secs(decision::TIMEOUT_SECS),
                decide,
            ));
            let decision = loop {
                tokio::select! {
                    biased;
                    reason = cancelled(cancel) => {
                        if parallel.is_some() {
                            self.runner.cancel(&lead.id);
                        }
                        return End::by(reason);
                    }
                    result = &mut decide => break result,
                    Some(outcome) = async {
                        match parallel.as_mut() {
                            Some((_, run)) if early.is_none() => Some(run.as_mut().await),
                            _ => std::future::pending().await,
                        }
                    } => {
                        trace.run_finished();
                        early = Some(outcome);
                    }
                }
            };
            drop(decide);
            let decision = decision.map(|result| {
                result.map(|choice| choice.answering_addressed(requester.addressed, call_utterance))
            });
            trace.decided(latency::decision_label(&decision));
            let answers = matches!(&decision, Ok(Ok(Decision::AnswerNow)) | Err(_) | Ok(Err(_)))
                && !matches!(&decision, Ok(Err(RunError::SessionChanged)));
            match parallel.take() {
                Some((opener, run)) if answers => {
                    opener.open();
                    parallel = Some((opener, run));
                }
                Some((opener, run)) => {
                    opener.shut();
                    drop(run);
                    self.runner.cancel(&lead.id);
                    job.tool_gate = None;
                    early = None;
                }
                None => {}
            }
            match decision {
                Ok(Ok(Decision::NoAnswer)) => return self.stay_silent(lead).await,
                Ok(Ok(Decision::WaitFor(participant))) => {
                    if kind != ConversationKind::Group {
                        return self.stay_silent(lead).await;
                    }
                    match self
                        .hub
                        .wait_for(&lead.id, &self.config.host_id, &participant)
                        .await
                    {
                        Ok(WaitOutcome::Parked) => {
                            return End {
                                status: TurnStatus::Done,
                                error_code: flow_like_types::bot_turn::ERROR_SILENT.to_owned(),
                                reply: None,
                            };
                        }
                        Ok(WaitOutcome::AlreadyAnswered) => {}
                        Ok(WaitOutcome::Unavailable) => {}
                        Ok(WaitOutcome::Superseded) => return self.stay_silent(lead).await,
                        Err(error) if error.lease_lost() => return End::lease_lost(),
                        Err(error) => {
                            tracing::warn!(%error, bot_id, "Bot host: participant wait could not be saved");
                            return self.abandon(lead, CODE_SETUP).await;
                        }
                    }
                }
                Ok(Err(RunError::SessionChanged)) => {
                    return End::dropped(crate::host::CODE_SESSION_CHANGED);
                }
                Ok(Err(_)) | Err(_) => tracing::warn!(
                    bot_id,
                    "Bot host: decision model unavailable; using normal response behavior"
                ),
                Ok(Ok(Decision::Acknowledge)) => job.acknowledgement = call_utterance,
                Ok(Ok(Decision::AnswerNow)) => {}
            }
        }
        let acknowledgement = job.acknowledgement;
        let event_job = delivery_state.map(|_| job.clone());
        let approval_recovery = job.recovery.clone();
        let (_opener, mut run) = match parallel.take() {
            Some((opener, run)) => (Some(opener), run),
            None => {
                trace.run_started();
                reply = live_reply::open(self.events.as_ref(), &mut job, &lead.trigger_message_id);
                let run: RunFuture<'_> =
                    Box::pin(tokio::time::timeout(deadline, self.runner.run(job)));
                (None, run)
            }
        };
        let outcome = match early {
            Some(outcome) => outcome,
            None => {
                let outcome = tokio::select! {
                    biased;
                    reason = cancelled(cancel) => {
                        self.runner.cancel(&lead.id);
                        return End::by(reason);
                    }
                    outcome = &mut run => outcome,
                };
                trace.run_finished();
                outcome
            }
        };
        live_reply::pause(reply.as_ref(), &outcome);
        if let Some(card) = gate.parked() {
            if let Err(error) =
                crate::recovery::preserve_held_batch(approval_recovery.as_ref(), &gate).await
            {
                return self
                    .retry_or_fail(
                        key,
                        lead,
                        &hub,
                        CODE_SETUP,
                        &format!("The pending approval could not be saved: {error}"),
                        sealed,
                        cancel,
                    )
                    .await;
            }
            return self.park(key, lead, &hub, &card, sealed, cancel).await;
        }
        let failed_execution = if let Some(job) = &event_job {
            crate::event::failed_execution(job).await
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
                result = tokio::time::timeout(Duration::from_secs(decision::TIMEOUT_SECS), crate::event::classify(self.runner.as_ref(), job, event, Some(text))) => result,
            };
            match classified {
                Ok(Ok(choice)) => event_delivery = Some(choice.unwrap_or(EventDecision::Notify)),
                Ok(Err(RunError::SessionChanged)) => {
                    return End::dropped(crate::host::CODE_SESSION_CHANGED);
                }
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
                    lead,
                    &hub,
                    &format!(
                        "The event completed, but its delivery state could not be saved: {error}"
                    ),
                    Some(CODE_SETUP),
                    sealed,
                    cancel,
                )
                .await;
        }
        if matches!(&outcome, Ok(Ok(_))) && event_delivery.is_some_and(EventDecision::quiet) {
            let end = self.stay_silent(lead).await;
            if end.status == TurnStatus::Done {
                clear_tokens(grants.as_ref(), &seen).await;
                if let Some(JobMemory(source)) = memory {
                    follow_up(bot_id, source);
                }
            }
            return end;
        }
        match outcome {
            Ok(Ok(text))
                if requester.answer_is_silent(&text)
                    || (acknowledgement
                        && flow_like::flow::copilot::bot_run::is_silent_reply(&text)) =>
            {
                self.stay_silent(lead).await
            }
            Ok(Err(RunError::SessionChanged)) => End::dropped(crate::host::CODE_SESSION_CHANGED),
            Ok(Err(_)) if event.is_none() && requester.may_stay_silent() => {
                self.stay_silent(lead).await
            }
            Ok(Err(RunError::Retry { code, message })) => {
                self.retry_or_fail(key, lead, &hub, &code, &message, sealed, cancel)
                    .await
            }
            Err(_) if event.is_none() && requester.may_stay_silent() => {
                self.runner.cancel(&lead.id);
                self.stay_silent(lead).await
            }
            Ok(Ok(text)) if !text.trim().is_empty() => {
                let end = self
                    .post_with_delivery(
                        key,
                        lead,
                        &hub,
                        &clipped(&text, MAX_REPLY_CHARS),
                        None,
                        event_delivery,
                        acknowledgement,
                        sealed,
                        cancel,
                    )
                    .await;
                let posted = end.status == TurnStatus::Done && end.reply.is_some();
                live_reply::finish(reply.take(), posted).await;
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
                let text = "The language model returned no answer. You can ask again.";
                self.post(key, lead, &hub, text, Some(CODE_MODEL), sealed, cancel)
                    .await
            }
            Ok(Err(RunError::Failed { code, message })) => {
                let text = format!(
                    "I couldn't answer this: {}",
                    clipped(&message, MAX_FAILURE_DETAIL_CHARS)
                );
                self.post(key, lead, &hub, &text, Some(&code), sealed, cancel)
                    .await
            }
            Err(_) => {
                self.runner.cancel(&lead.id);
                let text = "I ran out of time before I could answer. You can ask again.";
                self.post(key, lead, &hub, text, Some(CODE_TIMEOUT), sealed, cancel)
                    .await
            }
        }
    }

    /// A failure no tool with side effects preceded: the hub takes the turn back for another
    /// attempt while it has attempts left, else the failure is reported to the conversation.
    async fn retry_or_fail(
        &self,
        key: &Key,
        lead: &BotTurn,
        hub: &Arc<dyn TurnHub>,
        code: &str,
        message: &str,
        sealed: bool,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        // Claim generations keep increasing across approvals and manual retries. The hub owns
        // the separate recovery budget and decides whether another retry is available.
        match self.hub.release(&lead.id, &self.config.host_id, code).await {
            Ok(true) => return End::dropped(code),
            Ok(false) => {}
            Err(failure) if failure.lease_lost() => return End::lease_lost(),
            Err(failure) => {
                tracing::warn!(%failure, turn_id = %lead.id, "Bot host: releasing a failed turn for retry failed");
            }
        }
        let text = format!(
            "I couldn't answer this: {}",
            clipped(message, MAX_FAILURE_DETAIL_CHARS)
        );
        self.post(key, lead, hub, &text, Some(code), sealed, cancel)
            .await
    }

    /// A hub error before any tool ran: the turn goes back to the hub for another attempt; with
    /// its retries spent or no release route it ends failed without a reply.
    async fn release_or_abandon(&self, lead: &BotTurn, code: &'static str) -> End {
        match self.hub.release(&lead.id, &self.config.host_id, code).await {
            Ok(true) => End::dropped(code),
            Ok(false) => self.abandon(lead, code).await,
            Err(failure) if failure.lease_lost() => End::lease_lost(),
            Err(failure) => {
                tracing::debug!(%failure, turn_id = %lead.id, "Bot host: releasing a turn after a hub error failed");
                self.abandon(lead, code).await
            }
        }
    }

    /// Ends a turn that can never run on this host failed, without a reply.
    async fn abandon(&self, lead: &BotTurn, code: &'static str) -> End {
        match self.hub.abandon(&lead.id, &self.config.host_id, code).await {
            Ok(()) => End::failed(code),
            Err(failure) if failure.lease_lost() => End::lease_lost(),
            Err(failure) => {
                tracing::warn!(%failure, turn_id = %lead.id, "Bot host: the hub did not record an abandoned turn");
                End::failed(code)
            }
        }
    }

    /// Ends a turn that read along without answering: done, nothing posted.
    async fn stay_silent(&self, lead: &BotTurn) -> End {
        match self.hub.silent(&lead.id, &self.config.host_id).await {
            Ok(()) => End {
                status: TurnStatus::Done,
                error_code: flow_like_types::bot_turn::ERROR_SILENT.to_owned(),
                reply: None,
            },
            Err(failure) if failure.lease_lost() => End::lease_lost(),
            Err(failure) => {
                tracing::warn!(%failure, "Bot host: the hub did not record a silent turn");
                self.abandon(lead, CODE_HUB).await
            }
        }
    }

    /// The turn a late answer woke reads the conversation the question was asked from, with the
    /// answer on top, and goes on counting its questions to the callee; `None` for any other turn.
    async fn late_answer(
        &self,
        lead: &BotTurn,
        hub: &Arc<dyn TurnHub>,
    ) -> Result<Option<LateContinuation>, HubFailure> {
        let Some(asked) = hub.ask_result(&lead.bot_id, &lead.id).await? else {
            return Ok(None);
        };
        let mut origin = hub
            .entries(&lead.bot_id, &asked.origin_conversation)
            .await?;
        origin.reverse();
        let conversation = hub
            .read(
                &format!("inbox/conversations/{}", asked.origin_conversation),
                Vec::new(),
            )
            .await
            .ok();
        let owner_direct = conversation.as_ref().is_some_and(|conversation| {
            memory::hub_owner_direct(conversation, &lead.bot_id)
                && conversation["members"].as_array().is_some_and(|members| {
                    members.iter().any(|member| {
                        member["principal_id"].as_str() == Some(lead.owner_id.as_str())
                    })
                })
        });
        let kind = if owner_direct {
            ConversationKind::Direct
        } else {
            ConversationKind::Group
        };
        let history = transcript::continuation_history(
            &lead.bot_id,
            kind,
            &origin,
            &HashMap::new(),
            &lead.owner_id,
        );
        let transcript = transcript::Transcript {
            prompt: flow_like_types::bot_message::ask_result_prompt(
                &asked.callee_name,
                &asked.question,
                &asked.answer,
                asked.failed,
                None,
            ),
            history,
            addressed: true,
            topic_at: transcript::topic_at(&origin),
        };
        let answered_round = asked.answered_round();
        Ok(Some(LateContinuation {
            conversation_id: asked.origin_conversation,
            kind,
            transcript,
            answered_round,
        }))
    }

    /// Asks the hub to park the turn this host holds; the hub posts the card as the bot.
    async fn park(
        &self,
        key: &Key,
        lead: &BotTurn,
        hub: &Arc<dyn TurnHub>,
        card: &ApprovalRequest,
        sealed: bool,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        if let Some(reason) = *cancel.borrow() {
            return End::by(reason);
        }
        match hub.park(&lead.id, &self.config.host_id, card).await {
            Ok(()) => End {
                status: TurnStatus::Waiting,
                error_code: String::new(),
                reply: None,
            },
            Err(failure) if failure.lease_lost() => End::lease_lost(),
            Err(failure) => {
                tracing::warn!(%failure, "Bot host: the hub did not park an account turn");
                let text = format!(
                    "I needed your approval to run {} but could not ask you: {}",
                    crate::gate::card_tools(card),
                    clipped(&failure.message, MAX_FAILURE_DETAIL_CHARS)
                );
                self.post(key, lead, hub, &text, Some(CODE_SETUP), sealed, cancel)
                    .await
            }
        }
    }

    async fn post(
        &self,
        key: &Key,
        lead: &BotTurn,
        hub: &Arc<dyn TurnHub>,
        text: &str,
        failure: Option<&str>,
        sealed: bool,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        self.post_with_delivery(key, lead, hub, text, failure, None, false, sealed, cancel)
            .await
    }

    async fn post_with_delivery(
        &self,
        key: &Key,
        lead: &BotTurn,
        hub: &Arc<dyn TurnHub>,
        text: &str,
        failure: Option<&str>,
        event_delivery: Option<EventDecision>,
        acknowledgement: bool,
        sealed: bool,
        cancel: &mut watch::Receiver<Option<Cancel>>,
    ) -> End {
        let (note, trail, files, usage);
        {
            let mut state = lock(&self.state);
            let Some(live) = state.live.get_mut(key) else {
                return End::lease_lost();
            };
            if let Some(reason) = *cancel.borrow() {
                return End::by(reason);
            }
            live.posting = true;
            note = live.notes.as_ref().and_then(NotesHandle::touched);
            trail = live.trail.clone();
            files = live.reply_files.clone();
            usage = live.usage.clone();
        }
        let message_id = if failure.is_some() {
            failure_message_id(&lead.bot_id, &lead.trigger_message_id)
        } else {
            reply_message_id(&lead.bot_id, &lead.trigger_message_id)
        };
        let mut body = reply_body(&lead.trigger_message_id, lead.hops, text, failure);
        if failure.is_none() && !acknowledgement {
            crate::notes::attach_note(&mut body, note.as_ref());
            if let Some(files) = &files {
                files.attach(&mut body);
            }
            if let Some(trail) = &trail {
                trail.attach(&mut body);
            }
        }
        if failure.is_none() && acknowledgement {
            body[decision::ACKNOWLEDGEMENT_FIELD] = json!(true);
        }
        if failure.is_none() && event_delivery == Some(EventDecision::Respond) {
            body[decision_event::REPLY_DELIVERY_FIELD] = json!("respond");
        }
        if lead.conversation_id != key.1 {
            // The trigger belongs to the bot exchange and is not a message in the origin room.
            if let Some(body) = body.as_object_mut() {
                body.remove("reply_to");
            }
        }
        usage.attach(&mut body);
        let message = if sealed {
            match self.sealed_message(lead, &message_id, &body, failure).await {
                Ok(message) => message,
                Err(failure) => {
                    tracing::warn!(code = failure.code, message = %failure.message, "Bot host: an encrypted reply could not be sealed");
                    return self.abandon(lead, failure.code).await;
                }
            }
        } else {
            json!({
                "message_id": message_id,
                "kind": "message",
                "envelope": plain_envelope(body),
            })
        };
        for attempt in 0..=self.config.send_retries {
            let sent = hub
                .send(
                    &lead.bot_id,
                    &lead.conversation_id,
                    message.clone(),
                    &lead.id,
                    &self.config.host_id,
                )
                .await;
            match sent {
                Ok(()) => {
                    return End {
                        status: if failure.is_some() {
                            TurnStatus::Failed
                        } else {
                            TurnStatus::Done
                        },
                        error_code: failure.unwrap_or_default().to_owned(),
                        reply: Some(message_id),
                    };
                }
                Err(failure) if failure.lease_lost() => return End::lease_lost(),
                Err(failure) if failure.status.is_none() && attempt < self.config.send_retries => {
                    tracing::debug!(%failure, "Bot host: posting an account reply failed; retrying");
                    tokio::time::sleep(self.config.send_retry_delay).await;
                }
                Err(failure) => {
                    tracing::warn!(%failure, "Bot host: the hub refused an account reply");
                    return self.abandon(lead, CODE_REPLY_REFUSED).await;
                }
            }
        }
        self.abandon(lead, CODE_REPLY_REFUSED).await
    }

    async fn open_sealed(
        &self,
        bot_id: &str,
        trigger_message_id: &str,
        entries: Vec<InboxEntry>,
    ) -> Result<Vec<InboxEntry>, e2e::E2eFailure> {
        let Some(e2e) = self.e2e.get() else {
            return Err(e2e::E2eFailure {
                code: e2e::CODE_E2E_UNAVAILABLE,
                message: "End-to-end encryption is not enabled on this host".into(),
            });
        };
        e2e.open_entries(bot_id, trigger_message_id, entries).await
    }

    /// The sealed send of `body`. A failure report names its code beside the envelopes, because
    /// the hub records why the turn failed and can't read the sealed body.
    async fn sealed_message(
        &self,
        lead: &BotTurn,
        message_id: &str,
        body: &serde_json::Value,
        failure: Option<&str>,
    ) -> Result<serde_json::Value, e2e::E2eFailure> {
        let Some(e2e) = self.e2e.get() else {
            return Err(e2e::E2eFailure {
                code: e2e::CODE_E2E_UNAVAILABLE,
                message: "End-to-end encryption is not enabled on this host".into(),
            });
        };
        let envelopes = e2e
            .seal(&lead.bot_id, &lead.conversation_id, message_id, body)
            .await?;
        let mut message = json!({
            "message_id": message_id,
            "kind": "message",
            "envelopes": envelopes,
        });
        if let Some(code) = failure {
            message["failure_code"] = json!(code);
        }
        Ok(message)
    }

    fn start(&self, key: &Key, lead: &BotTurn) -> Arc<watch::Sender<Option<Cancel>>> {
        let (info, cancel) = {
            let mut state = lock(&self.state);
            let live = state.live.get_mut(key).expect("a claimed turn is live");
            let queued_at = live.info.queued_at.clone();
            live.info = info_of(lead, TurnStatus::Running);
            live.info.queued_at = queued_at;
            live.info.started_at = Some(now());
            (live.info.clone(), live.cancel.clone())
        };
        self.events.turn(&info);
        cancel
    }

    fn finish(&self, key: &Key, end: End) {
        let info = {
            let mut state = lock(&self.state);
            let Some(mut live) = state.live.remove(key) else {
                return;
            };
            live.info.status = end.status;
            live.info.finished_at = Some(now());
            live.info.error_code = (!end.error_code.is_empty()).then_some(end.error_code);
            live.info.reply_message_id = end.reply;
            live.info.status_text = None;
            state.finished.push_back(live.info.clone());
            while state.finished.len() > FINISHED_KEPT {
                state.finished.pop_front();
            }
            live.info
        };
        self.events.turn(&info);
    }
}

fn info_of(turn: &BotTurn, status: TurnStatus) -> TurnInfo {
    TurnInfo {
        id: turn.id.clone(),
        bot_id: turn.bot_id.clone(),
        conversation_id: turn.conversation_id.clone(),
        trigger_message_id: turn.trigger_message_id.clone(),
        status,
        hops: turn.hops,
        requester_id: turn.requester_id.clone().unwrap_or_default(),
        audience: turn.audience,
        batched: u32::try_from(turn.batched.len()).unwrap_or(u32::MAX),
        queued_at: now(),
        started_at: None,
        finished_at: None,
        error_code: None,
        reply_message_id: None,
        status_text: None,
    }
}
