//! The bot-turn ledger: one `BotTurn` row per message that wakes a bot, claimed by the bot's
//! host with a lease. Queueing happens inside the transaction that wrote the message; a claim
//! serializes per owner so a bot has one running turn per conversation and an owner at most
//! [`MAX_RUNNING_PER_OWNER`] of them. A claim resumes checkpointed work first; otherwise it takes
//! the newest queued turn in the conversation and folds older unstarted turns into it.

use crate::{
    db::{DbDialect, RetryPolicy, coordination::coordinate, retry_transaction},
    entity::{bot, bot_host_heartbeat, bot_turn, bot_turn_checkpoint, conversation},
    error::ApiError,
    routes::{
        bots::{model::hands_off_to_desktop, presence::ONLINE_WINDOW_SECS},
        inbox::delivery::{BotDelivery, BotRecipient},
    },
    state::AppState,
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{
        BotTurn, ERROR_BATCHED, ERROR_BOT_DELETED, ERROR_HOST_LOST, ERROR_QUEUE_EXPIRED,
        ERROR_SILENT, INTERACTIVE_GRACE_SECS, LEASE_SECS, MAX_ATTEMPTS, MAX_HOPS,
        MAX_RUNNING_PER_OWNER, QUEUED_EXPIRY_SECS, STORED_CANCELLED, STORED_DESKTOP, STORED_DEVICE,
        STORED_DONE, STORED_FAILED, STORED_QUEUED, STORED_RUNNING, STORED_SERVER, STORED_WAITING,
        TurnAudience, TurnHostKind, TurnStatus, reply_message_id, turn_id,
    },
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, ExprTrait, OnConflict},
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(crate) const BOT_TURN_NOT_QUEUED: &str = "BOT_TURN_NOT_QUEUED";
pub(crate) const BOT_TURN_BUSY: &str = "BOT_TURN_BUSY";
pub(crate) const BOT_TURN_OWNER_BUSY: &str = "BOT_TURN_OWNER_BUSY";
pub(crate) const BOT_TURN_HOST_KIND: &str = "BOT_TURN_HOST_KIND";
pub(crate) const BOT_TURN_LEASE_LOST: &str = "BOT_TURN_LEASE_LOST";
pub(crate) const BOT_TURN_NOT_RETRYABLE: &str = "BOT_TURN_NOT_RETRYABLE";
pub(crate) const BOT_TURN_NOT_CANCELLABLE: &str = "BOT_TURN_NOT_CANCELLABLE";
pub(crate) const ERROR_OWNER_CANCELLED: &str = "BOT_TURN_CANCELLED";
pub(crate) const ERROR_RESUMING: &str = "BOT_TURN_RESUMING";

/// How long a turn handed off to the owner's desktop waits for it before the server takes it
/// back: the desktop host lists turns every 30 seconds, plus a margin.
pub(crate) const DESKTOP_HANDOFF_SECS: i64 = 45;
/// Rows one recovery rule touches per pass; a busy ledger drains over several passes, and a
/// pass stays far below DSQL's per-transaction row cap.
const RECOVER_BATCH: u64 = 200;
/// Turns one listing returns.
pub(crate) const LIST_LIMIT: u64 = 100;
/// Queued rows one listing reads before it groups them.
const LIST_SCAN: u64 = 400;

/// Until when a message from an interactive chat is left to that chat.
pub(crate) fn interactive_hold_until(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    now + Duration::seconds(INTERACTIVE_GRACE_SECS)
}

fn lease_from(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    now + Duration::seconds(LEASE_SECS)
}

pub(crate) fn host_kind_of(location: BotLocation) -> TurnHostKind {
    match location {
        BotLocation::Server => TurnHostKind::Server,
        BotLocation::Desktop => TurnHostKind::Desktop,
        BotLocation::Device => TurnHostKind::Device,
    }
}

/// Whether the message wakes the bot: only messages and spoken transcripts, not one the bot
/// sent itself, not past
/// [`MAX_HOPS`] bot turns, and in a group only when it names the bot or replies to it, or when a
/// person wrote it and the bot reads along ([`BotRecipient::read_along`]). A person's reaction
/// wakes only the bot whose message it reacts to.
/// Transcript deliveries set `read_along` only for bots with a decision model.
pub(crate) fn queues_turn(delivery: &BotDelivery, recipient: &BotRecipient) -> bool {
    if recipient.bot_id == delivery.sender_id || recipient.hops > MAX_HOPS {
        return false;
    }
    match delivery.kind {
        InboxEntryKind::Reaction => recipient.reacted,
        InboxEntryKind::Transcript => recipient.mentioned || recipient.read_along,
        kind if kind.wakes_bot() => {
            delivery.conversation_kind == ConversationKind::Direct
                || recipient.mentioned
                || recipient.replied_to
                || (delivery.kind == InboxEntryKind::Message && recipient.read_along)
        }
        _ => false,
    }
}

impl BotRecipient {
    /// The entry doesn't address the bot in a group: it woke the bot to read along.
    pub(crate) fn reads_along_only(&self) -> bool {
        self.read_along && !self.mentioned && !self.replied_to
    }

    /// Nobody asked the bot a question: it reads along or only got a reaction.
    pub(crate) fn unprompted(&self) -> bool {
        self.reacted || self.reads_along_only()
    }
}

/// A recipient whose turn is about to queue, with whose request it answers.
pub(crate) struct Waking<'a> {
    pub recipient: &'a BotRecipient,
    pub requester_id: String,
    pub audience: TurnAudience,
}

/// Whose request the message is: its sender, except that another bot of the owner in the owner's
/// own conversation speaks for the owner, so it doesn't count against a requester's daily limit.
async fn requester_of<C: ConnectionTrait>(
    db: &C,
    delivery: &BotDelivery,
    owner_id: &str,
) -> Result<String, DbErr> {
    let sender = delivery.sender_id.as_str();
    if sender == owner_id || delivery.conversation_kind != ConversationKind::Direct {
        return Ok(sender.to_string());
    }
    let owned = bot::Entity::find_by_id(sender)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
        .is_some();
    Ok(if owned { owner_id } else { sender }.to_string())
}

