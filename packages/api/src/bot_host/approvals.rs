//! Parked turns on the account: the stored tool grants, parking a running turn behind an approval
//! card, the owner's answer that queues it again and the seven-day limit of waiting. A parked
//! turn is `WAITING` with the card's id in `replyMessageId`; it holds no lease, so recovery
//! leaves it alone until the limit cancels it with a notice.

use super::ledger::{self, BOT_TURN_LEASE_LOST};
use crate::{
    bot_notify::display_name,
    entity::{bot, bot_tool_grant, bot_turn, conversation, inbox_entry},
    error::ApiError,
    push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent},
    routes::{
        bots::owned_bot,
        inbox::{
            approvals::{
                card_in, holds_sealed_card, lock, open_conversation, post_answer_in, post_card_in,
                post_notice_in, post_sealed_in, sealed_owner_chat,
            },
            entries::Sent,
            now_millis, wake,
        },
    },
    state::AppState,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like::flow::copilot::tool_grants::{GrantKey, GrantStore, record_batch_answer};
use flow_like_types::{
    bot_message::ApprovalRequest,
    bot_notify::{chat_link, group_key},
    bot_turn::{STORED_CANCELLED, STORED_QUEUED, STORED_RUNNING, STORED_WAITING},
    create_id,
    inbox::{ConversationEncryption, direct_pair_key, is_valid_message_id},
    tool_grant::{
        APPROVAL_WAIT_DAYS, ApprovalAnswer, DENY_PREFIX, ERROR_APPROVAL_EXPIRED,
        ERROR_APPROVAL_ITEM_UNKNOWN, GRANT_SLIDE_HOURS, GRANT_TTL_DAYS, ONCE_PREFIX,
        SealedApprovalCard, TOKEN_TTL_HOURS, ToolGrant, ToolGrantKey, card_matches_id, check_key,
        is_token_scope, sealed_approval_response_message_id,
    },
};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, OnConflict},
};
use serde_json::Value;
use std::{collections::BTreeMap, ops::Deref};

pub(crate) const BOT_TURN_NOT_WAITING: &str = "BOT_TURN_NOT_WAITING";
pub(crate) const BOT_APPROVAL_UNKNOWN: &str = "BOT_APPROVAL_UNKNOWN";
pub(crate) const BOT_APPROVAL_UNREACHABLE: &str = "BOT_APPROVAL_UNREACHABLE";
const EXPIRE_BATCH: u64 = 100;

fn token_cutoff(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    now - Duration::hours(TOKEN_TTL_HOURS)
}

fn grant_expiry(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    now + Duration::days(GRANT_TTL_DAYS)
}

fn slide_threshold(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    grant_expiry(now) - Duration::hours(GRANT_SLIDE_HOURS)
}

fn live(row: &bot_tool_grant::Model, now: DateTime<FixedOffset>) -> bool {
    row.expires_at.is_none_or(|expires_at| expires_at > now)
}

fn grant_of(row: bot_tool_grant::Model) -> ToolGrant {
    ToolGrant {
        tool: row.tool,
        scope: row.scope,
        granted_at: row.granted_at.to_rfc3339(),
        granted_by: row.granted_by,
        expires_at: row.expires_at.map(|expires_at| expires_at.to_rfc3339()),
    }
}

/// Moves the expiry of the grants a bot just used forward, at most once a day each. A grant that
/// is gone or already lapsed stays so: a use never brings one back.
pub(crate) async fn touch_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    keys: &[(String, String)],
    now: DateTime<FixedOffset>,
) -> Result<(), sea_orm::DbErr> {
    for (tool, scope) in keys {
        bot_tool_grant::Entity::update_many()
            .col_expr(
                bot_tool_grant::Column::ExpiresAt,
                Expr::value(grant_expiry(now)),
            )
            .filter(bot_tool_grant::Column::BotId.eq(bot_id))
            .filter(bot_tool_grant::Column::Tool.eq(tool.as_str()))
            .filter(bot_tool_grant::Column::Scope.eq(scope.as_str()))
            .filter(
                Condition::any()
                    .add(bot_tool_grant::Column::ExpiresAt.is_null())
                    .add(
                        Condition::all()
                            .add(bot_tool_grant::Column::ExpiresAt.gt(now))
                            .add(bot_tool_grant::Column::ExpiresAt.lt(slide_threshold(now))),
                    ),
            )
            .exec(db)
            .await?;
    }
    Ok(())
}

