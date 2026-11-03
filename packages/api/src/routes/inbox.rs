//! Inboxes: conversations between people and bots, and every bot's history. Every principal (a
//! user sub or a bot id) owns an inbox, the `InboxEntry` rows addressed to it, and sending fans
//! one event out into the inbox of every active member in one transaction, at the conversation's
//! next position (`seq`), the same in every copy. Only active members read a conversation; an
//! owner reads a bot's inbox with `?as=<bot_id>` and sends as it only to answer a turn its host
//! claimed. In an `e2e` conversation the server stores one sealed envelope per member and never
//! sees content.

use crate::{
    db::coordination::coordinate,
    entity::{bot, conversation, conversation_member, inbox_entry},
    bot_caller,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::owned_bot,
    state::AppState,
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use chrono::{DateTime, FixedOffset, Timelike};
use flow_like_types::inbox::{InboxEntryKind, MAX_CONVERSATION_MEMBERS, MAX_ENVELOPE_BYTES};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, DbErr,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, SqlErr, Statement,
};

pub(crate) mod approvals;
pub mod conversations;
pub mod delivery;
pub mod entries;
pub(crate) mod exchange;
pub mod import;
pub mod invitations;
pub mod keys;
pub(crate) mod mail_threads;
pub mod members;
pub mod model;
pub(crate) mod person_push;
mod positions;
pub(crate) mod requests;
pub mod retirement;
pub mod segments;
mod targets;
pub(crate) mod triggers;
pub mod turns;
mod view;

#[cfg(test)]
mod bot_tests;
#[cfg(test)]
mod bot_token_tests;
#[cfg(all(test, feature = "db-chaos"))]
mod concurrency_tests;
#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod invitations_tests;
#[cfg(test)]
mod mail_threads_tests;
#[cfg(test)]
mod requests_tests;
#[cfg(test)]
mod tests;

pub(crate) const INBOX_MEMBER_NOT_ALLOWED: &str = "INBOX_MEMBER_NOT_ALLOWED";
pub(crate) const INBOX_MEMBER_LIMIT: &str = "INBOX_MEMBER_LIMIT";
pub(crate) const INBOX_DIRECT_FIXED: &str = "INBOX_DIRECT_FIXED";
pub(crate) const INBOX_REMOVE_NOT_ALLOWED: &str = "INBOX_REMOVE_NOT_ALLOWED";
pub(crate) const INBOX_MESSAGE_ID_TAKEN: &str = "INBOX_MESSAGE_ID_TAKEN";
pub(crate) const INBOX_ENVELOPES_MISMATCH: &str = "INBOX_ENVELOPES_MISMATCH";
pub(crate) const INBOX_TARGET_NOT_FOUND: &str = "INBOX_TARGET_NOT_FOUND";
pub(crate) const INBOX_NOT_YOUR_MESSAGE: &str = "INBOX_NOT_YOUR_MESSAGE";
pub(crate) const INBOX_KEY_EXISTS: &str = "INBOX_KEY_EXISTS";
pub(crate) const INBOX_KEY_LIMIT: &str = "INBOX_KEY_LIMIT";
pub(crate) const INBOX_EDIT_LIMIT: &str = "INBOX_EDIT_LIMIT";
pub(crate) const INBOX_HANDLE_NOT_ALLOWED: &str = "INBOX_HANDLE_NOT_ALLOWED";
pub(crate) const BOT_TURN_NOT_CLAIMED: &str = "BOT_TURN_NOT_CLAIMED";
pub(crate) const INBOX_INVITE_LIMIT: &str = "INBOX_INVITE_LIMIT";
pub(crate) const INBOX_INVITE_NOT_ALLOWED: &str = "INBOX_INVITE_NOT_ALLOWED";
pub(crate) const INBOX_REQUEST_LIMIT: &str = "INBOX_REQUEST_LIMIT";

pub(crate) const MEMBER_ACTIVE: &str = "ACTIVE";
pub(crate) const MEMBER_PENDING: &str = "PENDING";
pub(crate) const MEMBER_DECLINED: &str = "DECLINED";
pub(crate) const MEMBER_BLOCKED: &str = "BLOCKED";

/// Rows that read, receive and count: not left and not still an invitation.
pub(crate) fn active_member() -> Condition {
    Condition::all()
        .add(conversation_member::Column::LeftAt.is_null())
        .add(conversation_member::Column::Status.eq(MEMBER_ACTIVE))
}

/// Rows per insert statement, so a 50-member join fan-out never builds one huge statement.
const ENTRY_INSERT_CHUNK: usize = 500;