async fn waking_of<'a, C: ConnectionTrait>(
    db: &C,
    delivery: &BotDelivery,
    recipients: Vec<&'a BotRecipient>,
) -> Result<Vec<Waking<'a>>, DbErr> {
    let mut waking = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        waking.push(Waking {
            requester_id: requester_of(db, delivery, &recipient.owner_id).await?,
            audience: TurnAudience::of(delivery.conversation_kind),
            recipient,
        });
    }
    Ok(waking)
}

/// The bots among `recipients` that hand off and that one of their owner's desktops reported
/// within the online window.
async fn desktop_online<C: ConnectionTrait>(
    db: &C,
    recipients: &[&BotRecipient],
    now: DateTime<FixedOffset>,
) -> Result<HashSet<String>, DbErr> {
    let ids: Vec<&str> = recipients
        .iter()
        .filter(|recipient| recipient.hands_off)
        .map(|recipient| recipient.bot_id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let seen: Vec<String> = bot_host_heartbeat::Entity::find()
        .select_only()
        .column(bot_host_heartbeat::Column::BotId)
        .distinct()
        .filter(bot_host_heartbeat::Column::BotId.is_in(ids))
        .filter(bot_host_heartbeat::Column::HostKind.eq(STORED_DESKTOP))
        .filter(
            bot_host_heartbeat::Column::LastSeenAt.gte(now - Duration::seconds(ONLINE_WINDOW_SECS)),
        )
        .into_tuple()
        .all(db)
        .await?;
    Ok(seen.into_iter().collect())
}

/// Where a recipient's turn goes: its location's host, or the desktop for a bot that hands off
/// while a desktop of its owner is online.
fn host_kind(recipient: &BotRecipient, desktops: &HashSet<String>) -> TurnHostKind {
    if recipient.hands_off && desktops.contains(&recipient.bot_id) {
        TurnHostKind::Desktop
    } else {
        host_kind_of(recipient.location)
    }
}

/// Whether a host of `kind` may run the bot's turns: the host of its location, and for a bot
/// that hands off, the owner's desktop.
fn runs_on(bot: &bot::Model, kind: TurnHostKind) -> bool {
    kind == host_kind_of(BotLocation::parse(&bot.location).unwrap_or_default())
        || (kind == TurnHostKind::Desktop && hands_off_to_desktop(bot))
}

/// The turn row a recipient gets for `trigger_message_id`.
fn queued_row(
    waking: &Waking,
    kind: TurnHostKind,
    now: DateTime<FixedOffset>,
) -> bot_turn::ActiveModel {
    let recipient = waking.recipient;
    bot_turn::ActiveModel {
        id: Set(turn_id(&recipient.bot_id, &recipient.trigger_message_id)),
        bot_id: Set(recipient.bot_id.clone()),
        owner_id: Set(recipient.owner_id.clone()),
        conversation_id: Set(recipient.conversation_id.clone()),
        trigger_message_id: Set(recipient.trigger_message_id.clone()),
        host_kind: Set(kind.stored().to_string()),
        status: Set(STORED_QUEUED.to_string()),
        attempts: Set(0),
        host_id: Set(None),
        lease_until: Set(recipient.interactive.then(|| interactive_hold_until(now))),
        operation_id: Set(None),
        reply_message_id: Set(None),
        error_code: Set(None),
        hops: Set(i32::try_from(recipient.hops).unwrap_or(i32::MAX)),
        requester_id: Set(Some(waking.requester_id.clone())),
        audience: Set(waking.audience.stored().to_string()),
        status_text: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
}

/// The server reads no sealed envelope, so its bots take no turn in an end-to-end conversation.
async fn server_can_read<C: ConnectionTrait>(db: &C, conversation_id: &str) -> Result<bool, DbErr> {
    let encryption: Option<String> = conversation::Entity::find_by_id(conversation_id)
        .select_only()
        .column(conversation::Column::Encryption)
        .into_tuple()
        .one(db)
        .await?;
    Ok(
        encryption.and_then(|value| ConversationEncryption::parse(&value))
            != Some(ConversationEncryption::E2e),
    )
}

/// Queues a turn for each recipient the message wakes, in the transaction that wrote it. A
/// recipient that already has a turn for the message keeps it. Returns how many rows were
/// written.
pub(crate) async fn enqueue_in<C: ConnectionTrait>(
    db: &C,
    delivery: &BotDelivery,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    let wait_routing = super::decision_wait::routing(db, delivery, now).await?;
    let mut waking: Vec<&BotRecipient> = delivery
        .recipients
        .iter()
        .filter(|recipient| {
            recipient.bot_id != delivery.sender_id
                && recipient.hops <= MAX_HOPS
                && wait_routing
                    .get(&recipient.bot_id)
                    .copied()
                    .unwrap_or_else(|| queues_turn(delivery, recipient))
        })
        .collect();
    let desktops = desktop_online(db, &waking, now).await?;
    let on_server =
        |recipient: &BotRecipient| host_kind(recipient, &desktops) == TurnHostKind::Server;
    let server_conversation = waking
        .iter()
        .find(|recipient| on_server(recipient))
        .map(|recipient| recipient.conversation_id.clone());
    if let Some(conversation_id) = server_conversation
        && !server_can_read(db, &conversation_id).await?
    {
        waking.retain(|recipient| !on_server(recipient));
    }
    let waking = super::cap::admit(db, waking, now).await?;
    let waking = waking_of(db, delivery, waking).await?;
    let waking = super::cap::admit_requesters(db, waking, now).await?;
    if waking.is_empty() {
        return Ok(0);
    }
    let rows: Vec<_> = waking
        .iter()
        .map(|item| queued_row(item, host_kind(item.recipient, &desktops), now))
        .collect();
    let inserted = bot_turn::Entity::insert_many(rows)
        .on_conflict(
            OnConflict::columns([bot_turn::Column::BotId, bot_turn::Column::TriggerMessageId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    let admitted: Vec<&BotRecipient> = waking.iter().map(|item| item.recipient).collect();
    super::talk::mark_turns(db, &admitted, now).await?;
    Ok(inserted)
}

/// A turn as hosts and watchers read it.
pub(crate) fn turn_json(turn: &bot_turn::Model, batched: Vec<String>) -> BotTurn {
    BotTurn {
        id: turn.id.clone(),
        bot_id: turn.bot_id.clone(),
        conversation_id: turn.conversation_id.clone(),
        trigger_message_id: turn.trigger_message_id.clone(),
        host_kind: TurnHostKind::parse(&turn.host_kind).unwrap_or(TurnHostKind::Server),
        status: TurnStatus::parse(&turn.status).unwrap_or(TurnStatus::Failed),
        attempts: u32::try_from(turn.attempts).unwrap_or(0),
        host_id: turn.host_id.clone(),
        lease_until: turn.lease_until.map(|at| at.to_rfc3339()),
        reply_message_id: turn.reply_message_id.clone(),
        error_code: turn.error_code.clone(),
        hops: u32::try_from(turn.hops).unwrap_or(0),
        owner_id: turn.owner_id.clone(),
        requester_id: turn.requester_id.clone(),
        requester_name: None,
        audience: TurnAudience::parse(&turn.audience).unwrap_or(TurnAudience::Room),
        created_at: turn.created_at.to_rfc3339(),
        updated_at: turn.updated_at.to_rfc3339(),
        batched,
    }
}

fn ready(now: DateTime<FixedOffset>) -> Condition {
    Condition::any()
        .add(bot_turn::Column::LeaseUntil.is_null())
        .add(bot_turn::Column::LeaseUntil.lte(now))
}

fn live_bots() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::DeletedAt.is_null())
        .into_query()
}

fn checkpoint_turns() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    bot_turn_checkpoint::Entity::find().select_only()
        .column(bot_turn_checkpoint::Column::TurnId)
        .filter(bot_turn_checkpoint::Column::Revision.gt(0)).into_query()
}

fn resuming() -> Condition {
    Condition::any().add(bot_turn::Column::ErrorCode.eq(ERROR_RESUMING))
        .add(bot_turn::Column::Id.in_subquery(checkpoint_turns()))
}

/// Recover interrupted work first; otherwise take the newest queued turn in the conversation.
pub(crate) async fn next_head<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    kind: TurnHostKind,
    now: DateTime<FixedOffset>,
) -> Result<Option<bot_turn::Model>, DbErr> {
    let queued = bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::ConversationId.eq(conversation_id))
        .filter(bot_turn::Column::HostKind.eq(kind.stored()))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(ready(now));
    // Recovered work retains its trigger and checkpoint even if newer messages arrive.
    if let Some(resuming) = queued.clone()
        .filter(resuming())
        .order_by_asc(bot_turn::Column::CreatedAt)
        .order_by_asc(bot_turn::Column::Id).one(db).await? {
        return Ok(Some(resuming));
    }
    queued
        .order_by_desc(bot_turn::Column::CreatedAt)
        .order_by_desc(bot_turn::Column::Id)
        .one(db)
        .await
}

