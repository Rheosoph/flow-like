//! Account bots asking each other. The exchange between two bots is an inbox group with their
//! owner; a question is a message of the asking bot that mentions the other and carries
//! `ask{id, origin_conversation, hops, reply_by, chain, wait_secs}`, which queues the callee's
//! turn on the callee's host through the ledger. A server callee runs inline in the asker's
//! process; any other waits for its host. The asker waits for the callee's turn (polling) and
//! moves on with `pending` when the answer is late or the callee is offline; the late answer
//! then wakes the asker with a turn in the conversation it asked from.

use super::{
    approvals::{self, GrantRows},
    history::{self, HistoryError, Transcript},
    ledger,
};
use crate::{
    entity::{bot, bot_permission, bot_turn, conversation, inbox_entry},
    error::ApiError,
    routes::{
        bots::{
            fire::{FiredTurn, wait_for_turn},
            permissions::grant_in,
            presence::{PresenceState, presence_in},
        },
        inbox::{
            activity::record_bot_ask,
            approvals::{lock as lock_conversation, open_conversation, post_notice_in},
            delivery::BotRecipient,
            exchange::{message_outgoing, open_exchange, post_as_bot},
            now_millis, wake,
        },
    },
    state::AppState,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset};
use flow_like::flow::copilot::{
    FLUX_BOT_NAME, GENERAL_BOT_NAME,
    bot_talk::{
        BotPresence, BotTalkHost, CrewMember, HeadlessPermission, MAX_ASK_HOPS, NestedBotTurn,
        PendingAnswer,
    },
    tool_grants::{GrantCheck, GrantStore, approval_request, check_call},
};
use flow_like_types::{
    bot::{BotLocation, BotSystemKind},
    bot_message::{ApprovalRequest, MessageBody},
    bot_turn::{MAX_HOPS, STORED_QUEUED, turn_id},
    data_classification::{DataClassification, DataSensitivity},
    inbox::{ActivityBody, ConversationKind, TOMBSTONE_FIELD},
    tokio,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::Expr,
};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long a server bot waits for the answer to a question.
pub(crate) const WAIT_SECS: u32 = 90;
/// A late answer wakes the asker this long after its wait ended, so a poll that is about to read
/// the answer wins.
const ANSWER_GRACE_SECS: i64 = 5;
/// A question nobody picked up lapses after this long.
pub(crate) const REPLY_BY_DAYS: i64 = 7;
pub(crate) const ERROR_ASK_QUEUED: &str = "BOT_ASK_QUEUED";
pub(crate) const ERROR_ASK_EXPIRED: &str = "BOT_ASK_EXPIRED";
pub(crate) const ERROR_ASK_ANSWERED: &str = "BOT_ASK_ANSWERED";
const GRANT_TOOL: &str = "ask_bot";
const QUESTION_PREVIEW_CHARS: usize = 300;
const EXPIRE_BATCH: u64 = 50;

pub(crate) fn grant_scope(callee_id: &str) -> String {
    format!("bot:{callee_id}")
}

/// The conversation a desktop-hosted turn of `bot_id` (its `run_id` is the turn id) continues;
/// a late answer continues its original conversation. A chat run has no turn and returns `None`.
pub(crate) async fn turn_conversation<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    run_id: &str,
) -> Result<Option<String>, ApiError> {
    if run_id.is_empty() {
        return Ok(None);
    }
    let Some(turn) = bot_turn::Entity::find_by_id(run_id)
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    turn_origin_conversation(db, &turn).await.map(Some)
}

