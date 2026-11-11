//! Room activity that arrives after the message a running bot is answering.

use super::history::{plain_body, render, sender_names};
use crate::entity::{conversation_member, inbox_entry};
use async_trait::async_trait;
use flow_like::flow::copilot::platform::RoomUpdates;
use flow_like_types::{
    bot_message::MessageBody,
    bot_turn::room_message,
    inbox::{
        InboxEntryKind, MemberEventBody, MemberOp, ReactionEvent, RoomChange, RoomEventBody,
        TranscriptBody, TriggerBody, body_target, is_tombstone,
    },
    tokio::sync::Mutex,
};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::{collections::HashMap, sync::Arc};

const MAX_ENTRIES: u64 = 100;
const MAX_UPDATE_CHARS: usize = 20_000;
const MAX_MESSAGE_CHARS: usize = 8_000;

#[derive(Debug)]
pub(super) struct ServerRoomUpdates {
    db: DatabaseConnection,
    bot_id: String,
    conversation_id: String,
    owner_id: String,
    cursor: Mutex<inbox_entry::Model>,
}

impl ServerRoomUpdates {
    pub(super) async fn after(
        db: &DatabaseConnection,
        bot_id: &str,
        conversation_id: &str,
        trigger_message_id: &str,
        owner_id: &str,
    ) -> Result<Option<Arc<Self>>, DbErr> {
        let trigger = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(bot_id))
            .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
            .filter(inbox_entry::Column::MessageId.eq(trigger_message_id))
            .one(db)
            .await?;
        Ok(trigger.map(|cursor| {
            Arc::new(Self {
                db: db.clone(),
                bot_id: bot_id.to_owned(),
                conversation_id: conversation_id.to_owned(),
                owner_id: owner_id.to_owned(),
                cursor: Mutex::new(cursor),
            })
        }))
    }

    async fn read(&self) -> Result<Vec<String>, DbErr> {
        let active = conversation_member::Entity::find()
            .filter(conversation_member::Column::ConversationId.eq(&self.conversation_id))
            .filter(conversation_member::Column::PrincipalId.eq(&self.bot_id))
            .filter(crate::routes::inbox::active_member())
            .one(&self.db)
            .await?;
        if active.is_none() {
            return Ok(Vec::new());
        }
        let mut cursor = self.cursor.lock().await;
        let after = match cursor.seq {
            Some(seq) => Condition::all().add(inbox_entry::Column::Seq.gt(seq)),
            None => Condition::any()
                .add(inbox_entry::Column::CreatedAt.gt(cursor.created_at))
                .add(
                    Condition::all()
                        .add(inbox_entry::Column::CreatedAt.eq(cursor.created_at))
                        .add(inbox_entry::Column::Id.gt(&cursor.id)),
                ),
        };
        let mut entries = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(&self.bot_id))
            .filter(inbox_entry::Column::ConversationId.eq(&self.conversation_id))
            .filter(after)
            .order_by_desc(inbox_entry::Column::Seq)
            .order_by_desc(inbox_entry::Column::CreatedAt)
            .order_by_desc(inbox_entry::Column::Id)
            .limit(MAX_ENTRIES + 1)
            .all(&self.db)
            .await?;
        let overflow = entries.len() > MAX_ENTRIES as usize;
        entries.truncate(MAX_ENTRIES as usize);
        entries.reverse();
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<&str> = entries
            .iter()
            .map(|entry| entry.sender_id.as_str())
            .collect();
        let names = sender_names(&self.db, &ids).await?;
        let mut updates = Vec::new();
        let mut remaining = MAX_UPDATE_CHARS;
        // Keep the newest activity when a busy room exceeds the prompt budget.
        let mut clipped = overflow;
        for entry in entries.iter().rev() {
            if let Some(note) = render_update(entry, &self.bot_id, &self.owner_id, &names) {
                let text: String = note
                    .chars()
                    .take(MAX_MESSAGE_CHARS.min(remaining))
                    .collect();
                clipped |= text.chars().count() < note.chars().count();
                remaining = remaining.saturating_sub(text.chars().count());
                if !text.is_empty() {
                    updates.push(text);
                }
            }
        }
        updates.reverse();
        if clipped {
            updates.insert(
                0,
                "Some room activity was omitted or shortened to fit; these are the latest updates."
                    .into(),
            );
        }
        *cursor = entries.pop().expect("nonempty room update page");
        Ok(updates)
    }
}

#[async_trait]
impl RoomUpdates for ServerRoomUpdates {
    async fn drain(&self) -> Vec<String> {
        match self.read().await {
            Ok(updates) => updates,
            Err(error) => {
                tracing::warn!(%error, "Bot turn: room updates could not be read");
                Vec::new()
            }
        }
    }
}