/// Turns waiting for a host of this kind, one per bot and conversation: interrupted work first,
/// otherwise the newest with older trigger messages it will answer too. Deleted bots and bots with a
/// turn running in that conversation are left out. `owner_id` limits them to one person's.
pub(crate) async fn queued_heads<C: ConnectionTrait>(
    db: &C,
    owner_id: Option<&str>,
    kind: TurnHostKind,
    now: DateTime<FixedOffset>,
    limit: u64,
) -> Result<Vec<(bot_turn::Model, Vec<String>)>, DbErr> {
    queued_heads_scoped(db, owner_id, kind, None, now, limit).await
}

/// [`queued_heads`] for one device: only turns of the owner's live bots bound to `device_id`,
/// filtered in SQL so another device's turns never fill the scan.
pub(crate) async fn queued_device_heads<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    device_id: &str,
    now: DateTime<FixedOffset>,
    limit: u64,
) -> Result<Vec<(bot_turn::Model, Vec<String>)>, DbErr> {
    let bots = device_bots(owner_id, device_id);
    queued_heads_scoped(
        db,
        Some(owner_id),
        TurnHostKind::Device,
        Some(bots),
        now,
        limit,
    )
    .await
}

/// The live bots of `owner_id` that run on the device `device_id`.
pub(crate) fn device_bots(owner_id: &str, device_id: &str) -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::DeletedAt.is_null())
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::Location.eq(STORED_DEVICE))
        .filter(bot::Column::DeviceId.eq(device_id))
        .into_query()
}

async fn queued_heads_scoped<C: ConnectionTrait>(
    db: &C,
    owner_id: Option<&str>,
    kind: TurnHostKind,
    bots: Option<sea_orm::sea_query::SelectStatement>,
    now: DateTime<FixedOffset>,
    limit: u64,
) -> Result<Vec<(bot_turn::Model, Vec<String>)>, DbErr> {
    let mut queued = bot_turn::Entity::find()
        .filter(bot_turn::Column::HostKind.eq(kind.stored()))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(ready(now))
        .filter(bot_turn::Column::BotId.in_subquery(bots.unwrap_or_else(live_bots)));
    let mut running = bot_turn::Entity::find()
        .filter(bot_turn::Column::HostKind.eq(kind.stored()))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::LeaseUntil.gt(now));
    if let Some(owner_id) = owner_id {
        queued = queued.filter(bot_turn::Column::OwnerId.eq(owner_id));
        running = running.filter(bot_turn::Column::OwnerId.eq(owner_id));
    }
    let queued = queued
        .order_by_asc(bot_turn::Column::CreatedAt)
        .order_by_asc(bot_turn::Column::Id)
        .limit(LIST_SCAN)
        .all(db)
        .await?;
    if queued.is_empty() {
        return Ok(Vec::new());
    }
    let checkpointed: HashSet<String> = bot_turn_checkpoint::Entity::find()
        .select_only().column(bot_turn_checkpoint::Column::TurnId)
        .filter(bot_turn_checkpoint::Column::TurnId.is_in(queued.iter().map(|turn| turn.id.clone())))
        .filter(bot_turn_checkpoint::Column::Revision.gt(0))
        .into_tuple().all(db).await?.into_iter().collect();
    let busy: BTreeSet<(String, String)> = running
        .all(db)
        .await?
        .into_iter()
        .map(|turn| (turn.bot_id, turn.conversation_id))
        .collect();
    let mut groups: BTreeMap<(String, String), Vec<bot_turn::Model>> = BTreeMap::new();
    for turn in queued {
        groups
            .entry((turn.bot_id.clone(), turn.conversation_id.clone()))
            .or_default()
            .push(turn);
    }
    let mut heads: Vec<(bot_turn::Model, Vec<String>)> = groups
        .into_iter()
        .filter(|(key, _)| !busy.contains(key))
        .filter_map(|(_, mut turns)| {
            if let Some(index) = turns.iter().position(|turn| turn.error_code.as_deref() == Some(ERROR_RESUMING)
                || checkpointed.contains(&turn.id)) {
                return Some((turns.remove(index), Vec::new()));
            }
            let head = turns.pop()?;
            let older = turns
                .into_iter()
                .map(|turn| turn.trigger_message_id)
                .collect();
            Some((head, older))
        })
        .collect();
    heads.sort_by(|(a, _), (b, _)| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    heads.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(heads)
}