/// Where a turn's further questions should return, including a turn woken by a late answer.
pub(crate) async fn turn_origin_conversation<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<String, ApiError> {
    Ok(answered_ask(db, turn).await?.map_or_else(
        || turn.conversation_id.clone(),
        |answered| answered.origin_conversation,
    ))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoteTalkContext {
    pub ancestors: Vec<String>,
    pub hops: u32,
}

/// A remote host's ask keeps the ancestry and hop count of its inbox turn. An unknown run id
/// is an interactive chat; an existing turn must belong to the caller and its owner.
pub(crate) async fn remote_context<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    caller_id: &str,
    callee_id: &str,
    run_id: &str,
) -> Result<RemoteTalkContext, ApiError> {
    let Some(turn) = bot_turn::Entity::find_by_id(run_id).one(db).await? else {
        return Ok(RemoteTalkContext::default());
    };
    if turn.owner_id != owner_id || turn.bot_id != caller_id {
        return Err(ApiError::not_found(
            "The asking turn does not belong to this bot",
        ));
    }
    let ancestors: Vec<String> = asked_by(db, &turn)
        .await?
        .into_iter()
        .map(|member| member.id)
        .collect();
    if ancestors.iter().any(|ancestor| ancestor == callee_id) {
        return Err(ApiError::bad_request(
            "This bot is already in the question's chain; asking it would create a cycle",
        ));
    }
    if ancestors.len() >= MAX_ASK_HOPS {
        return Err(ApiError::bad_request(format!(
            "A question may pass at most {MAX_ASK_HOPS} bots; answer from the available information"
        )));
    }
    let hops = u32::try_from(turn.hops).unwrap_or_default();
    if hops >= MAX_HOPS {
        return Err(ApiError::bad_request(format!(
            "This run reached the bot message hop limit ({MAX_HOPS}); no further bot question was sent"
        )));
    }
    Ok(RemoteTalkContext { ancestors, hops })
}

fn clip(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", text[..index].trim_end()),
        None => text.to_string(),
    }
}

fn presence_of(state: PresenceState) -> BotPresence {
    match state {
        PresenceState::Online => BotPresence::Online,
        PresenceState::Offline => BotPresence::Offline,
        PresenceState::Unknown => BotPresence::Unknown,
    }
}

/// A bot row as another bot's crew roster shows it.
pub(crate) fn crew_member(row: &bot::Model, presence: BotPresence) -> CrewMember {
    let system = row.system_key.as_deref().and_then(BotSystemKind::parse);
    let name = row
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match system {
            Some(BotSystemKind::General) => GENERAL_BOT_NAME.to_string(),
            Some(BotSystemKind::Flux) => FLUX_BOT_NAME.to_string(),
            None => row.id.clone(),
        });
    CrewMember {
        id: row.id.clone(),
        name,
        description: row
            .description
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        system,
        location: BotLocation::parse(&row.location).unwrap_or_default(),
        presence,
        classification: DataSensitivity {
            classification: row
                .classification
                .as_deref()
                .and_then(DataClassification::parse),
            pii: row.pii,
        },
    }
}

/// The owner's other live bots with their presence.
pub(crate) async fn roster_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    caller_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<CrewMember>, ApiError> {
    let presence = presence_in(db, owner_id, now).await?;
    let rows = bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .filter(bot::Column::Id.ne(caller_id))
        .order_by_asc(bot::Column::CreatedAt)
        .order_by_asc(bot::Column::Id)
        .all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let state = presence
                .get(&row.id)
                .map_or(PresenceState::Unknown, |presence| presence.state);
            crew_member(row, presence_of(state))
        })
        .collect())
}

/// When a host last reported the bot, as the roster's offline note reads it.
async fn last_seen<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Option<String> {
    presence_in(db, owner_id, now)
        .await
        .ok()?
        .get(bot_id)?
        .last_seen_at
        .clone()
}

/// Whether `from` may ask `to` without asking the owner: a stored permission, or the standing
/// answer an approval card left, which becomes a permission here.
pub(crate) async fn granted_in<C: ConnectionTrait + Send + Sync>(
    db: &C,
    owner_id: &str,
    from: &str,
    to: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let stored = bot_permission::Entity::find()
        .filter(bot_permission::Column::OwnerId.eq(owner_id))
        .filter(bot_permission::Column::FromBotId.eq(from))
        .filter(bot_permission::Column::ToBotId.eq(to))
        .one(db)
        .await?
        .is_some();
    if stored {
        return Ok(true);
    }
    let scope = grant_scope(to);
    let card_answer = GrantRows::new(db, from, owner_id)
        .has(GRANT_TOOL, &scope)
        .await
        .unwrap_or(false);
    if !card_answer {
        return Ok(false);
    }
    grant_in(db, owner_id, from, to, now).await?;
    approvals::delete_in(db, from, GRANT_TOOL, &scope).await?;
    Ok(true)
}