fn render_update(
    entry: &inbox_entry::Model,
    bot_id: &str,
    owner_id: &str,
    names: &HashMap<String, String>,
) -> Option<String> {
    if is_tombstone(&entry.envelope) {
        return None;
    }
    let body = plain_body(&entry.envelope)?;
    if flow_like_types::bot_user_context::is_ephemeral_body(body) {
        return None;
    }
    let text = match InboxEntryKind::parse(&entry.kind)? {
        InboxEntryKind::Message | InboxEntryKind::Edit => {
            let parsed: MessageBody = serde_json::from_value(body.clone()).ok()?;
            if parsed.approval_response.is_some() {
                return None;
            }
            let text = render(&parsed);
            if text.trim().is_empty() {
                return None;
            }
            if entry.kind.eq_ignore_ascii_case("edit") {
                format!("[edited message {}] {text}", body_target(&entry.envelope)?)
            } else {
                format!("[message {}] {text}", entry.message_id)
            }
        }
        InboxEntryKind::Transcript => TranscriptBody::parse(body).ok()?.render(),
        InboxEntryKind::Trigger => TriggerBody::parse(body).ok()?.render(),
        InboxEntryKind::Delete => format!("[deleted message {}]", body_target(&entry.envelope)?),
        InboxEntryKind::Topic => {
            "[started a new topic; earlier discussion belongs to the previous topic]".into()
        }
        InboxEntryKind::Reaction => {
            let reaction =
                ReactionEvent::parse(&entry.message_id, &entry.sender_id, &entry.envelope)?;
            format!(
                "[{} reaction {} on message {}]",
                if reaction.add { "added" } else { "removed" },
                reaction.emoji,
                reaction.target
            )
        }
        InboxEntryKind::Member => {
            let member: MemberEventBody = serde_json::from_value(body.clone()).ok()?;
            format!(
                "[{} {} the room]",
                member.principal_id,
                match member.op {
                    MemberOp::Join => "joined",
                    MemberOp::Leave => "left",
                }
            )
        }
        InboxEntryKind::Room => {
            let change: RoomEventBody = serde_json::from_value(body.clone()).ok()?;
            match change.change {
                RoomChange::Title => match change.title {
                    Some(title) if !change.removed => format!("[renamed the room to {title}]"),
                    _ => "[removed the room name]".into(),
                },
                RoomChange::Image => "[changed the room picture]".into(),
            }
        }
        _ => return None,
    };
    if entry.sender_id == bot_id {
        return Some(format!("[You already posted this room event] {text}"));
    }
    let name = names.get(&entry.sender_id).unwrap_or(&entry.sender_id);
    Some(room_message(name, entry.sender_id == owner_id, &text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::fixture::{conversation, database, entry, insert_bot, message};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    use serde_json::json;

    async fn room() -> DatabaseConnection {
        let db = database().await;
        insert_bot(&db, "bot_1", "user-a", "SERVER").await;
        insert_bot(&db, "bot_2", "user-a", "SERVER").await;
        conversation(&db, "room", "GROUP", "PLAIN", &["user-a", "bot_1", "bot_2"]).await;
        message(
            &db,
            "room",
            "trigger",
            "user-a",
            1,
            json!({"text": "introduce yourselves"}),
            &["bot_1"],
        )
        .await;
        db
    }

    #[tokio::test]
    async fn late_peer_and_owner_messages_are_read_once_in_room_order() {
        let db = room().await;
        message(
            &db,
            "room",
            "peer",
            "bot_2",
            2,
            json!({"text": "I already introduced myself"}),
            &["bot_1"],
        )
        .await;
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        message(
            &db,
            "room",
            "owner",
            "user-a",
            3,
            json!({"text": "keep it brief"}),
            &["bot_1"],
        )
        .await;
        message(
            &db,
            "room",
            "own",
            "bot_1",
            4,
            json!({"text": "own message"}),
            &["bot_1"],
        )
        .await;
        // The same room's other inbox and a different room are outside this turn's view.
        message(
            &db,
            "room",
            "private",
            "user-a",
            5,
            json!({"text": "another inbox"}),
            &["bot_2"],
        )
        .await;
        conversation(&db, "other", "GROUP", "PLAIN", &["user-a", "bot_1"]).await;
        message(
            &db,
            "other",
            "elsewhere",
            "user-a",
            6,
            json!({"text": "another room"}),
            &["bot_1"],
        )
        .await;
        let updates = source.drain().await;
        assert_eq!(updates.len(), 3);
        assert!(
            updates[0].contains("member, not your owner")
                && updates[0].contains("I already introduced myself")
        );
        assert!(updates[1].contains("your owner") && updates[1].contains("keep it brief"));
        assert!(updates[2].contains("You already posted") && updates[2].contains("own message"));
        assert!(source.drain().await.is_empty());
        message(
            &db,
            "room",
            "later",
            "bot_2",
            7,
            json!({"text": "one more thing"}),
            &["bot_1"],
        )
        .await;
        assert_eq!(source.drain().await.len(), 1);
    }

    #[tokio::test]
    async fn temporary_context_waits_for_its_own_turn() {
        let db = room().await;
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        for (index, body) in [
            json!({"text": "request", "user_context_request": {}}),
            json!({"text": "temporary values", "user_context_response": {}}),
            json!({"text": "derived location", "user_context_derived": true}),
        ]
        .into_iter()
        .enumerate()
        {
            message(
                &db,
                "room",
                &format!("context-{index}"),
                "user-a",
                index as i64 + 2,
                body,
                &["bot_1"],
            )
            .await;
        }
        assert!(source.drain().await.is_empty());
        message(
            &db,
            "room",
            "ordinary",
            "user-a",
            5,
            json!({"text": "ordinary follow-up"}),
            &["bot_1"],
        )
        .await;
        let updates = source.drain().await;
        assert_eq!(updates.len(), 1);
        assert!(updates[0].contains("ordinary follow-up"));
    }

    #[tokio::test]
    async fn corrections_and_topic_changes_reach_the_running_turn() {
        let db = room().await;
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        entry(
            &db,
            "room",
            "edit",
            "user-a",
            2,
            "EDIT",
            json!({"target": "trigger", "text": "only introduce yourself"}),
            &["bot_1"],
        )
        .await;
        entry(
            &db,
            "room",
            "delete",
            "user-a",
            3,
            "DELETE",
            json!({"target": "trigger"}),
            &["bot_1"],
        )
        .await;
        entry(
            &db,
            "room",
            "topic",
            "user-a",
            4,
            "TOPIC",
            json!({}),
            &["bot_1"],
        )
        .await;
        let updates = source.drain().await;
        assert_eq!(updates.len(), 3);
        assert!(
            updates[0].contains("edited message trigger")
                && updates[0].contains("only introduce yourself")
        );
        assert!(updates[1].contains("deleted message trigger"));
        assert!(updates[2].contains("started a new topic"));
        assert!(source.drain().await.is_empty());
    }

    #[tokio::test]
    async fn updates_stop_when_the_bot_leaves_the_room() {
        let db = room().await;
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        let membership = conversation_member::Entity::find()
            .filter(conversation_member::Column::ConversationId.eq("room"))
            .filter(conversation_member::Column::PrincipalId.eq("bot_1"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut membership: conversation_member::ActiveModel = membership.into();
        membership.left_at = Set(Some(crate::bot_host::fixture::at(2)));
        membership.update(&db).await.unwrap();
        message(
            &db,
            "room",
            "late",
            "user-a",
            3,
            json!({"text": "after leaving"}),
            &["bot_1"],
        )
        .await;
        assert!(source.drain().await.is_empty());
    }

    #[tokio::test]
    async fn legacy_entries_with_equal_timestamps_use_the_entry_id_as_a_tiebreaker() {
        let db = room().await;
        message(
            &db,
            "room",
            "z-later",
            "user-a",
            2,
            json!({"text": "same timestamp, later entry"}),
            &["bot_1"],
        )
        .await;
        inbox_entry::Entity::update_many()
            .col_expr(
                inbox_entry::Column::Seq,
                sea_orm::sea_query::Expr::value(None::<i64>),
            )
            .col_expr(
                inbox_entry::Column::CreatedAt,
                sea_orm::sea_query::Expr::value(crate::bot_host::fixture::at(1)),
            )
            .exec(&db)
            .await
            .unwrap();
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        let updates = source.drain().await;
        assert_eq!(updates.len(), 1);
        assert!(updates[0].contains("same timestamp, later entry"));
        assert!(source.drain().await.is_empty());
    }

    #[tokio::test]
    async fn large_room_updates_keep_the_latest_text_with_an_explicit_limit_note() {
        let db = room().await;
        let source = ServerRoomUpdates::after(&db, "bot_1", "room", "trigger", "user-a")
            .await
            .unwrap()
            .unwrap();
        for seq in 2..=5 {
            message(
                &db,
                "room",
                &format!("m{seq}"),
                "user-a",
                seq,
                json!({"text": format!("latest-{seq} {}", "x".repeat(9_000))}),
                &["bot_1"],
            )
            .await;
        }
        let updates = source.drain().await;
        assert!(updates[0].contains("omitted or shortened"));
        assert!(updates.last().unwrap().contains("latest-5"));
        assert!(
            updates
                .iter()
                .map(|text| text.chars().count())
                .sum::<usize>()
                < MAX_UPDATE_CHARS + 100
        );
        assert!(source.drain().await.is_empty());
    }
}