/// Why a claim took nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotFound,
    /// The turn is not waiting: another host holds it or it is finished.
    NotQueued(TurnStatus),
    /// The turn retries later.
    NotYet,
    /// Another turn of this bot runs in this conversation.
    Busy,
    /// The owner already has [`MAX_RUNNING_PER_OWNER`] turns running.
    OwnerBusy,
    /// The turn belongs to a host of another kind.
    HostKind(TurnHostKind),
    /// The bot was deleted; the turn is cancelled.
    BotGone,
    /// Another host took the turn between the check and the claim.
    Lost,
}

impl Refusal {
    pub(crate) fn into_api(self, turn_id: &str) -> ApiError {
        let conflict =
            |code: &str, message: String| ApiError::coded(StatusCode::CONFLICT, code, message);
        match self {
            Self::NotFound => ApiError::not_found(format!("Turn '{turn_id}' not found")),
            Self::BotGone => ApiError::not_found(format!(
                "The bot of turn '{turn_id}' was deleted; the turn is cancelled"
            )),
            Self::NotQueued(status) => conflict(
                BOT_TURN_NOT_QUEUED,
                format!("Turn '{turn_id}' is {}, not queued", status.as_str()),
            ),
            Self::NotYet => conflict(
                BOT_TURN_NOT_QUEUED,
                format!("Turn '{turn_id}' waits to retry; claim it again shortly"),
            ),
            Self::Lost => conflict(
                BOT_TURN_NOT_QUEUED,
                format!("Turn '{turn_id}' was claimed by another host"),
            ),
            Self::Busy => conflict(
                BOT_TURN_BUSY,
                format!(
                    "Another turn of this bot is running in this conversation; '{turn_id}' follows it"
                ),
            ),
            Self::OwnerBusy => conflict(
                BOT_TURN_OWNER_BUSY,
                format!(
                    "You already have {MAX_RUNNING_PER_OWNER} bot turns running; '{turn_id}' waits for one to finish"
                ),
            ),
            Self::HostKind(kind) => conflict(
                BOT_TURN_HOST_KIND,
                format!(
                    "Turn '{turn_id}' belongs to a {} host, not this one",
                    kind.as_str()
                ),
            ),
        }
    }
}

/// A host asking for a turn.
#[derive(Debug, Clone)]
pub(crate) struct ClaimRequest {
    pub turn_id: String,
    /// Set when a person's token claims: only their own turns are theirs to claim.
    pub owner_id: Option<String>,
    pub host_kind: TurnHostKind,
    pub host_id: String,
    pub now: DateTime<FixedOffset>,
}

/// The turn a claim took and the trigger messages of the older turns it replaced.
#[derive(Debug, Clone)]
pub(crate) struct Claimed {
    pub turn: bot_turn::Model,
    pub batched: Vec<String>,
}

pub(crate) type ClaimResult = Result<Claimed, Refusal>;

pub(crate) async fn find<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: Option<&str>,
) -> Result<Option<bot_turn::Model>, DbErr> {
    let mut query = bot_turn::Entity::find_by_id(turn_id);
    if let Some(owner_id) = owner_id {
        query = query.filter(bot_turn::Column::OwnerId.eq(owner_id));
    }
    query.one(db).await
}

fn live_running(now: DateTime<FixedOffset>) -> sea_orm::Select<bot_turn::Entity> {
    bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
}