/// One bot's grants and answer tokens in the account database. `D` is a connection, a
/// transaction or an owner of one.
pub(crate) struct GrantRows<D> {
    db: D,
    bot_id: String,
    owner_id: String,
}

impl<D> GrantRows<D> {
    pub(crate) fn new(db: D, bot_id: &str, owner_id: &str) -> Self {
        Self {
            db,
            bot_id: bot_id.to_string(),
            owner_id: owner_id.to_string(),
        }
    }
}

#[async_trait]
impl<D, C> GrantStore for GrantRows<D>
where
    D: Deref<Target = C> + Send + Sync,
    C: ConnectionTrait + Send + Sync,
{
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        let now = now_millis();
        let cutoff = token_cutoff(now);
        let rows = bot_tool_grant::Entity::find()
            .filter(bot_tool_grant::Column::BotId.eq(self.bot_id.as_str()))
            .all(&*self.db)
            .await?;
        Ok(rows
            .into_iter()
            .filter(|row| live(row, now))
            .filter(|row| !is_token_scope(&row.scope) || row.granted_at >= cutoff)
            .map(|row| GrantKey::new(row.tool, row.scope))
            .collect())
    }

    async fn touch(&self, keys: &[GrantKey]) -> flow_like_types::Result<()> {
        let keys: Vec<(String, String)> = keys
            .iter()
            .map(|key| (key.tool.clone(), key.scope.clone()))
            .collect();
        touch_in(&*self.db, &self.bot_id, &keys, now_millis()).await?;
        Ok(())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        put_in(
            &*self.db,
            &self.bot_id,
            &self.owner_id,
            tool,
            scope,
            now_millis(),
        )
        .await?;
        Ok(())
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        if is_token_scope(scope) {
            bot_tool_grant::Entity::delete_many()
                .filter(bot_tool_grant::Column::BotId.eq(self.bot_id.as_str()))
                .filter(bot_tool_grant::Column::Tool.eq(tool))
                .filter(bot_tool_grant::Column::Scope.eq(scope))
                .filter(bot_tool_grant::Column::GrantedAt.lt(token_cutoff(now_millis())))
                .exec(&*self.db)
                .await?;
        }
        Ok(delete_in(&*self.db, &self.bot_id, tool, scope).await?)
    }
}

pub(crate) async fn put_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    owner_id: &str,
    tool: &str,
    scope: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), sea_orm::DbErr> {
    let existing = bot_tool_grant::Entity::find()
        .filter(bot_tool_grant::Column::BotId.eq(bot_id))
        .filter(bot_tool_grant::Column::Tool.eq(tool))
        .filter(bot_tool_grant::Column::Scope.eq(scope))
        .one(db)
        .await?;
    if let Some(existing) = existing {
        let stale_token = is_token_scope(scope) && existing.granted_at < token_cutoff(now);
        if stale_token || !live(&existing, now) {
            bot_tool_grant::Entity::update_many()
                .col_expr(bot_tool_grant::Column::GrantedAt, Expr::value(now))
                .col_expr(
                    bot_tool_grant::Column::ExpiresAt,
                    Expr::value(grant_expiry(now)),
                )
                .filter(bot_tool_grant::Column::Id.eq(existing.id))
                .exec(db)
                .await?;
        } else {
            touch_in(db, bot_id, &[(tool.to_string(), scope.to_string())], now).await?;
        }
        return Ok(());
    }
    bot_tool_grant::Entity::insert(bot_tool_grant::ActiveModel {
        id: Set(format!("tg_{}", create_id())),
        bot_id: Set(bot_id.to_string()),
        owner_id: Set(owner_id.to_string()),
        tool: Set(tool.to_string()),
        scope: Set(scope.to_string()),
        granted_at: Set(now),
        granted_by: Set(owner_id.to_string()),
        expires_at: Set(Some(grant_expiry(now))),
    })
    .on_conflict(
        OnConflict::columns([
            bot_tool_grant::Column::BotId,
            bot_tool_grant::Column::Tool,
            bot_tool_grant::Column::Scope,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

pub(crate) async fn delete_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    tool: &str,
    scope: &str,
) -> Result<bool, sea_orm::DbErr> {
    let removed = bot_tool_grant::Entity::delete_many()
        .filter(bot_tool_grant::Column::BotId.eq(bot_id))
        .filter(bot_tool_grant::Column::Tool.eq(tool))
        .filter(bot_tool_grant::Column::Scope.eq(scope))
        .exec(db)
        .await?;
    Ok(removed.rows_affected > 0)
}

/// The bot's grants for its owner, oldest first; answer tokens only for a host that asks.
pub(crate) async fn list_grants<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    with_tokens: bool,
) -> Result<Vec<ToolGrant>, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let now = now_millis();
    let cutoff = token_cutoff(now);
    let rows = bot_tool_grant::Entity::find()
        .filter(bot_tool_grant::Column::BotId.eq(bot_id))
        .filter(bot_tool_grant::Column::OwnerId.eq(owner_id))
        .order_by_asc(bot_tool_grant::Column::GrantedAt)
        .order_by_asc(bot_tool_grant::Column::Id)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| live(row, now))
        .filter(|row| !is_token_scope(&row.scope) || (with_tokens && row.granted_at >= cutoff))
        .map(grant_of)
        .collect())
}

