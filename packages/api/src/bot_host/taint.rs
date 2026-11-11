//! What a turn read and what it passes on (`todo/bots-talk.md`, Step 2). A bot's reply records the
//! taint the turn ended with in `body.taint`; the next turn that reads the message starts from it
//! together with the classification of the bot that wrote it and the origin of mail and webhook
//! triggers. The room-mention gate applies the `ask_bot` rules to a bot's message that names
//! another bot.

use super::ledger::Waking;
use crate::{
    entity::{bot, inbox_entry},
    error::ApiError,
    routes::inbox::approvals::post_notice_in,
};
use chrono::{DateTime, FixedOffset};
use flow_like::flow::copilot::{
    TurnReads,
    bot_talk::{TalkGate, classification_gate},
};
use flow_like_types::{
    bot_turn::classification_refused_message_id,
    data_classification::{DataClassification, DataSensitivity},
    inbox::{InboxEntryKind, PLAIN_ENCRYPTION, TriggerBody, TriggerSource, is_tombstone},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) const TAINT_FIELD: &str = "taint";

pub(crate) fn bot_label(row: &bot::Model) -> DataSensitivity {
    DataSensitivity {
        classification: row
            .classification
            .as_deref()
            .and_then(DataClassification::parse),
        pii: row.pii,
    }
}

/// The taint a message body records; empty when it records none.
pub(crate) fn recorded(body: &Value) -> DataSensitivity {
    body.get(TAINT_FIELD)
        .and_then(|taint| serde_json::from_value(taint.clone()).ok())
        .unwrap_or_default()
}

/// Records `taint` on a reply body; an empty taint leaves the body as it is.
pub(crate) fn attach(body: &mut Value, taint: &DataSensitivity) {
    if !taint.is_empty() {
        body[TAINT_FIELD] = json!(taint);
    }
}

fn plain_body(entry: &inbox_entry::Model) -> Option<&Value> {
    (entry.envelope.get("enc").and_then(Value::as_str) == Some(PLAIN_ENCRYPTION)
        && !is_tombstone(&entry.envelope))
    .then(|| entry.envelope.get("body"))
    .flatten()
}

fn is_external_trigger(body: &Value) -> bool {
    TriggerBody::parse(body).is_ok_and(|trigger| {
        matches!(
            trigger.source,
            TriggerSource::Email | TriggerSource::Webhook
        )
    })
}

/// What reading `window` (oldest first, as the turn reads it) puts into the turn of `bot_id`.
/// Entries before the last "New topic" marker are not read.
pub(crate) async fn reads_of<C: ConnectionTrait>(
    db: &C,
    window: &[inbox_entry::Model],
    bot_id: &str,
) -> Result<TurnReads, ApiError> {
    let since = window
        .iter()
        .rposition(|entry| InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Topic))
        .map_or(0, |marker| marker + 1);
    let mut reads = TurnReads::default();
    let mut senders: HashSet<&str> = HashSet::new();
    for entry in &window[since..] {
        let Some(body) = plain_body(entry) else {
            continue;
        };
        reads.recorded = reads.recorded.combine(recorded(body));
        if entry.sender_id != bot_id {
            senders.insert(entry.sender_id.as_str());
        }
        if InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Trigger) {
            reads.external_trigger |= is_external_trigger(body);
        }
    }
    if !senders.is_empty() {
        let rows = bot::Entity::find()
            .filter(bot::Column::Id.is_in(senders))
            .all(db)
            .await?;
        reads.bot_senders = rows.iter().map(bot_label).collect();
    }
    Ok(reads)
}

/// The rule behind a refused wake, or `None` when `sender` may wake `recipient`.
pub(crate) fn wake_refusal(
    sender: &DataSensitivity,
    taint: &DataSensitivity,
    recipient: &DataSensitivity,
) -> Option<String> {
    match classification_gate(sender, taint, recipient) {
        TalkGate::Allow => None,
        TalkGate::Refuse(reason) => Some(reason),
    }
}