fn ask_message_id(run_id: &str, callee_id: &str, question: &str) -> String {
    let digest = blake3::hash(format!("{run_id}\0{callee_id}\0{question}").as_bytes()).to_hex();
    format!("ask-{}", &digest[..32])
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Question<'a> {
    pub text: &'a str,
    pub callee_id: &'a str,
    pub ask_id: &'a str,
    pub origin_conversation: &'a str,
    pub hops: u32,
    pub chain: &'a [String],
    pub wait_secs: u32,
    /// The asker's question number to this callee for the trigger; 0 from askers that predate it.
    pub round: u32,
}

pub(crate) fn ask_body(question: &Question, now: DateTime<FixedOffset>) -> Value {
    let reply_by = now + ChronoDuration::days(REPLY_BY_DAYS);
    let mut ask = json!({
        "id": question.ask_id,
        "origin_conversation": question.origin_conversation,
        "hops": question.hops,
        "reply_by": reply_by.to_rfc3339(),
        "chain": question.chain,
    });
    if question.wait_secs > 0 {
        ask["wait_secs"] = json!(question.wait_secs);
    }
    if question.round > 0 {
        ask["round"] = json!(question.round);
    }
    json!({
        "text": question.text,
        "mentions": [question.callee_id],
        "hops": question.hops,
        "ask": ask,
    })
}

/// What a run that cannot ask the owner does about a first question: the owner's answer to an
/// earlier card decides, otherwise the turn parks behind a new card.
pub(crate) async fn headless_verdict(
    store: &dyn GrantStore,
    turn_id: &str,
    caller: &CrewMember,
    callee: &CrewMember,
    question: &str,
) -> (HeadlessPermission, Option<ApprovalRequest>) {
    let args = json!({ "bot": callee.id, "message": question });
    match check_call(store, GRANT_TOOL, &args, None).await {
        Ok(check) if check.runs() => (HeadlessPermission::Allowed, None),
        Ok(GrantCheck::Denied) => (HeadlessPermission::Denied, None),
        Ok(_) => {
            let summary = format!(
                "{} wants to ask {}: {}",
                caller.name,
                callee.name,
                clip(question, QUESTION_PREVIEW_CHARS)
            );
            let mut card = approval_request(turn_id, GRANT_TOOL, &args, None, &summary);
            card.scope = grant_scope(&callee.id);
            (HeadlessPermission::Parked, Some(card))
        }
        Err(_) => (HeadlessPermission::Unavailable, None),
    }
}

/// The turn a server host holds while its bot asks.
#[derive(Clone)]
pub(crate) struct HeldTurn {
    pub turn: bot_turn::Model,
    pub host_id: String,
}

#[derive(Default)]
struct Asked {
    callee: Option<CrewMember>,
    ask_id: Option<String>,
    reply_id: Option<String>,
}

/// The talk host of a server bot's run: its chat with the owner or one of its headless turns.
pub struct ServerTalkHost {
    state: AppState,
    owner_id: String,
    run_id: String,
    held: Option<HeldTurn>,
    ancestors: Vec<String>,
    trigger_hops: u32,
    parked: Mutex<Option<(ApprovalRequest, Option<String>)>>,
    asked: Mutex<Asked>,
}

impl ServerTalkHost {
    /// A chat run of `run_id`; its late answers return to the owner's direct conversation, or to
    /// the conversation of the desktop-hosted turn `run_id` names.
    pub fn for_chat(state: AppState, owner_id: &str, run_id: &str) -> Self {
        Self::new(state, owner_id, run_id, None, Vec::new())
    }

    /// An ask made by another host carries the validated turn context without claiming its lease.
    pub(crate) fn for_remote(
        state: AppState,
        owner_id: &str,
        run_id: &str,
        context: RemoteTalkContext,
    ) -> Self {
        let mut host = Self::new(state, owner_id, run_id, None, context.ancestors);
        host.trigger_hops = context.hops;
        host
    }

    /// A headless turn the host holds. `ancestors` are the bots waiting on the ask that started
    /// it, the first asker first.
    pub(crate) fn for_turn(state: AppState, held: HeldTurn, ancestors: Vec<String>) -> Self {
        let owner = held.turn.owner_id.clone();
        let run = held.turn.id.clone();
        Self::new(state, &owner, &run, Some(held), ancestors)
    }