fn checked(key: &ToolGrantKey) -> Result<(), ApiError> {
    check_key(key).map_err(ApiError::bad_request)
}

/// Stores a grant (again: nothing changes) and returns it.
pub(crate) async fn grant_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    key: &ToolGrantKey,
    now: DateTime<FixedOffset>,
) -> Result<ToolGrant, ApiError> {
    checked(key)?;
    owned_bot(db, owner_id, bot_id).await?;
    put_in(db, bot_id, owner_id, &key.tool, &key.scope, now).await?;
    bot_tool_grant::Entity::find()
        .filter(bot_tool_grant::Column::BotId.eq(bot_id))
        .filter(bot_tool_grant::Column::Tool.eq(key.tool.as_str()))
        .filter(bot_tool_grant::Column::Scope.eq(key.scope.as_str()))
        .one(db)
        .await?
        .map(grant_of)
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' not found")))
}

/// Takes a grant back; false when there was none.
pub(crate) async fn revoke_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    key: &ToolGrantKey,
) -> Result<bool, ApiError> {
    checked(key)?;
    owned_bot(db, owner_id, bot_id).await?;
    Ok(delete_in(db, bot_id, &key.tool, &key.scope).await?)
}

fn conflict(code: &str, message: String) -> ApiError {
    ApiError::coded(StatusCode::CONFLICT, code, message)
}

async fn park_transaction(
    txn: &sea_orm::DatabaseTransaction,
    turn: &bot_turn::Model,
    host_id: &str,
    card: &ApprovalRequest,
) -> Result<Sent, ApiError> {
    let now = now_millis();
    let conversation = open_conversation(
        txn,
        &turn.owner_id,
        &turn.bot_id,
        &turn.conversation_id,
        now,
    )
    .await?
    .ok_or_else(|| {
        conflict(
            BOT_APPROVAL_UNREACHABLE,
            "The owner's conversation with this bot is end-to-end encrypted: the server cannot post an approval card into it, so the bot's host has to seal the card itself (update the app that runs the bot)".to_string(),
        )
    })?;
    lock(txn, &conversation).await?;
    park_in(txn, turn, host_id, card, &conversation, now).await
}

async fn park_sealed_transaction(
    txn: &sea_orm::DatabaseTransaction,
    turn: &bot_turn::Model,
    host_id: &str,
    sealed: &SealedApprovalCard,
) -> Result<Sent, ApiError> {
    let now = now_millis();
    let conversation = sealed_owner_chat(txn, &turn.owner_id, &turn.bot_id, &turn.conversation_id)
        .await?
        .ok_or_else(|| {
            conflict(
                BOT_APPROVAL_UNREACHABLE,
                format!(
                    "Turn '{}' does not run in the owner's end-to-end encrypted direct chat with the bot, the only place a sealed approval card can go",
                    turn.id
                ),
            )
        })?;
    lock(txn, &conversation).await?;
    park_sealed_in(txn, turn, host_id, sealed, &conversation, now).await
}

/// Sets the turn `host_id` holds `WAITING` and posts the sealed card in `conversation`, the
/// owner's encrypted chat whose lock the caller holds.
pub(crate) async fn park_sealed_in<C: ConnectionTrait>(
    txn: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    sealed: &SealedApprovalCard,
    conversation: &str,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    wait_in(txn, turn, host_id, &sealed.approval_id, now).await?;
    post_sealed_in(
        txn,
        conversation,
        &turn.bot_id,
        u32::try_from(turn.hops).unwrap_or(0),
        &sealed.approval_id,
        &sealed.envelopes,
        now,
    )
    .await
}