/// Claims the newest queued turn of the requested turn's bot in its conversation, folding the
/// older ones into it, or takes over a running turn whose lease ran out. A repeat claim by the
/// host that holds a live lease returns the turn as it is. The caller holds the owner's
/// `bot-turn-owner` lock, which is what keeps the one-turn-per-conversation and per-owner caps
/// exact; the compare-and-set on the row is the second fence.
pub(crate) async fn claim_in<C: ConnectionTrait>(
    db: &C,
    request: &ClaimRequest,
) -> Result<ClaimResult, ApiError> {
    let now = request.now;
    let Some(requested) = find(db, &request.turn_id, request.owner_id.as_deref()).await? else {
        return Ok(Err(Refusal::NotFound));
    };
    let kind = TurnHostKind::parse(&requested.host_kind).unwrap_or(TurnHostKind::Server);
    if kind != request.host_kind {
        return Ok(Err(Refusal::HostKind(kind)));
    }
    let bot = bot::Entity::find_by_id(&requested.bot_id).one(db).await?;
    let Some(bot) = bot.filter(|bot| bot.deleted_at.is_none()) else {
        cancel_in(db, &requested.id, ERROR_BOT_DELETED, now).await?;
        return Ok(Err(Refusal::BotGone));
    };
    if !runs_on(&bot, kind) {
        let located = host_kind_of(BotLocation::parse(&bot.location).unwrap_or_default());
        rehome_in(db, &requested.id, located, now).await?;
        return Ok(Err(Refusal::HostKind(located)));
    }
    let status = TurnStatus::parse(&requested.status).unwrap_or(TurnStatus::Failed);
    let lease_live = requested.lease_until.is_some_and(|lease| lease > now);
    match status {
        TurnStatus::Queued if lease_live => return Ok(Err(Refusal::NotYet)),
        TurnStatus::Queued => {}
        TurnStatus::Running if lease_live => {
            return Ok(
                if requested.host_id.as_deref() == Some(request.host_id.as_str()) {
                    Ok(Claimed {
                        turn: requested,
                        batched: Vec::new(),
                    })
                } else {
                    Err(Refusal::NotQueued(status))
                },
            );
        }
        TurnStatus::Running => {}
        finished => return Ok(Err(Refusal::NotQueued(finished))),
    }
    let siblings_running = live_running(now)
        .filter(bot_turn::Column::BotId.eq(requested.bot_id.as_str()))
        .filter(bot_turn::Column::ConversationId.eq(requested.conversation_id.as_str()))
        .filter(bot_turn::Column::Id.ne(requested.id.as_str()))
        .count(db)
        .await?;
    if siblings_running > 0 {
        return Ok(Err(Refusal::Busy));
    }
    let owner_running = live_running(now)
        .filter(bot_turn::Column::OwnerId.eq(requested.owner_id.as_str()))
        .filter(bot_turn::Column::Id.ne(requested.id.as_str()))
        .count(db)
        .await?;
    if owner_running >= MAX_RUNNING_PER_OWNER as u64 {
        return Ok(Err(Refusal::OwnerBusy));
    }
    let lead = match status {
        TurnStatus::Queued => {
            next_head(db, &requested.bot_id, &requested.conversation_id, kind, now)
                .await?
                .unwrap_or_else(|| requested.clone())
        }
        _ => requested.clone(),
    };
    let failures = if status == TurnStatus::Running {
        if !lock_snapshot(db, &lead).await? { return Ok(Err(Refusal::Lost)); }
        super::recovery::charge_failure(db, &lead).await?
    } else {
        super::recovery::failures(db, &lead).await?
    };
    if failures >= MAX_ATTEMPTS as i32 {
        let failed = end_all(
            db,
            std::slice::from_ref(&lead),
            &[STORED_QUEUED, STORED_RUNNING],
            STORED_FAILED,
            ERROR_HOST_LOST,
            now,
        )
        .await?;
        return Ok(Err(if failed > 0 {
            Refusal::NotQueued(TurnStatus::Failed)
        } else {
            Refusal::Lost
        }));
    }
    if !take_in(db, &lead.id, &request.host_id, now).await? {
        return Ok(Err(Refusal::Lost));
    }
    let batched = if status == TurnStatus::Queued && lead.error_code.as_deref() != Some(ERROR_RESUMING) {
        fold_into(db, &lead, now).await?
    } else {
        Vec::new()
    };
    let turn = find(db, &lead.id, None).await?.ok_or_else(|| {
        ApiError::internal(format!("Turn '{}' vanished after its claim", lead.id))
    })?;
    Ok(Ok(Claimed { turn, batched }))
}

/// The compare-and-set of a claim: the turn is the host's only if it is still queued and due, or
/// running with a lease that ran out. `false`: another claim got there first.
pub(crate) async fn take_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let taken = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_RUNNING))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::HostId, Expr::value(host_id))
        .col_expr(bot_turn::Column::LeaseUntil, Expr::value(lease_from(now)))
        .col_expr(
            bot_turn::Column::Attempts,
            Expr::col(bot_turn::Column::Attempts).add(1),
        )
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Attempts.lt(i32::MAX))
        .filter(bot_turn::Column::Id.not_in_subquery({
            use sea_orm::QueryTrait;
            bot_turn_checkpoint::Entity::find().select_only()
                .column(bot_turn_checkpoint::Column::TurnId)
                .filter(bot_turn_checkpoint::Column::TurnId.eq(turn_id))
                .filter(bot_turn_checkpoint::Column::RecoveryAttempts.gte(MAX_ATTEMPTS as i32))
                .into_query()
        }))
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(bot_turn::Column::Status.eq(STORED_QUEUED))
                        .add(ready(now)),
                )
                .add(
                    Condition::all()
                        .add(bot_turn::Column::Status.eq(STORED_RUNNING))
                        .add(bot_turn::Column::LeaseUntil.lt(now)),
                ),
        )
        .exec(db)
        .await?;
    Ok(taken.rows_affected > 0)
}

/// The queued turns of the lead's bot in its conversation, which the lead answers too: they end
/// `CANCELLED` with [`ERROR_BATCHED`] and the lead's reply id. Returns their trigger messages.
async fn fold_into<C: ConnectionTrait>(
    db: &C,
    lead: &bot_turn::Model,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, DbErr> {
    if bot_turn_checkpoint::Entity::find_by_id(&lead.id)
        .filter(bot_turn_checkpoint::Column::Revision.gt(0)).one(db).await?.is_some() {
        return Ok(Vec::new());
    }
    let older = bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(lead.bot_id.as_str()))
        .filter(bot_turn::Column::ConversationId.eq(lead.conversation_id.as_str()))
        .filter(bot_turn::Column::HostKind.eq(lead.host_kind.as_str()))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(bot_turn::Column::Id.ne(lead.id.as_str()))
        .filter(Condition::any().add(bot_turn::Column::ErrorCode.is_null())
            .add(bot_turn::Column::ErrorCode.ne(ERROR_RESUMING)))
        .filter(bot_turn::Column::Id.not_in_subquery(checkpoint_turns()))
        .order_by_asc(bot_turn::Column::CreatedAt)
        .order_by_asc(bot_turn::Column::Id)
        .all(db)
        .await?;
    if older.is_empty() {
        return Ok(Vec::new());
    }
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_CANCELLED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(ERROR_BATCHED))
        .col_expr(
            bot_turn::Column::ReplyMessageId,
            Expr::value(reply_message_id(&lead.bot_id, &lead.trigger_message_id)),
        )
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.is_in(older.iter().map(|turn| turn.id.clone())))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .exec(db)
        .await?;
    Ok(older
        .into_iter()
        .map(|turn| turn.trigger_message_id)
        .collect())
}