    fn new(
        state: AppState,
        owner_id: &str,
        run_id: &str,
        held: Option<HeldTurn>,
        ancestors: Vec<String>,
    ) -> Self {
        let trigger_hops = held
            .as_ref()
            .map_or(0, |held| u32::try_from(held.turn.hops).unwrap_or_default());
        Self {
            state,
            owner_id: owner_id.to_string(),
            run_id: run_id.to_string(),
            held,
            ancestors,
            trigger_hops,
            parked: Mutex::default(),
            asked: Mutex::default(),
        }
    }

    /// The card the run needs answered before its turn goes on; the worker parks the turn behind
    /// it instead of posting a reply.
    pub(crate) fn parked_card(&self) -> Option<ApprovalRequest> {
        self.parked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|(card, _)| card.clone())
    }

    pub(crate) fn parked_recovery_call_id(&self) -> Option<String> {
        self.parked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|(_, call_id)| call_id.clone())
    }

    fn remember<T>(&self, change: impl FnOnce(&mut Asked) -> T) -> T {
        change(&mut self.asked.lock().unwrap_or_else(|p| p.into_inner()))
    }

    async fn origin_conversation(
        &self,
        caller: &CrewMember,
        exchange_id: &str,
    ) -> flow_like_types::Result<String> {
        if let Some(held) = &self.held {
            return turn_origin_conversation(&self.state.db, &held.turn)
                .await
                .map_err(|error| flow_like_types::anyhow!("{error}"));
        }
        if let Some(origin) =
            turn_conversation(&self.state.db, &self.owner_id, &caller.id, &self.run_id)
                .await
                .map_err(|error| flow_like_types::anyhow!("{error}"))?
        {
            return Ok(origin);
        }
        let direct =
            open_conversation(&self.state.db, &self.owner_id, &caller.id, "", now_millis())
                .await
                .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        Ok(direct.unwrap_or_else(|| exchange_id.to_string()))
    }

    fn hops(&self) -> u32 {
        self.trigger_hops.saturating_add(1)
    }

    async fn wait_for_answer(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        tid: &str,
        started: Instant,
    ) -> flow_like_types::Result<String> {
        let remaining = Duration::from_secs(u64::from(WAIT_SECS)).saturating_sub(started.elapsed());
        let state = wait_for_turn(&self.state.db, &self.owner_id, tid, remaining)
            .await
            .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        let (reply_id, text) = answer_from(&state, callee)?;
        self.settle_answered(caller, &reply_id).await;
        self.remember(|asked| asked.reply_id = Some(reply_id));
        Ok(text)
    }

    async fn settle_answered(&self, caller: &CrewMember, reply_id: &str) {
        let follow_up = turn_id(&caller.id, reply_id);
        if let Err(error) =
            ledger::cancel_in(&self.state.db, &follow_up, ERROR_ASK_ANSWERED, now_millis()).await
        {
            tracing::warn!(%error, turn_id = %follow_up, "Bot talk: the answered ask's follow-up turn was not cancelled");
        }
    }
}

/// The reply id and text of a settled turn; a failed turn is an error, a turn still working is
/// [`PendingAnswer`].
pub(crate) fn answer_from(
    state: &FiredTurn,
    callee: &CrewMember,
) -> flow_like_types::Result<(String, String)> {
    if let Some(reply) = &state.reply {
        if state.status == "failed" || (state.error_code.is_some() && state.status != "done") {
            return Err(flow_like_types::anyhow!(
                "{} could not answer ({}): {}",
                callee.name,
                state.error_code.as_deref().unwrap_or("failed"),
                reply.text
            ));
        }
        return Ok((reply.message_id.clone(), reply.text.clone()));
    }
    match state.status.as_str() {
        "failed" | "cancelled" => Err(flow_like_types::anyhow!(
            "{} could not answer ({})",
            callee.name,
            state.error_code.as_deref().unwrap_or(&state.status)
        )),
        _ => Err(PendingAnswer(format!(
            "{0} hasn't answered within {1} seconds. Your question waits in its inbox; when {0} answers, you are woken in this conversation with the answer. Continue without it for now.",
            callee.name, WAIT_SECS
        ))
        .into()),
    }
}