/// The recipients a bot's message may wake. Anything but a mention passes: a person's message,
/// replies and read-along are not an ask. A refused recipient gets no turn, the message stays, and
/// the recipient says in the room that it was not woken.
pub(crate) async fn admit_mentions<'a, C: ConnectionTrait>(
    db: &C,
    delivery_sender: &str,
    message_id: &str,
    conversation_id: &str,
    waking: Vec<Waking<'a>>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Waking<'a>>, ApiError> {
    if !waking.iter().any(|item| item.recipient.mentioned) {
        return Ok(waking);
    }
    let Some(sender) = bot::Entity::find_by_id(delivery_sender).one(db).await? else {
        return Ok(waking);
    };
    let taint = match inbox_entry::Entity::find()
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .one(db)
        .await?
    {
        Some(entry) => plain_body(&entry).map(recorded).unwrap_or_default(),
        None => DataSensitivity::default(),
    };
    let sender = bot_label(&sender);
    let ids: Vec<&str> = waking
        .iter()
        .filter(|item| item.recipient.mentioned)
        .map(|item| item.recipient.bot_id.as_str())
        .collect();
    let labels: std::collections::HashMap<String, DataSensitivity> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .all(db)
        .await?
        .iter()
        .map(|row| (row.id.clone(), bot_label(row)))
        .collect();
    let mut admitted = Vec::with_capacity(waking.len());
    for item in waking {
        let recipient = item.recipient;
        let refusal = recipient
            .mentioned
            .then(|| labels.get(&recipient.bot_id))
            .flatten()
            .and_then(|label| wake_refusal(&sender, &taint, label));
        let Some(rule) = refusal else {
            admitted.push(item);
            continue;
        };
        tracing::info!(
            bot_id = recipient.bot_id,
            sender_id = delivery_sender,
            rule,
            "A bot's room message did not wake the bot it mentions"
        );
        Box::pin(post_notice_in(
            db,
            &recipient.conversation_id,
            &recipient.bot_id,
            &classification_refused_message_id(&recipient.bot_id, &recipient.trigger_message_id),
            "I was not woken for that message: the classification rules do not let a bot with that message's data ask me. Nothing of it was passed to me. Ask my owner, or write to me yourself.",
            now,
        ))
        .await?;
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bot_host::{
            fixture::{at, conversation, database, delivery, message, recipient},
            ledger::enqueue_in,
        },
        entity::bot_turn,
        routes::inbox::fixture::insert_bot,
    };
    use flow_like_types::{bot::BotLocation, inbox::ConversationKind};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};

    fn level(classification: DataClassification) -> DataSensitivity {
        DataSensitivity {
            classification: Some(classification),
            pii: false,
        }
    }

    async fn classify(db: &DatabaseConnection, id: &str, classification: &str) {
        bot::ActiveModel {
            id: Set(id.into()),
            classification: Set(Some(classification.into())),
            ..Default::default()
        }
        .update(db)
        .await
        .unwrap();
    }

    async fn room(sender: &str, receiver: &str, taint: Option<&str>) -> (DatabaseConnection, u64) {
        let db = database().await;
        insert_bot(&db, "bot_1", "user-a", "SERVER").await;
        insert_bot(&db, "bot_2", "user-a", "SERVER").await;
        classify(&db, "bot_1", sender).await;
        classify(&db, "bot_2", receiver).await;
        conversation(&db, "c1", "GROUP", "PLAIN", &["user-a", "bot_1", "bot_2"]).await;
        let mut body = json!({ "text": "@bot_2 what do you think?", "mentions": ["bot_2"], "hops": 1 });
        if let Some(taint) = taint {
            body[TAINT_FIELD] = json!({ "classification": taint });
        }
        message(&db, "c1", "m1", "bot_1", 1, body, &["user-a", "bot_1", "bot_2"]).await;
        crate::entity::conversation::ActiveModel {
            id: Set("c1".into()),
            last_seq: Set(Some(1)),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();
        let mut woken = recipient("bot_2", "user-a", BotLocation::Server);
        woken.mentioned = true;
        woken.hops = 1;
        let mut wake = delivery(ConversationKind::Group, vec![woken]);
        wake.sender_id = "bot_1".into();
        let queued = enqueue_in(&db, &wake, at(2)).await.unwrap();
        (db, queued)
    }

    async fn notices(db: &DatabaseConnection) -> usize {
        inbox_entry::Entity::find()
            .filter(inbox_entry::Column::SenderId.eq("bot_2"))
            .filter(inbox_entry::Column::InboxId.eq("user-a"))
            .all(db)
            .await
            .unwrap()
            .len()
    }

    #[test]
    fn the_wake_rules_are_the_ask_rules() {
        let none = DataSensitivity::default();
        let internal = level(DataClassification::Internal);
        let confidential = level(DataClassification::Confidential);
        assert!(wake_refusal(&internal, &none, &internal).is_none());
        assert!(wake_refusal(&confidential, &none, &internal).is_none());
        assert!(wake_refusal(&internal, &none, &confidential).is_some());
        assert!(wake_refusal(&confidential, &confidential, &internal).is_some());
        assert!(wake_refusal(&none, &none, &none).is_none());
        assert!(wake_refusal(&none, &none, &confidential).is_some());
    }

    #[test]
    fn a_reply_records_its_taint_only_when_it_has_one() {
        let mut body = json!({ "text": "hi" });
        attach(&mut body, &DataSensitivity::default());
        assert!(body.get(TAINT_FIELD).is_none());
        attach(&mut body, &level(DataClassification::Confidential));
        assert_eq!(recorded(&body), level(DataClassification::Confidential));
        assert!(recorded(&json!({ "text": "hi" })).is_empty());
    }

    #[tokio::test]
    async fn a_mention_wakes_a_bot_the_sender_may_ask() {
        let (db, queued) = room("confidential", "internal", None).await;
        assert_eq!(queued, 1);
        assert_eq!(bot_turn::Entity::find().all(&db).await.unwrap().len(), 1);
        assert_eq!(notices(&db).await, 0);
    }

    #[tokio::test]
    async fn a_mention_from_below_the_callee_wakes_nothing_and_the_bot_says_so() {
        let (db, queued) = room("internal", "confidential", None).await;
        assert_eq!(queued, 0);
        assert!(bot_turn::Entity::find().all(&db).await.unwrap().is_empty());
        assert_eq!(notices(&db).await, 1);
    }

    #[tokio::test]
    async fn a_mention_carrying_data_above_the_callee_wakes_nothing() {
        let (db, queued) = room("confidential", "internal", Some("confidential")).await;
        assert_eq!(queued, 0);
        assert_eq!(notices(&db).await, 1);
        let (_, queued) = room("confidential", "internal", Some("internal")).await;
        assert_eq!(queued, 1);
    }

    #[tokio::test]
    async fn the_next_turn_starts_from_the_sender_and_what_the_message_carried() {
        let db = database().await;
        insert_bot(&db, "bot_1", "user-a", "SERVER").await;
        insert_bot(&db, "bot_2", "user-a", "SERVER").await;
        classify(&db, "bot_1", "internal").await;
        conversation(&db, "c1", "GROUP", "PLAIN", &["user-a", "bot_1", "bot_2"]).await;
        message(
            &db,
            "c1",
            "m1",
            "bot_1",
            1,
            json!({ "text": "hello", TAINT_FIELD: { "classification": "confidential" } }),
            &["bot_2"],
        )
        .await;
        message(&db, "c1", "m2", "user-a", 2, json!({ "text": "and?" }), &["bot_2"]).await;
        let window = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq("bot_2"))
            .all(&db)
            .await
            .unwrap();
        let reads = reads_of(&db, &window, "bot_2").await.unwrap();
        assert_eq!(reads.seed(), level(DataClassification::Confidential));
        assert_eq!(reads.bot_senders, vec![level(DataClassification::Internal)]);
        let person_only = reads_of(&db, &window[1..], "bot_2").await.unwrap();
        assert!(person_only.seed().is_empty());
    }
}