/// Claims `request.turn_id` under the owner's lock, in a retried transaction.
pub(crate) async fn claim(
    state: &AppState,
    request: ClaimRequest,
) -> Result<ClaimResult, ApiError> {
    claim_with(&state.db, state.db_dialect, request).await
}

pub(crate) async fn claim_with(
    db: &DatabaseConnection,
    dialect: DbDialect,
    request: ClaimRequest,
) -> Result<ClaimResult, ApiError> {
    let Some(owner) = find(db, &request.turn_id, request.owner_id.as_deref())
        .await?
        .map(|turn| turn.owner_id)
    else {
        return Ok(Err(Refusal::NotFound));
    };
    retry_transaction(db, dialect, None, &RetryPolicy::default(), |txn| {
        let (owner, request) = (owner.clone(), request.clone());
        Box::pin(async move {
            coordinate(txn, "bot-turn-owner", &[owner.as_str()]).await?;
            claim_in(txn, &request).await
        })
    })
    .await
}

/// The turn when it belongs to a live bot of `owner_id` bound to the device `device_id`.
pub(crate) async fn device_turn<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: &str,
    device_id: &str,
) -> Result<Option<bot_turn::Model>, DbErr> {
    bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::BotId.in_subquery(device_bots(owner_id, device_id)))
        .one(db)
        .await
}

/// [`claim_with`] for the device `device_id` of `request.owner_id`: a turn whose bot isn't bound
/// to that device reads as missing, checked in the claim's own transaction.
pub(crate) async fn claim_for_device(
    db: &DatabaseConnection,
    dialect: DbDialect,
    device_id: &str,
    request: ClaimRequest,
) -> Result<ClaimResult, ApiError> {
    let Some(owner) = request.owner_id.clone() else {
        return Err(ApiError::internal(
            "A device claim names the owner the device belongs to",
        ));
    };
    retry_transaction(db, dialect, None, &RetryPolicy::default(), |txn| {
        let (owner, device_id, request) = (owner.clone(), device_id.to_string(), request.clone());
        Box::pin(async move {
            coordinate(txn, "bot-turn-owner", &[owner.as_str()]).await?;
            claim_device_in(txn, &device_id, &request).await
        })
    })
    .await
}

/// [`claim_in`] that takes only the turns of bots bound to `device_id`; the caller holds the
/// owner's `bot-turn-owner` lock.
pub(crate) async fn claim_device_in<C: ConnectionTrait>(
    db: &C,
    device_id: &str,
    request: &ClaimRequest,
) -> Result<ClaimResult, ApiError> {
    let Some(owner) = request.owner_id.as_deref() else {
        return Err(ApiError::internal(
            "A device claim names the owner the device belongs to",
        ));
    };
    if device_turn(db, &request.turn_id, owner, device_id)
        .await?
        .is_none()
    {
        return Ok(Err(Refusal::NotFound));
    }
    claim_in(db, request).await
}

/// An interactive chat's hold on the turn of the message it sent: pushes the not-before that
/// keeps hosts away while the chat streams its answer. `false`: a host claimed the turn or it is
/// finished, so the chat must not write a second reply. One that isn't `owner_id`'s reads as
/// missing.
pub(crate) async fn hold_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let held = bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(interactive_hold_until(now)),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .exec(db)
        .await?;
    Ok(held.rows_affected > 0)
}

/// Extends the lease of a turn the host holds. A turn that isn't the host's, or whose lease ran
/// out, is [`BOT_TURN_LEASE_LOST`]; one that isn't `owner_id`'s reads as missing.
pub(crate) async fn heartbeat_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: Option<&str>,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bot_turn::Model, ApiError> {
    let mut extend = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::LeaseUntil, Expr::value(lease_from(now)))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now));
    if let Some(owner_id) = owner_id {
        extend = extend.filter(bot_turn::Column::OwnerId.eq(owner_id));
    }
    if extend.exec(db).await?.rows_affected > 0 {
        return find(db, turn_id, owner_id)
            .await?
            .ok_or_else(|| Refusal::NotFound.into_api(turn_id));
    }
    Err(lease_lost(db, turn_id, owner_id, host_id).await?)
}

async fn lease_lost<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: Option<&str>,
    host_id: &str,
) -> Result<ApiError, ApiError> {
    let turn = find(db, turn_id, owner_id)
        .await?
        .ok_or_else(|| Refusal::NotFound.into_api(turn_id))?;
    let reason = if turn.status != STORED_RUNNING {
        format!("it is {}, not running", turn.status.to_ascii_lowercase())
    } else if turn.host_id.as_deref() != Some(host_id) {
        "another host holds it".to_string()
    } else {
        "its lease ran out; claim it again".to_string()
    };
    Ok(ApiError::coded(
        StatusCode::CONFLICT,
        BOT_TURN_LEASE_LOST,
        format!("Host '{host_id}' no longer holds turn '{turn_id}': {reason}"),
    ))
}

/// Sets the live status line of a turn the host holds (`None` clears it), under the same
/// filters as [`heartbeat_in`]. `false`: the lease is no longer the host's.
pub(crate) async fn set_status_text_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    status_text: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let set = bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::StatusText,
            Expr::value(status_text.map(str::to_owned)),
        )
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .exec(db)
        .await?;
    Ok(set.rows_affected > 0)
}

/// [`set_status_text_in`] for a host acting for `owner_id`: a turn that isn't theirs reads as
/// missing, one the host no longer holds is [`BOT_TURN_LEASE_LOST`].
pub(crate) async fn set_owned_status_text_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    owner_id: &str,
    host_id: &str,
    status_text: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    find(db, turn_id, Some(owner_id))
        .await?
        .ok_or_else(|| Refusal::NotFound.into_api(turn_id))?;
    if set_status_text_in(db, turn_id, host_id, status_text, now).await? {
        return Ok(());
    }
    Err(lease_lost(db, turn_id, Some(owner_id), host_id).await?)
}

/// Whether `host_id` still holds a live lease on the turn.
pub(crate) async fn holds_lease<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    Ok(bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .count(db)
        .await?
        > 0)
}