/// The card with whose message woke the turn and the room it runs in, so the owner knows who
/// they are approving for.
async fn named_requester<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    card: &ApprovalRequest,
) -> Result<ApprovalRequest, ApiError> {
    let mut card = card.clone();
    let Some(requester) = turn.requester_id.as_deref() else {
        return Ok(card);
    };
    let names = super::history::sender_names(db, &[requester]).await?;
    card.requester_id = Some(requester.to_string());
    card.requester_name = names.get(requester).cloned();
    card.room = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(db)
        .await?
        .filter(|row| row.kind.eq_ignore_ascii_case("group"))
        .and_then(|row| row.title);
    Ok(card)
}

/// Sets the turn `host_id` holds `WAITING` and posts the card in `conversation`, which the caller
/// holds the lock of.
pub(crate) async fn park_in<C: ConnectionTrait>(
    txn: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    card: &ApprovalRequest,
    conversation: &str,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let card = &named_requester(txn, turn, card).await?;
    wait_in(txn, turn, host_id, &card.id, now).await?;
    post_card_in(
        txn,
        conversation,
        &turn.bot_id,
        u32::try_from(turn.hops).unwrap_or(0),
        card,
        now,
    )
    .await
}

/// Sets the turn `host_id` holds `WAITING` behind the card `approval_id`, without a lease.
async fn wait_in<C: ConnectionTrait>(
    txn: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    approval_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let parked = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_WAITING))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::LeaseUntil,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(bot_turn::Column::ReplyMessageId, Expr::value(approval_id))
        .col_expr(bot_turn::Column::ErrorCode, Expr::value(None::<String>))
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn.id.as_str()))
        .filter(bot_turn::Column::Status.eq(STORED_RUNNING))
        .filter(bot_turn::Column::HostId.eq(host_id))
        .filter(bot_turn::Column::LeaseUntil.gt(now))
        .exec(txn)
        .await?;
    if parked.rows_affected == 0 {
        return Err(conflict(
            BOT_TURN_LEASE_LOST,
            format!("Turn '{}' is not running under host '{host_id}'", turn.id),
        ));
    }
    Ok(())
}

/// Parks the turn `host_id` holds: posts the card as the bot, sets the turn `WAITING` without a
/// lease, and tells the owner (inbox wake and a push "needs your approval").
pub(crate) async fn park_held(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    card: &ApprovalRequest,
) -> Result<(), ApiError> {
    let sent = state
        .transaction(|txn| {
            let (turn, host_id, card) = (turn.clone(), host_id.to_string(), card.clone());
            Box::pin(async move { park_transaction(txn, &turn, &host_id, &card).await })
        })
        .await?;
    wake(state, sent.wake).await;
    notify_owner(state, turn, &CardNotice::of(card)).await;
    Ok(())
}

/// Parks the turn `host_id` holds behind a card the host sealed for the owner's end-to-end
/// encrypted chat: the envelopes are posted as the bot's message unread, and the push says only
/// that the bot is waiting.
pub(crate) async fn park_sealed(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    sealed: &SealedApprovalCard,
) -> Result<(), ApiError> {
    if !sealed.approval_id.starts_with("ap-") || !is_valid_message_id(&sealed.approval_id) {
        return Err(ApiError::bad_request(format!(
            "sealed.approval_id '{}' must be a card id ('ap-' and its hash)",
            sealed.approval_id.chars().take(80).collect::<String>()
        )));
    }
    let sent = state
        .transaction(|txn| {
            let (turn, host_id, sealed) = (turn.clone(), host_id.to_string(), sealed.clone());
            Box::pin(async move { park_sealed_transaction(txn, &turn, &host_id, &sealed).await })
        })
        .await?;
    wake(state, sent.wake).await;
    notify_owner(state, turn, &CardNotice::sealed(&sealed.approval_id)).await;
    Ok(())
}

/// What the "needs your approval" push may say about a card.
struct CardNotice<'a> {
    id: &'a str,
    /// The card's line; `None` for a sealed card the server can't read.
    summary: Option<&'a str>,
    /// Only the card itself can answer it, not the lock screen.
    review_only: bool,
}