#[async_trait]
impl BotTalkHost for ServerTalkHost {
    async fn roster(&self, caller: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
        roster_in(&self.state.db, &self.owner_id, &caller.id, now_millis())
            .await
            .map_err(|error| flow_like_types::anyhow!("{error}"))
    }

    async fn has_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<bool> {
        granted_in(
            &self.state.db,
            &self.owner_id,
            &from.id,
            &to.id,
            now_millis(),
        )
        .await
        .map_err(|error| flow_like_types::anyhow!("{error}"))
    }

    async fn store_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<()> {
        grant_in(
            &self.state.db,
            &self.owner_id,
            &from.id,
            &to.id,
            now_millis(),
        )
        .await
        .map(|_| ())
        .map_err(|error| flow_like_types::anyhow!("{error}"))
    }

    async fn headless_permission(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
    ) -> HeadlessPermission {
        let Some(held) = &self.held else {
            return HeadlessPermission::Unavailable;
        };
        let store = GrantRows::new(&self.state.db, &caller.id, &self.owner_id);
        let (verdict, card) =
            headless_verdict(&store, &held.turn.id, caller, callee, question).await;
        if let Some(card) = card {
            self.parked
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get_or_insert_with(|| {
                    (
                        card,
                        flow_like::flow::copilot::run_recovery::current_recovery_call_id(),
                    )
                });
        }
        verdict
    }

    async fn exchange_conversation(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
    ) -> flow_like_types::Result<String> {
        let title = format!("{} ↔ {}", caller.name, callee.name);
        let id = open_exchange(
            &self.state,
            &self.owner_id,
            &caller.id,
            &callee.id,
            Some(title),
        )
        .await
        .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        self.remember(|asked| asked.callee = Some(callee.clone()));
        Ok(id)
    }

    async fn record(
        &self,
        conversation_id: &str,
        sender: &CrewMember,
        text: &str,
    ) -> flow_like_types::Result<String> {
        self.post(conversation_id, sender, text, 0).await
    }

    async fn record_question(
        &self,
        conversation_id: &str,
        caller: &CrewMember,
        question: &str,
        round: u32,
    ) -> flow_like_types::Result<String> {
        self.post(conversation_id, caller, question, round).await
    }

    async fn run_turn(&self, turn: NestedBotTurn) -> flow_like_types::Result<String> {
        self.await_queued(&turn.caller, &turn.callee, &turn.question_message_id)
            .await
    }
}

impl ServerTalkHost {
    /// The callee's answer is already in the exchange (its turn posted it); anything else the
    /// caller sends is its question of `round`.
    async fn post(
        &self,
        conversation_id: &str,
        sender: &CrewMember,
        text: &str,
        round: u32,
    ) -> flow_like_types::Result<String> {
        let callee = self.remember(|asked| asked.callee.clone());
        let Some(callee) = callee.filter(|callee| callee.id != sender.id) else {
            return self
                .remember(|asked| asked.reply_id.clone())
                .ok_or_else(|| flow_like_types::anyhow!("The answer is not in the exchange yet"));
        };
        let ask_id = ask_message_id(&self.run_id, &callee.id, text);
        let origin = self.origin_conversation(sender, conversation_id).await?;
        let mut chain = self.ancestors.clone();
        chain.push(sender.id.clone());
        let now = now_millis();
        let wait_secs = match callee.presence {
            BotPresence::Offline => 0,
            _ => WAIT_SECS,
        };
        let question = Question {
            text,
            callee_id: &callee.id,
            ask_id: &ask_id,
            origin_conversation: &origin,
            hops: self.hops(),
            chain: &chain,
            wait_secs,
            round,
        };
        let outgoing = message_outgoing(&ask_id, ask_body(&question, now))
            .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        post_as_bot(
            &self.state,
            conversation_id,
            &sender.id,
            question.hops,
            outgoing,
        )
        .await
        .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        let activity =
            ActivityBody::bot_ask(&ask_id, &sender.id, &callee.id, conversation_id, text);
        record_bot_ask(&self.state, &self.owner_id, &origin, &activity).await;
        self.remember(|asked| asked.ask_id = Some(ask_id.clone()));
        Ok(ask_id)
    }