/// Ends a turn the host holds as failed without a reply. `false`: the lease is no longer the
/// host's.
pub(crate) async fn fail_held_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let ended = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_FAILED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(error_code))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .exec(db)
        .await?;
    Ok(ended.rows_affected > 0)
}

/// Marks a turn `FAILED` right after its failure report was sent as the turn's reply, in the
/// same transaction, so the turn never reads `DONE` for a message that says it failed.
pub(crate) async fn mark_failed_after_reply_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    reply_message_id: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let marked = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_FAILED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(error_code))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_DONE))
        .filter(bot_turn::Column::ReplyMessageId.eq(reply_message_id))
        .exec(db)
        .await?;
    Ok(marked.rows_affected > 0)
}

/// Ends a turn the host holds `DONE` without a reply: the bot read along and stayed silent.
pub(crate) async fn complete_silent_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let done = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(ERROR_SILENT))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .exec(db)
        .await?;
    if done.rows_affected > 0 {
        super::recovery::clear_finished_in(db, turn_id).await?;
    }
    Ok(done.rows_affected > 0)
}

/// Under a transaction, spend one failure attempt and requeue at `retry_at`, or fail when spent.
pub(crate) async fn release_for_retry_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    host_id: &str,
    retry_at: DateTime<FixedOffset>,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let Some(turn) = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now)).one(db).await? else { return Ok(false); };
    if !lock_snapshot(db, &turn).await? { return Ok(false); }
    if super::recovery::charge_failure(db, &turn).await? >= MAX_ATTEMPTS as i32 {
        return fail_held_in(db, turn_id, host_id, error_code, now).await;
    }
    let released = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_QUEUED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::LeaseUntil, Expr::value(retry_at))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(error_code))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .exec(db)
        .await?;
    Ok(released.rows_affected > 0)
}

/// Fails a turn nobody holds, whatever host it was claimed by.
async fn fail_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    end_unfinished_in(db, turn_id, STORED_FAILED, error_code, now).await
}

/// Cancels a turn that is queued, running or waiting for an approval: its bot is gone, or its owner stopped it.
pub(crate) async fn cancel_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    end_unfinished_in(db, turn_id, STORED_CANCELLED, error_code, now).await
}

/// The owner's cancellation of a queued, running or waiting turn of `bot_id`. A running turn loses its
/// lease with the status, so its host's next heartbeat is [`BOT_TURN_LEASE_LOST`]. A turn that
/// isn't the owner's reads as missing; a finished one is [`BOT_TURN_NOT_CANCELLABLE`].
pub(crate) async fn cancel_owned_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<bot_turn::Model, ApiError> {
    let cancelled = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_CANCELLED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::ErrorCode,
            Expr::value(ERROR_OWNER_CANCELLED),
        )
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::Status.is_in([STORED_QUEUED, STORED_RUNNING, STORED_WAITING]))
        .exec(db)
        .await?;
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .one(db)
        .await?
        .ok_or_else(|| Refusal::NotFound.into_api(turn_id))?;
    if cancelled.rows_affected > 0 {
        super::recovery::clear_in(db, turn_id).await?;
    }
    if cancelled.rows_affected == 0 {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_NOT_CANCELLABLE,
            format!(
                "Turn '{turn_id}' is {} and can't be cancelled",
                turn.status.to_ascii_lowercase()
            ),
        ));
    }
    Ok(turn)
}

async fn end_unfinished_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    status: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(status))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(error_code))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.is_in([STORED_QUEUED, STORED_RUNNING, STORED_WAITING]))
        .exec(db)
        .await?;
    if status == STORED_CANCELLED {
        super::recovery::clear_in(db, turn_id).await?;
    }
    Ok(())
}

/// Moves a queued turn to the host kind its bot runs on now.
async fn rehome_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    kind: TurnHostKind,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::HostKind, Expr::value(kind.stored()))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .exec(db)
        .await?;
    Ok(())
}

/// Gives back to the server the turns of server bots that sat queued for a desktop for
/// [`DESKTOP_HANDOFF_SECS`]: the desktop went offline before it claimed them, or lost its lease.
/// Returns their ids, for the server's host to pick up at once.
pub(crate) async fn hand_back_in<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, DbErr> {
    use sea_orm::QueryTrait;
    let server_bots = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::Location.eq(BotLocation::Server.as_str().to_ascii_uppercase()))
        .into_query();
    let unclaimed: Vec<String> = bot_turn::Entity::find()
        .select_only()
        .column(bot_turn::Column::Id)
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(bot_turn::Column::HostKind.eq(STORED_DESKTOP))
        .filter(ready(now))
        .filter(bot_turn::Column::UpdatedAt.lt(now - Duration::seconds(DESKTOP_HANDOFF_SECS)))
        .filter(bot_turn::Column::BotId.in_subquery(server_bots))
        .limit(RECOVER_BATCH)
        .into_tuple()
        .all(db)
        .await?;
    if unclaimed.is_empty() {
        return Ok(unclaimed);
    }
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::HostKind, Expr::value(STORED_SERVER))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.is_in(unclaimed.iter().cloned()))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(bot_turn::Column::HostKind.eq(STORED_DESKTOP))
        .exec(db)
        .await?;
    Ok(unclaimed)
}

/// Queues the owner's failed turn with a fresh failure budget and a monotonic claim generation.
/// The caller holds a transaction. `None` means the owner has no such failed turn.
pub(crate) async fn retry_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Option<bot_turn::Model>, ApiError> {
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .one(db)
        .await?;
    let Some(turn) = turn else {
        return Ok(None);
    };
    if turn.status != STORED_FAILED {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_NOT_RETRYABLE,
            format!(
                "Turn '{turn_id}' is {}; only a failed turn can be retried",
                turn.status.to_ascii_lowercase()
            ),
        ));
    }
    let queued = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_QUEUED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(
            bot_turn::Column::ReplyMessageId,
            Expr::value(None::<String>),
        )
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn_id))
        .filter(bot_turn::Column::Status.eq(STORED_FAILED))
        .exec(db)
        .await?;
    if queued.rows_affected == 0 {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_NOT_RETRYABLE,
            format!("Turn '{turn_id}' changed before it was queued again"),
        ));
    }
    bot_turn_checkpoint::Entity::update_many()
        .col_expr(bot_turn_checkpoint::Column::RecoveryAttempts, Expr::value(0))
        .filter(bot_turn_checkpoint::Column::TurnId.eq(turn_id))
        .exec(db)
        .await?;
    super::recovery::rebase_deadline_in(db, turn_id, now).await?;
    Ok(find(db, turn_id, Some(owner_id)).await?)
}

