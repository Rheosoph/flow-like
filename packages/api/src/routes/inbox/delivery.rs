//! Bots among the recipients of an entry. Every copy written into a bot's inbox wakes its owner,
//! whose host reads it with `?as=`; a sent entry also hands the bots that received it to
//! [`deliver_to_bots`], inside the transaction that wrote it, where their hosts' turns are
//! queued.

use super::members::Principal;
use crate::{entity::bot, error::ApiError};
use flow_like_types::{
    bot::BotLocation,
    inbox::{ConversationKind, InboxEntryKind, PrincipalKind},
};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter};
use std::collections::HashSet;

/// A bot member that received a copy of a sent entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BotRecipient {
    pub bot_id: String,
    pub owner_id: String,
    pub location: BotLocation,
    pub device_id: Option<String>,
    /// The entry names the bot in `mentions`.
    pub mentioned: bool,
    /// The entry replies to a message the bot sent.
    pub replied_to: bool,
    pub conversation_id: String,
    /// The entry's message id: what a turn for it answers.
    pub trigger_message_id: String,
    /// Bot turns that led to the entry: 0 from a person, the answered turn's hops + 1 from a bot.
    pub hops: u32,
    /// An interactive chat answers the entry itself and holds its turn.
    pub interactive: bool,
}

/// One sent entry as the bots among its recipients see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BotDelivery {
    pub conversation_kind: ConversationKind,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    pub seq: i64,
    /// Never empty: an entry no bot received isn't delivered.
    pub recipients: Vec<BotRecipient>,
}

/// Where bot hosts learn about a sent entry, inside the transaction that wrote it: turns for the
/// recipients are queued here, so an entry and its turns commit together. Owners are woken after
/// the commit either way.
pub(crate) async fn deliver_to_bots<C: ConnectionTrait>(
    txn: &C,
    delivery: &BotDelivery,
) -> Result<(), ApiError> {
    crate::bot_host::enqueue_in(txn, delivery, super::now_millis()).await?;
    Ok(())
}

/// The live bots among `principals`, in their order, found in one query.
pub(super) async fn live_bots<C: ConnectionTrait>(
    db: &C,
    principals: &[Principal],
) -> Result<Vec<bot::Model>, DbErr> {
    let ids: Vec<&str> = principals
        .iter()
        .filter(|principal| principal.kind == PrincipalKind::Bot)
        .map(|principal| principal.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut bots = bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids.iter().copied()))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    bots.sort_by_key(|bot| ids.iter().position(|id| *id == bot.id));
    Ok(bots)
}

/// Whom a write into these principals' inboxes wakes: the people among them, then the owners of
/// the live bots among them, each once.
pub(super) fn woken(principals: &[Principal], bots: &[bot::Model]) -> Vec<String> {
    let mut seen = HashSet::new();
    principals
        .iter()
        .filter(|principal| principal.kind == PrincipalKind::User)
        .map(|principal| principal.id.clone())
        .chain(bots.iter().map(|bot| bot.owner_id.clone()))
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// [`woken`] for principals whose bots aren't loaded yet.
pub(super) async fn people_to_wake<C: ConnectionTrait>(
    db: &C,
    principals: &[Principal],
) -> Result<Vec<String>, DbErr> {
    let bots = live_bots(db, principals).await?;
    Ok(woken(principals, &bots))
}

/// What one sent entry says to the bots that received it.
pub(super) struct Addressed<'a> {
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub sender_id: &'a str,
    pub mentions: &'a [String],
    /// The sender of the message the entry replies to.
    pub replied_sender: Option<&'a str>,
    pub hops: u32,
    pub interactive: bool,
}

/// Whether a plain envelope's body carries the `interactive` mark of a chat that answers it.
pub(crate) fn is_interactive(envelope: &serde_json::Value) -> bool {
    envelope
        .get("body")
        .and_then(|body| body.get("interactive"))
        .and_then(|mark| mark.get("client_run_id"))
        .is_some_and(|id| id.as_str().is_some_and(|id| !id.is_empty()))
}

/// The live bots among the recipients, the sender excluded, in member order.
pub(super) fn bot_recipients(bots: &[bot::Model], entry: &Addressed) -> Vec<BotRecipient> {
    bots.iter()
        .filter(|bot| bot.id != entry.sender_id)
        .map(|bot| BotRecipient {
            bot_id: bot.id.clone(),
            owner_id: bot.owner_id.clone(),
            location: BotLocation::parse(&bot.location).unwrap_or_default(),
            device_id: bot.device_id.clone(),
            mentioned: entry.mentions.contains(&bot.id),
            replied_to: entry.replied_sender == Some(bot.id.as_str()),
            conversation_id: entry.conversation_id.to_string(),
            trigger_message_id: entry.message_id.to_string(),
            hops: entry.hops,
            interactive: entry.interactive,
        })
        .collect()
}