impl<'a> CardNotice<'a> {
    /// A secret, a batch, a call asked again after web content or several identical calls need
    /// the card itself; only a plain card for one call can be answered from the lock screen.
    fn of(card: &'a ApprovalRequest) -> Self {
        Self {
            id: &card.id,
            summary: Some(&card.summary),
            review_only: card.secret.is_some() || !card.items.is_empty(),
        }
    }

    fn sealed(id: &'a str) -> Self {
        Self {
            id,
            summary: None,
            review_only: true,
        }
    }
}

const SEALED_CARD_PREVIEW: &str = "Open the chat to see what it asks";

/// The card waits in the owner's chat with the bot, so the push opens that chat and groups
/// under the bot like its other notifications.
async fn notify_owner(state: &AppState, turn: &bot_turn::Model, card: &CardNotice<'_>) {
    let name = bot::Entity::find_by_id(&turn.bot_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|row| display_name(&row))
        .unwrap_or_else(|| "Your bot".to_string());
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&turn.owner_id))
        .filter(inbox_entry::Column::SenderId.eq(&turn.bot_id))
        .filter(inbox_entry::Column::MessageId.eq(card.id))
        .one(&state.db)
        .await
        .ok()
        .flatten();
    let expires_at = entry
        .as_ref()
        .map_or_else(now_millis, |entry| entry.created_at)
        + Duration::days(APPROVAL_WAIT_DAYS);
    let encryption = if card.summary.is_some() {
        ConversationEncryption::Plain
    } else {
        ConversationEncryption::E2e
    };
    let sent = dispatch_notification_idempotent(
        state,
        &format!("botapproval_{}", card.id),
        DispatchNotificationInput {
            notification_priority: Default::default(),
            bot_context: Some(flow_like_types::bot_notify::BotNotificationContext {
                bot_id: turn.bot_id.clone(),
                review_only: card.review_only,
                turn_id: Some(turn.id.clone()),
                approval_id: (!card.review_only).then(|| card.id.to_string()),
                conversation_id: entry.as_ref().map(|entry| entry.conversation_id.clone()),
                conversation_encryption: Some(encryption.as_str().to_string()),
                message_id: Some(card.id.to_string()),
                expires_at: Some(expires_at.to_rfc3339()),
                ..Default::default()
            }),
            user_id: turn.owner_id.clone(),
            app_id: None,
            title: format!("{name} needs your approval"),
            description: Some(card.summary.unwrap_or(SEALED_CARD_PREVIEW).to_string()),
            icon: None,
            image: None,
            link: Some(chat_link(&turn.bot_id)),
            notification_type: crate::entity::sea_orm_active_enums::NotificationType::System,
            source_run_id: Some(group_key(&turn.bot_id)),
            source_node_id: Some(card.id.to_string()),
        },
    )
    .await;
    if let Err(error) = sent {
        tracing::warn!(%error, turn_id = %turn.id, "Bot turn: the approval notification was not sent");
    }
}

async fn answer_transaction(
    txn: &sea_orm::DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    answer: &ApprovalAnswer,
    sealed: Option<&SealedAnswer>,
) -> Result<(bot_turn::Model, Sent), ApiError> {
    let now = now_millis();
    owned_bot(txn, owner_id, bot_id).await?;
    let turn = ledger::find(txn, turn_id, Some(owner_id))
        .await?
        .filter(|turn| turn.bot_id == bot_id)
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    let (conversation, source) = match open_conversation(
        txn,
        owner_id,
        bot_id,
        &turn.conversation_id,
        now,
    )
    .await?
    {
        Some(conversation) => (conversation, CardSource::Posted),
        None => {
            let conversation = sealed_owner_chat(txn, owner_id, bot_id, &turn.conversation_id)
                .await?
                .ok_or_else(|| unknown_approval(turn_id, &answer.approval_id))?;
            let sealed = sealed.ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "Approval '{}' waits in an end-to-end encrypted chat: answer it from a device that opened the card, sending `card` and the sealed answer in `envelopes`",
                        answer.approval_id
                    ))
                })?;
            (conversation, CardSource::Sealed(sealed))
        }
    };
    let origin = super::talk::turn_origin_conversation(txn, &turn).await?;
    let mut rooms = vec![conversation.as_str(), origin.as_str()];
    rooms.sort_unstable();
    rooms.dedup();
    for room in rooms {
        lock(txn, room).await?;
    }
    settle_in(
        txn,
        owner_id,
        bot_id,
        turn_id,
        answer,
        &conversation,
        source,
        now,
    )
    .await
}