    /// Waits for the callee's answer to the question already recorded as `question_message_id`.
    async fn await_queued(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question_message_id: &str,
    ) -> flow_like_types::Result<String> {
        let started = Instant::now();
        let tid = turn_id(&callee.id, question_message_id);
        let queued = bot_turn::Entity::find_by_id(&tid)
            .one(&self.state.db)
            .await
            .map_err(|error| flow_like_types::anyhow!("{error}"))?;
        if queued.is_none() {
            return Err(flow_like_types::anyhow!(
                "{} took no turn for this question: it is past the limit of bots asking bots, or the exchange can't be read by the server",
                callee.name
            ));
        }
        match callee.location {
            BotLocation::Server => {
                let (state, run_id) = (self.state.clone(), tid.clone());
                tokio::spawn(async move { super::run_hosted(&state, &run_id).await });
            }
            _ if callee.presence == BotPresence::Offline => {
                let seen = last_seen(&self.state.db, &self.owner_id, &callee.id, now_millis())
                    .await
                    .map_or_else(|| "never seen".to_string(), |at| format!("last seen {at}"));
                return Err(PendingAnswer(format!(
                    "{0} is offline ({seen}); the question waits in its inbox and its answer reaches you in this conversation when it is back.",
                    callee.name
                ))
                .into());
            }
            _ => {}
        }
        self.wait_for_answer(caller, callee, &tid, started).await
    }

    /// A bot on a desktop asks `callee` through the hub its question of `round` (0 from desktops
    /// that predate rounds): the exchange, the question and the wait, with the late answer
    /// returning to the asker's direct conversation with its owner.
    pub(crate) async fn ask_for_desktop(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
        round: u32,
    ) -> flow_like_types::Result<DesktopAnswer> {
        let exchange_id = self.exchange_conversation(caller, callee).await?;
        let ask_id = self
            .record_question(&exchange_id, caller, question, round)
            .await?;
        let outcome = self.await_queued(caller, callee, &ask_id).await;
        let reply_id = self.remember(|asked| asked.reply_id.clone());
        Ok(DesktopAnswer {
            exchange_id,
            ask_id,
            reply_id,
            outcome,
        })
    }
}

pub(crate) struct DesktopAnswer {
    pub exchange_id: String,
    pub ask_id: String,
    pub reply_id: Option<String>,
    pub outcome: flow_like_types::Result<String>,
}

/// What a message that mentions or answers a bot says about the ask behind it.
async fn body_of<C: ConnectionTrait>(
    db: &C,
    inbox_id: &str,
    message_id: &str,
) -> Result<Option<(inbox_entry::Model, MessageBody)>, ApiError> {
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox_id))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .one(db)
        .await?;
    Ok(entry.and_then(|entry| {
        let body = entry.envelope.get("body")?;
        if body.get(TOMBSTONE_FIELD).is_some() {
            return None;
        }
        let parsed = MessageBody::parse(body).ok()?;
        Some((entry, parsed))
    }))
}