/// A turn the person may watch: its bot's owner, or an active person in its conversation.
pub(crate) async fn watched_turn<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    conversation_id: &str,
    turn_id: &str,
) -> Result<Option<bot_turn::Model>, DbErr> {
    use crate::entity::conversation_member;
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?;
    let Some(turn) = turn else {
        return Ok(None);
    };
    if turn.owner_id == sub {
        return Ok(Some(turn));
    }
    let member =
        conversation_member::Entity::find_by_id((conversation_id.to_string(), sub.to_string()))
            .filter(crate::routes::inbox::active_member())
            .count(db)
            .await?;
    Ok((member > 0).then_some(turn))
}

/// What one recovery pass changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Recovered {
    /// Turns of deleted bots, cancelled.
    pub cancelled: u64,
    /// Queued turns nobody took within the hour, failed.
    pub expired: u64,
    /// Running turns whose lease ran out, queued again.
    pub requeued: u64,
    /// Running turns whose lease ran out after their last attempt, failed.
    pub lost: u64,
}

/// Ends or re-queues what no host is looking after: turns of deleted bots are cancelled, turns
/// queued for [`QUEUED_EXPIRY_SECS`] fail, and a running turn whose lease ran out goes back to
/// the queue, or fails after its last attempt. Each rule takes at most [`RECOVER_BATCH`] rows.
pub(crate) async fn recover_in<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<Recovered, ApiError> {
    let unfinished = [STORED_QUEUED, STORED_RUNNING, STORED_WAITING];
    let deleted = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.is_in(unfinished))
        .filter(bot_turn::Column::BotId.not_in_subquery(live_bots()))
        .limit(RECOVER_BATCH)
        .all(db)
        .await?;
    let cancelled = end_all(
        db,
        &deleted,
        &unfinished,
        STORED_CANCELLED,
        ERROR_BOT_DELETED,
        now,
    )
    .await?;
    let stale = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .filter(bot_turn::Column::UpdatedAt.lt(now - Duration::seconds(QUEUED_EXPIRY_SECS)))
        .filter(
            Condition::any()
                .add(bot_turn::Column::ErrorCode.is_null())
                .add(bot_turn::Column::ErrorCode.ne(super::talk::ERROR_ASK_QUEUED)),
        )
        .limit(RECOVER_BATCH)
        .all(db)
        .await?;
    let expired = end_all(
        db,
        &stale,
        &[STORED_QUEUED],
        STORED_FAILED,
        ERROR_QUEUE_EXPIRED,
        now,
    )
    .await?;
    let abandoned = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::LeaseUntil.lt(now))
        .limit(RECOVER_BATCH)
        .all(db)
        .await?;
    let (mut spent, mut retryable) = (Vec::new(), Vec::new());
    for turn in abandoned {
        if !lock_snapshot(db, &turn).await? { continue; }
        if super::recovery::charge_failure(db, &turn).await? >= MAX_ATTEMPTS as i32 {
            spent.push(turn);
        } else { retryable.push(turn); }
    }
    let lost = end_all(
        db,
        &spent,
        &[STORED_RUNNING],
        STORED_FAILED,
        ERROR_HOST_LOST,
        now,
    )
    .await?;
    let requeued = requeue_all(db, &retryable, now).await?;
    Ok(Recovered {
        cancelled,
        expired,
        requeued,
        lost,
    })
}

async fn requeue_all<C: ConnectionTrait>(
    db: &C,
    turns: &[bot_turn::Model],
    now: DateTime<FixedOffset>,
) -> Result<u64, DbErr> {
    if turns.is_empty() {
        return Ok(0);
    }
    Ok(bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_QUEUED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(ERROR_RESUMING))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(unchanged_turns(turns))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::LeaseUntil.lt(now))
        .exec(db)
        .await?
        .rows_affected)
}

pub(super) async fn end_all<C: ConnectionTrait>(
    db: &C,
    turns: &[bot_turn::Model],
    from: &[&str],
    status: &str,
    error_code: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, DbErr> {
    if turns.is_empty() {
        return Ok(0);
    }
    Ok(bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(status))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(error_code))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(unchanged_turns(turns))
        .filter(bot_turn::Column::Status.is_in(from.iter().copied()))
        .exec(db)
        .await?
        .rows_affected)
}

/// Recovery reads rows before updating them. A heartbeat, claim, retry or cancellation made
/// after that read must win over a decision based on the old row.
async fn lock_snapshot<C: ConnectionTrait>(db: &C, turn: &bot_turn::Model) -> Result<bool, DbErr> {
    Ok(bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::UpdatedAt, Expr::col(bot_turn::Column::UpdatedAt))
        .filter(unchanged_turns(std::slice::from_ref(turn))).exec(db).await?.rows_affected == 1)
}

fn unchanged_turns(turns: &[bot_turn::Model]) -> Condition {
    turns.iter().fold(Condition::any(), |all, turn| {
        let mut unchanged = Condition::all()
            .add(bot_turn::Column::Id.eq(&turn.id))
            .add(bot_turn::Column::Status.eq(&turn.status))
            .add(bot_turn::Column::Attempts.eq(turn.attempts))
            .add(bot_turn::Column::UpdatedAt.eq(turn.updated_at));
        unchanged = match &turn.host_id {
            Some(host) => unchanged.add(bot_turn::Column::HostId.eq(host)),
            None => unchanged.add(bot_turn::Column::HostId.is_null()),
        };
        unchanged = match turn.lease_until {
            Some(lease) => unchanged.add(bot_turn::Column::LeaseUntil.eq(lease)),
            None => unchanged.add(bot_turn::Column::LeaseUntil.is_null()),
        };
        all.add(unchanged)
    })
}