/// What the owner's device sends beside an answer to a card in an end-to-end encrypted chat.
#[derive(Debug, Clone)]
pub(crate) struct SealedAnswer {
    /// The card as the device opened it; the server checks it against the card id.
    pub card: ApprovalRequest,
    /// The answer entry sealed to every member's keys, under
    /// [`sealed_approval_response_message_id`].
    pub envelopes: BTreeMap<String, Value>,
}

/// Where the card an answer settles comes from.
#[derive(Clone, Copy)]
enum CardSource<'a> {
    /// The server posted it in a plain chat: read back from the owner's copy.
    Posted,
    /// A host sealed it: the copy the owner's device opened and sent.
    Sealed(&'a SealedAnswer),
}

fn unknown_approval(turn_id: &str, approval_id: &str) -> ApiError {
    conflict(
        BOT_APPROVAL_UNKNOWN,
        format!("Turn '{turn_id}' waits for another approval than '{approval_id}'"),
    )
}

/// Stores what the owner answered, records the answer in `conversation` (whose lock the caller
/// holds) and queues the waiting turn again.
pub(crate) async fn answer_in<C: ConnectionTrait + Send + Sync>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    answer: &ApprovalAnswer,
    conversation: &str,
    now: DateTime<FixedOffset>,
) -> Result<(bot_turn::Model, Sent), ApiError> {
    settle_in(
        txn,
        owner_id,
        bot_id,
        turn_id,
        answer,
        conversation,
        CardSource::Posted,
        now,
    )
    .await
}

/// [`answer_in`] for a card a host sealed in the owner's end-to-end encrypted chat.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn answer_sealed_in<C: ConnectionTrait + Send + Sync>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    answer: &ApprovalAnswer,
    conversation: &str,
    sealed: &SealedAnswer,
    now: DateTime<FixedOffset>,
) -> Result<(bot_turn::Model, Sent), ApiError> {
    settle_in(
        txn,
        owner_id,
        bot_id,
        turn_id,
        answer,
        conversation,
        CardSource::Sealed(sealed),
        now,
    )
    .await
}

/// The sealed card the owner's device sent, when it is the bot's card the turn waits on.
async fn opened_card<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    turn: &bot_turn::Model,
    approval_id: &str,
    conversation: &str,
    sealed: &SealedAnswer,
) -> Result<ApprovalRequest, ApiError> {
    let card = &sealed.card;
    if card.id != approval_id || !card_matches_id(card, &turn.id) {
        return Err(ApiError::bad_request(format!(
            "The card sent with the answer is not approval '{approval_id}' of turn '{}': its id must follow from the turn and its calls",
            turn.id
        )));
    }
    flow_like_types::bot_message::MessageBody {
        approval: Some(card.clone()),
        ..Default::default()
    }
    .check()
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if !holds_sealed_card(db, owner_id, &turn.bot_id, conversation, approval_id).await? {
        return Err(unknown_approval(&turn.id, approval_id));
    }
    Ok(card.clone())
}