/// Marks the turns just queued for `recipients` that belong to an ask: a question keeps its
/// turn for [`REPLY_BY_DAYS`] days, and the answer to a question whose asker stopped waiting
/// holds the asker's turn until that wait is over. Only messages of bot turns can be either.
pub(crate) async fn mark_turns<C: ConnectionTrait>(
    db: &C,
    recipients: &[&BotRecipient],
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    for recipient in recipients
        .iter()
        .filter(|recipient| recipient.hops > 0 || recipient.replied_to)
    {
        let Some((trigger, body)) =
            body_of(db, &recipient.bot_id, &recipient.trigger_message_id).await?
        else {
            continue;
        };
        let id = turn_id(&recipient.bot_id, &recipient.trigger_message_id);
        if body.ask.is_some() {
            bot_turn::Entity::update_many()
                .col_expr(bot_turn::Column::ErrorCode, Expr::value(ERROR_ASK_QUEUED))
                .filter(bot_turn::Column::Id.eq(id))
                .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
                .exec(db)
                .await?;
        } else if recipient.replied_to {
            let queued = bot_turn::Entity::find_by_id(&id)
                .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
                .one(db)
                .await?;
            if let Some(turn) = queued
                && let Some(answered) = answered_ask(db, &turn).await?
            {
                super::decision_wait::clear_before_in(
                    db,
                    &recipient.bot_id,
                    &answered.origin_conversation,
                    trigger.created_at,
                )
                .await?;
            }
            if let Some(hold) = answer_hold(db, recipient, &body)
                .await?
                .filter(|at| *at > now)
            {
                bot_turn::Entity::update_many()
                    .col_expr(bot_turn::Column::LeaseUntil, Expr::value(hold))
                    .filter(bot_turn::Column::Id.eq(id))
                    .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
                    .filter(bot_turn::Column::LeaseUntil.is_null())
                    .exec(db)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn answer_hold<C: ConnectionTrait>(
    db: &C,
    recipient: &BotRecipient,
    answer: &MessageBody,
) -> Result<Option<DateTime<FixedOffset>>, ApiError> {
    let Some(reply_to) = answer.reply_to.as_deref() else {
        return Ok(None);
    };
    let Some((entry, question)) = body_of(db, &recipient.bot_id, reply_to).await? else {
        return Ok(None);
    };
    let Some(ask) = question
        .ask
        .filter(|ask| entry.sender_id == recipient.bot_id && ask.wait_secs > 0)
    else {
        return Ok(None);
    };
    Ok(Some(
        entry.created_at + ChronoDuration::seconds(i64::from(ask.wait_secs) + ANSWER_GRACE_SECS),
    ))
}

/// A turn that answers the asker's own question after it stopped waiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnsweredAsk {
    pub origin_conversation: String,
    pub question: String,
    pub callee_id: String,
    pub answer: String,
    pub failed: bool,
    /// The question's round; the asker's woken run goes on counting from it (0: none kept).
    pub round: u32,
}

pub(crate) async fn answered_ask<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<Option<AnsweredAsk>, ApiError> {
    let Some((trigger, answer)) = body_of(db, &turn.bot_id, &turn.trigger_message_id).await? else {
        return Ok(None);
    };
    let Some(reply_to) = answer.reply_to.as_deref() else {
        return Ok(None);
    };
    let Some((entry, question)) = body_of(db, &turn.bot_id, reply_to).await? else {
        return Ok(None);
    };
    let Some(ask) = question.ask.filter(|_| entry.sender_id == turn.bot_id) else {
        return Ok(None);
    };
    Ok(Some(AnsweredAsk {
        origin_conversation: ask.origin_conversation,
        question: question.text,
        callee_id: trigger.sender_id,
        answer: answer.text,
        failed: answer.error.is_some(),
        round: ask.round,
    }))
}

/// The question and answer as the asker's turn reads them, on top of its conversation with the
/// owner.
pub(crate) async fn ask_result_transcript<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    answered: &AnsweredAsk,
) -> Result<Transcript, HistoryError> {
    let callee = bot_name(db, &answered.callee_id)
        .await
        .map_err(HistoryError::Db)?;
    let kind = conversation::Entity::find_by_id(&answered.origin_conversation)
        .one(db)
        .await
        .map_err(|error| HistoryError::Db(error.into()))?
        .and_then(|row| ConversationKind::parse(&row.kind))
        .unwrap_or(ConversationKind::Group);
    let history = history::recent_history(
        db,
        &turn.bot_id,
        &answered.origin_conversation,
        kind,
        &turn.owner_id,
    )
    .await?;
    Ok(Transcript {
        prompt: flow_like_types::bot_message::ask_result_prompt(
            &callee,
            &answered.question,
            &answered.answer,
            answered.failed,
            None,
        ),
        history,
        addressed: true,
        topic_at: None,
        trail: None,
    })
}

/// The name a bot goes by in the owner's crew, or its id when the row is gone.
pub(crate) async fn bot_name<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<String, ApiError> {
    Ok(bot::Entity::find_by_id(bot_id)
        .one(db)
        .await?
        .map(|row| crew_member(&row, BotPresence::Unknown).name)
        .unwrap_or_else(|| bot_id.to_string()))
}

/// A question that lapsed, and what its asker tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lapsed {
    pub turn_id: String,
    pub asker: String,
    pub origin_conversation: String,
    pub text: String,
}

/// Ends the questions nobody picked up in [`REPLY_BY_DAYS`] days (`CANCELLED`) and says what each
/// asker tells its owner.
pub(crate) async fn end_lapsed_in<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Lapsed>, ApiError> {
    let stale = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(bot_turn::Column::ErrorCode.eq(ERROR_ASK_QUEUED))
        .filter(bot_turn::Column::CreatedAt.lt(now - ChronoDuration::days(REPLY_BY_DAYS)))
        .limit(EXPIRE_BATCH)
        .all(db)
        .await?;
    let mut lapsed = Vec::new();
    for turn in stale {
        ledger::cancel_in(db, &turn.id, ERROR_ASK_EXPIRED, now).await?;
        let Some((entry, question)) = body_of(db, &turn.bot_id, &turn.trigger_message_id).await?
        else {
            continue;
        };
        let Some(ask) = question.ask else { continue };
        let callee = bot::Entity::find_by_id(&turn.bot_id).one(db).await?;
        let callee_name = callee.map_or(turn.bot_id.clone(), |row| {
            crew_member(&row, BotPresence::Unknown).name
        });
        lapsed.push(Lapsed {
            asker: entry.sender_id,
            origin_conversation: ask.origin_conversation,
            text: format!(
                "{callee_name} never picked up my question and it lapsed after {REPLY_BY_DAYS} days: {}",
                clip(&question.text, QUESTION_PREVIEW_CHARS)
            ),
            turn_id: turn.id,
        });
    }
    Ok(lapsed)
}

/// [`end_lapsed_in`], then each asker tells its owner. Returns how many ended.
pub(crate) async fn expire_asks(
    state: &AppState,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    let lapsed = end_lapsed_in(&state.db, now).await?;
    for notice in &lapsed {
        let notice_id = format!("askexp-{}", notice.turn_id.trim_start_matches("bt_"));
        let sent = state
            .transaction(|txn| {
                let notice = notice.clone();
                let notice_id = notice_id.clone();
                Box::pin(async move {
                    lock_conversation(txn, &notice.origin_conversation).await?;
                    post_notice_in(
                        txn,
                        &notice.origin_conversation,
                        &notice.asker,
                        &notice_id,
                        &notice.text,
                        now_millis(),
                    )
                    .await
                })
            })
            .await;
        match sent {
            Ok(sent) => wake(state, sent.wake).await,
            Err(error) => {
                tracing::warn!(%error, turn_id = %notice.turn_id, "Bot talk: the lapsed question was not announced")
            }
        }
    }
    Ok(lapsed.len() as u64)
}

/// The bots behind the question that started `turn`, the first asker first. A late answer
/// restores the chain from this bot's own question, excluding this bot from its ancestors.
pub(crate) async fn asked_by<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<Vec<CrewMember>, ApiError> {
    let Some((_, body)) = body_of(db, &turn.bot_id, &turn.trigger_message_id).await? else {
        return Ok(Vec::new());
    };
    let chain = match body.ask {
        Some(ask) => ask.chain,
        None => {
            let Some(reply_to) = body.reply_to.as_deref() else {
                return Ok(Vec::new());
            };
            let Some((question_entry, question)) = body_of(db, &turn.bot_id, reply_to).await?
            else {
                return Ok(Vec::new());
            };
            let Some(mut ask) = question.ask.filter(|_| {
                question_entry.sender_id == turn.bot_id
                    && question_entry.conversation_id == turn.conversation_id
            }) else {
                return Ok(Vec::new());
            };
            if ask.chain.last() != Some(&turn.bot_id) {
                return Ok(Vec::new());
            }
            ask.chain.pop();
            ask.chain
        }
    };
    let rows = bot::Entity::find()
        .filter(bot::Column::Id.is_in(chain.clone()))
        .all(db)
        .await?;
    Ok(chain
        .iter()
        .filter_map(|id| rows.iter().find(|row| &row.id == id))
        .map(|row| crew_member(row, BotPresence::Unknown))
        .collect())
}