/// A sealed envelope per member plus JSON overhead fits in one send.
const SEND_BODY_LIMIT: usize = MAX_CONVERSATION_MEMBERS * (MAX_ENVELOPE_BYTES + 1024) + 64 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/conversations",
            get(conversations::list_conversations).post(conversations::create_conversation),
        )
        .route(
            "/conversations/{conversation_id}",
            get(conversations::get_conversation).patch(conversations::rename_conversation),
        )
        .route(
            "/conversations/{conversation_id}/members",
            post(members::add_members),
        )
        .route(
            "/conversations/{conversation_id}/members/{principal_id}",
            delete(members::remove_member),
        )
        .route(
            "/conversations/{conversation_id}/invitations",
            post(invitations::invite),
        )
        .route(
            "/conversations/{conversation_id}/invitations/{principal_id}",
            delete(invitations::cancel_invitation),
        )
        .route(
            "/conversations/{conversation_id}/members/{principal_id}/limits",
            patch(invitations::set_limits),
        )
        .route("/invitations", get(invitations::list_invitations))
        .route(
            "/invitations/{invitation_id}/accept",
            post(invitations::accept_invitation),
        )
        .route(
            "/invitations/{invitation_id}/decline",
            post(invitations::decline_invitation),
        )
        .route(
            "/invitations/{invitation_id}/block",
            post(requests::block_invitation),
        )
        .route(
            "/conversations/{conversation_id}/mute",
            patch(person_push::set_mute),
        )
        .route("/invitable", get(invitations::list_invitable))
        .route(
            "/conversations/{conversation_id}/entries",
            get(entries::list_entries)
                .post(entries::send_entry)
                .layer(DefaultBodyLimit::max(SEND_BODY_LIMIT)),
        )
        .route(
            "/conversations/{conversation_id}/read",
            post(entries::mark_read),
        )
        .route(
            "/conversations/{conversation_id}/turns/{turn_id}",
            get(crate::routes::bots::turns::get_conversation_turn),
        )
        .route(
            "/conversations/{conversation_id}/segments",
            get(segments::list_segments).post(segments::write_segment),
        )
        .route("/keys", get(keys::list_keys).put(keys::register_key))
        .route("/keys/{key_id}", delete(keys::revoke_key))
        .route(
            "/import",
            post(import::import_conversation)
                .layer(DefaultBodyLimit::max(import::IMPORT_BODY_LIMIT)),
        )
}

/// Now, at the millisecond precision the columns store, so a cursor computed before the insert
/// equals the one read back.
pub(crate) fn now_millis() -> DateTime<FixedOffset> {
    let now = chrono::Utc::now();
    now.with_nanosecond(now.nanosecond() / 1_000_000 * 1_000_000)
        .unwrap_or(now)
        .fixed_offset()
}

/// How a wire id is stored: uppercase.
pub(crate) fn stored(id: &str) -> String {
    id.to_ascii_uppercase()
}

fn conversation_not_found(conversation_id: &str) -> ApiError {
    ApiError::not_found(format!("Conversation '{conversation_id}' not found"))
}

/// Whose inbox a read shows: the caller's own, or one of their bots', every conversation the bot
/// is in included, so its owner follows the bot's exchanges with other bots. A person's bots
/// leave a group with them, so reading as a bot never reaches a group its owner left.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reader {
    inbox: String,
    owner: Option<String>,
}

impl Reader {
    fn own(sub: &str) -> Self {
        Self {
            inbox: sub.to_string(),
            owner: None,
        }
    }

    /// The bot whose inbox this is, when reading as one.
    fn bot(&self) -> Option<&str> {
        self.owner.as_ref().map(|_| self.inbox.as_str())
    }
}

/// A query value trimmed, `None` when empty.
fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The caller's own inbox, or with `as` the inbox of one of their live bots.
async fn reader<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    as_bot: Option<&str>,
) -> Result<Reader, ApiError> {
    match trimmed(as_bot) {
        None => Ok(Reader::own(sub)),
        Some(bot_id) => Ok(Reader {
            inbox: owned_bot(db, sub, bot_id).await?.id,
            owner: Some(sub.to_string()),
        }),
    }
}

/// [`reader`] for a person; a bot token reads only its own inbox, and in the one room it was minted for.
async fn acting_reader<C: ConnectionTrait>(
    db: &C,
    user: &AppUser,
    as_bot: Option<&str>,
    conversation_id: &str,
) -> Result<Reader, ApiError> {
    match user {
        AppUser::Bot(bot) => {
            let names = bot_caller::Names {
                as_bot,
                conversation: Some(conversation_id),
                turn: None,
            };
            bot_caller::check(bot, &names)?;
            Ok(Reader {
                inbox: bot.bot_id.clone(),
                owner: Some(bot_caller::live_owner(db, &bot.bot_id).await?),
            })
        }
        _ => reader(db, &user.sub()?, as_bot).await,
    }
}

/// The reader's membership must be active.
async fn reader_membership<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &Reader,
) -> Result<(), ApiError> {
    active_membership(db, conversation_id, &reader.inbox)
        .await
        .map(|_| ())
}