#[allow(clippy::too_many_arguments)]
async fn settle_in<C: ConnectionTrait + Send + Sync>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    answer: &ApprovalAnswer,
    conversation: &str,
    source: CardSource<'_>,
    now: DateTime<FixedOffset>,
) -> Result<(bot_turn::Model, Sent), ApiError> {
    owned_bot(txn, owner_id, bot_id).await?;
    let turn = ledger::find(txn, turn_id, Some(owner_id))
        .await?
        .filter(|turn| turn.bot_id == bot_id)
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    if turn.status != STORED_WAITING {
        return Err(conflict(
            BOT_TURN_NOT_WAITING,
            format!(
                "Turn '{turn_id}' is {}, not waiting for an approval",
                turn.status.to_ascii_lowercase()
            ),
        ));
    }
    if turn.reply_message_id.as_deref() != Some(answer.approval_id.as_str()) {
        return Err(unknown_approval(turn_id, &answer.approval_id));
    }
    if turn.updated_at <= now - Duration::days(APPROVAL_WAIT_DAYS) {
        return Err(conflict(
            ERROR_APPROVAL_EXPIRED,
            "This approval has expired".to_string(),
        ));
    }
    let card = match source {
        CardSource::Posted => card_in(txn, owner_id, conversation, &answer.approval_id)
            .await?
            .filter(|card| card.turn_id == turn.id)
            .ok_or_else(|| unknown_approval(turn_id, &answer.approval_id))?,
        CardSource::Sealed(sealed) => {
            opened_card(
                txn,
                owner_id,
                &turn,
                &answer.approval_id,
                conversation,
                sealed,
            )
            .await?
        }
    };
    card.decisions(answer.decision, &answer.items)
        .map_err(|message| {
            ApiError::coded(
                StatusCode::BAD_REQUEST,
                ERROR_APPROVAL_ITEM_UNKNOWN,
                message,
            )
        })?;
    let store = GrantRows::new(txn, bot_id, owner_id);
    let response = record_batch_answer(&store, &card, answer)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let sent = match source {
        CardSource::Posted => post_answer_in(txn, conversation, owner_id, &response, now).await?,
        CardSource::Sealed(sealed) => {
            post_sealed_in(
                txn,
                conversation,
                owner_id,
                0,
                &sealed_approval_response_message_id(&answer.approval_id),
                &sealed.envelopes,
                now,
            )
            .await?
        }
    };
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
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn.id.as_str()))
        .filter(bot_turn::Column::Status.eq(STORED_WAITING))
        .exec(txn)
        .await?;
    if queued.rows_affected == 0 {
        return Err(conflict(
            BOT_TURN_NOT_WAITING,
            format!("Turn '{turn_id}' stopped waiting"),
        ));
    }
    super::recovery::rebase_deadline_in(txn, &turn.id, now).await?;
    let origin = super::talk::turn_origin_conversation(txn, &turn).await?;
    super::decision_wait::clear_before_in(txn, bot_id, &origin, now).await?;
    let turn = ledger::find(txn, turn_id, Some(owner_id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    Ok((turn, sent))
}

/// The owner's answer to the card a turn waits on: stores what it says, writes the answer entry
/// and queues the turn again. A repeat finds the turn no longer waiting.
pub(crate) async fn answer(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    body: &ApprovalAnswer,
    sealed: Option<&SealedAnswer>,
) -> Result<bot_turn::Model, ApiError> {
    let (turn, sent) = state
        .transaction(|txn| {
            let (owner, bot, turn, body, sealed) = (
                owner_id.to_string(),
                bot_id.to_string(),
                turn_id.to_string(),
                body.clone(),
                sealed.cloned(),
            );
            Box::pin(async move {
                answer_transaction(txn, &owner, &bot, &turn, &body, sealed.as_ref()).await
            })
        })
        .await?;
    wake(state, sent.wake).await;
    super::dispatch_turns(state, vec![turn.id.clone()]).await;
    Ok(turn)
}

/// Cancels the turn and posts a notice after its pending approval in the owner's chat.
pub(crate) async fn cancel_in<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(bot_turn::Model, Option<Sent>), ApiError> {
    let turn = ledger::cancel_owned_in(txn, owner_id, bot_id, turn_id, now).await?;
    let Some(approval_id) = turn.reply_message_id.as_deref() else {
        return Ok((turn, None));
    };
    let card = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(owner_id))
        .filter(inbox_entry::Column::SenderId.eq(bot_id))
        .filter(inbox_entry::Column::MessageId.eq(approval_id))
        .one(txn)
        .await?
        .filter(|entry| entry.envelope["body"]["approval"]["turn_id"].as_str() == Some(turn_id));
    let Some(card) = card else {
        return Ok((turn, None));
    };
    let sent = post_notice_in(
        txn,
        &card.conversation_id,
        bot_id,
        &format!("cancel-{}", turn.id),
        "You cancelled this task. I no longer need your approval.",
        now.max(card.created_at + Duration::milliseconds(1)),
    )
    .await?;
    Ok((turn, Some(sent)))
}