/// The inboxes a person's realtime fingerprint covers: their own and their live bots'.
pub(crate) fn watched_inboxes(column: impl ColumnTrait, sub: &str) -> Condition {
    let bots = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::OwnerId.eq(sub))
        .filter(bot::Column::DeletedAt.is_null())
        .into_query();
    Condition::any()
        .add(column.eq(sub))
        .add(column.in_subquery(bots))
}

/// The principal's membership if it is active; a conversation they left or were never in
/// reads as missing.
async fn active_membership<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<conversation_member::Model, ApiError> {
    conversation_member::Entity::find_by_id((conversation_id.to_string(), principal_id.to_string()))
        .filter(active_member())
        .one(db)
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))
}

/// The conversation `principal_id` is an active member of.
async fn member_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<conversation::Model, ApiError> {
    active_membership(db, conversation_id, principal_id).await?;
    conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))
}

/// Active members of the conversations, in the order they joined.
async fn active_members<C: ConnectionTrait>(
    db: &C,
    conversation_ids: &[String],
) -> Result<Vec<conversation_member::Model>, DbErr> {
    if conversation_ids.is_empty() {
        return Ok(Vec::new());
    }
    conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.is_in(conversation_ids.iter().cloned()))
        .filter(active_member())
        .order_by_asc(conversation_member::Column::JoinedAt)
        .order_by_asc(conversation_member::Column::PrincipalId)
        .all(db)
        .await
}

/// Serializes the writes on one conversation (sends, member changes) across replicas, so a
/// fan-out always covers exactly the members of the moment and the member cap stays exact.
async fn lock_conversation(txn: &DatabaseTransaction, conversation_id: &str) -> Result<(), DbErr> {
    coordinate(txn, "inbox-conversation", &[conversation_id]).await
}

/// One event about to be copied into several inboxes; every copy shares its time and position.
struct Event<'a> {
    conversation_id: &'a str,
    message_id: &'a str,
    sender_id: &'a str,
    kind: InboxEntryKind,
    created_at: DateTime<FixedOffset>,
    seq: i64,
}

impl Event<'_> {
    fn copy_for(&self, inbox_id: &str, envelope: flow_like_types::Value) -> inbox_entry::Model {
        inbox_entry::Model {
            id: flow_like_types::create_id(),
            inbox_id: inbox_id.to_string(),
            conversation_id: self.conversation_id.to_string(),
            message_id: self.message_id.to_string(),
            sender_id: self.sender_id.to_string(),
            kind: stored(self.kind.as_str()),
            envelope,
            created_at: self.created_at,
            seq: Some(self.seq),
        }
    }
}

const ALLOCATE_SEQS_SQL: &str = r#"UPDATE "Conversation" SET "lastSeq" = COALESCE("lastSeq", 0) + $2 WHERE "id" = $1 RETURNING "lastSeq""#;

/// Hands out the conversation's next `count` positions and returns the first. Positions are
/// never reused: the allocation commits or rolls back with the entries that carry it, and the
/// caller holds the conversation's lock or has just created it, so concurrent writers wait
/// (PostgreSQL) or retry (DSQL) instead of interleaving.
async fn allocate_seqs<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    count: usize,
) -> Result<i64, ApiError> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    let row = txn
        .query_one_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            ALLOCATE_SEQS_SQL,
            [conversation_id.into(), count.into()],
        ))
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))?;
    let last: i64 = row.try_get("", "lastSeq")?;
    Ok(last - count + 1)
}

/// Inserts the copies of one or more events. A message id that already exists in one of the
/// inboxes fails the whole fan-out with [`INBOX_MESSAGE_ID_TAKEN`].
async fn insert_copies<C: ConnectionTrait>(
    txn: &C,
    copies: Vec<inbox_entry::Model>,
) -> Result<(), ApiError> {
    let rows = copies
        .into_iter()
        .map(|copy| inbox_entry::ActiveModel::from(copy).reset_all())
        .collect::<Vec<_>>();
    crate::db::insert_chunked_in_txn(txn, rows, ENTRY_INSERT_CHUNK)
        .await
        .map(|_| ())
        .map_err(|error| match error.sql_err() {
            Some(SqlErr::UniqueConstraintViolation(_)) => message_id_taken(),
            _ => error.into(),
        })
}

fn message_id_taken() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_MESSAGE_ID_TAKEN,
        "This message id was already used for another event; send with a new one",
    )
}

/// Payload-free realtime wakeups for the people whose inbox changed, and for the owners of the
/// bots whose inbox did: their hosts read it with `?as=`.
pub(crate) async fn wake(state: &AppState, people: impl IntoIterator<Item = String>) {
    futures::future::join_all(people.into_iter().map(|sub| async move {
        state.channels.publish_realtime_invalidation(&sub).await;
    }))
    .await;
}