/// Serializes cancellation with approval posting and answering in the owner's direct chat.
pub(crate) async fn cancel(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<bot_turn::Model, ApiError> {
    let (turn, sent) = state
        .transaction(|txn| {
            let (owner, bot, turn) = (
                owner_id.to_string(),
                bot_id.to_string(),
                turn_id.to_string(),
            );
            Box::pin(async move {
                let direct = conversation::Entity::find()
                    .filter(conversation::Column::DirectKey.eq(direct_pair_key(&owner, &bot)))
                    .one(txn)
                    .await?;
                if let Some(direct) = direct {
                    lock(txn, &direct.id).await?;
                }
                cancel_in(txn, &owner, &bot, &turn, now_millis()).await
            })
        })
        .await?;
    if let Some(sent) = sent {
        wake(state, sent.wake).await;
    }
    Ok(turn)
}

/// Cancels one turn that waited too long and posts the bot's notice in `conversation`.
pub(crate) async fn expire_in<C: ConnectionTrait>(
    txn: &C,
    turn: &bot_turn::Model,
    conversation: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<Option<Sent>, ApiError> {
    let ended = bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_CANCELLED))
        .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::ErrorCode,
            Expr::value(ERROR_APPROVAL_EXPIRED),
        )
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
        .filter(bot_turn::Column::Id.eq(turn.id.as_str()))
        .filter(bot_turn::Column::Status.eq(STORED_WAITING))
        .filter(bot_turn::Column::UpdatedAt.lt(now - Duration::days(APPROVAL_WAIT_DAYS)))
        .exec(txn)
        .await?;
    if ended.rows_affected == 0 {
        return Ok(None);
    }
    super::recovery::clear_in(txn, &turn.id).await?;
    let Some(conversation) = conversation else {
        return Ok(None);
    };
    post_notice_in(
        txn,
        conversation,
        &turn.bot_id,
        &flow_like_types::bot_turn::failure_message_id(&turn.bot_id, &turn.trigger_message_id),
        EXPIRED_NOTICE,
        now,
    )
    .await
    .map(Some)
}

const EXPIRED_NOTICE: &str =
    "I waited seven days for your approval and stopped. Ask me again if you still want this done.";

async fn expire_one(state: &AppState, turn: &bot_turn::Model, now: DateTime<FixedOffset>) -> bool {
    let result = state
        .transaction(|txn| {
            let turn = turn.clone();
            Box::pin(async move {
                let conversation = open_conversation(
                    txn,
                    &turn.owner_id,
                    &turn.bot_id,
                    &turn.conversation_id,
                    now,
                )
                .await?;
                if let Some(conversation) = &conversation {
                    lock(txn, conversation).await?;
                }
                let ended = expire_in(txn, &turn, conversation.as_deref(), now).await?;
                Ok::<_, ApiError>(ended.map(|sent| sent.wake))
            })
        })
        .await;
    match result {
        Ok(Some(people)) => {
            wake(state, people).await;
            true
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(%error, turn_id = %turn.id, "Bot turn: an abandoned approval was not cancelled");
            false
        }
    }
}

/// What one pass over waiting turns and answer tokens changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Expired {
    pub turns: u64,
    pub tokens: u64,
    pub grants: u64,
}

/// Cancels turns that waited longer than [`APPROVAL_WAIT_DAYS`], with a notice from the bot, and
/// drops answer tokens older than [`TOKEN_TTL_HOURS`] and standing grants past their expiry.
pub(crate) async fn expire(
    state: &AppState,
    now: DateTime<FixedOffset>,
) -> Result<Expired, ApiError> {
    let stale = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_WAITING))
        .filter(bot_turn::Column::UpdatedAt.lt(now - Duration::days(APPROVAL_WAIT_DAYS)))
        .limit(EXPIRE_BATCH)
        .all(&state.db)
        .await?;
    let mut turns = 0;
    for turn in &stale {
        if expire_one(state, turn, now).await {
            turns += 1;
        }
    }
    let tokens = bot_tool_grant::Entity::delete_many()
        .filter(bot_tool_grant::Column::GrantedAt.lt(token_cutoff(now)))
        .filter(
            Condition::any()
                .add(bot_tool_grant::Column::Scope.starts_with(ONCE_PREFIX))
                .add(bot_tool_grant::Column::Scope.starts_with(DENY_PREFIX)),
        )
        .exec(&state.db)
        .await?
        .rows_affected;
    let grants = sweep_grants(&state.db, now).await?;
    Ok(Expired {
        turns,
        tokens,
        grants,
    })
}

/// Deletes the grants that lapsed and stamps the ones that predate the expiry with a full term.
pub(crate) async fn sweep_grants<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<u64, sea_orm::DbErr> {
    let lapsed = bot_tool_grant::Entity::delete_many()
        .filter(bot_tool_grant::Column::ExpiresAt.lte(now))
        .exec(db)
        .await?
        .rows_affected;
    bot_tool_grant::Entity::update_many()
        .col_expr(
            bot_tool_grant::Column::ExpiresAt,
            Expr::value(grant_expiry(now)),
        )
        .filter(bot_tool_grant::Column::ExpiresAt.is_null())
        .exec(db)
        .await?;
    Ok(lapsed)
}
