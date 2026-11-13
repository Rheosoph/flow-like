//! What a deleted bot leaves outside its inbox: its standing grants in both directions, its tool grants, the
//! heartbeats of the hosts that ran it and the conversation summaries it wrote. Bots are
//! soft-deleted, so their foreign keys never cascade: the bot's retirement
//! (`routes::inbox::retirement::finish_retirement`) purges these rows once the bot has left its
//! conversations, and account deletion drains the same rows through the User plan.

use sea_orm::ConnectionTrait;

use super::drain::{delete_page, page_size, select_page};
use super::graph::fk_graph;
use super::plan::Predicate;
use crate::error::ApiError;
use flow_like_types::anyhow;

/// The rows of each table that go with a deleted bot, selected by the bot's id (`$root`).
pub fn retired_bot_rows() -> [(&'static str, Predicate); 13] {
    let by = |column: &str| Predicate::Root {
        column: column.to_owned(),
    };
    [
        (
            "BotTurnCheckpoint",
            Predicate::Via {
                column: "turnId".into(),
                parent: "BotTurn".into(),
                parent_column: "id".into(),
                inner: Box::new(by("botId")),
            },
        ),
        ("BotDecisionWait", by("botId")),
        ("BotNote", by("botId")),
        ("BotPermission", by("fromBotId")),
        ("BotPermission", by("toBotId")),
        ("BotHostHeartbeat", by("botId")),
        ("BotToolGrant", by("botId")),
        ("BotMailMessage", by("botId")),
        ("BotMailThread", by("botId")),
        ("BotWebhook", by("botId")),
        ("BotSkill", by("botId")),
        ("BotConnector", by("botId")),
        ("ConversationSegment", by("authorId")),
    ]
}

/// Deletes a deleted bot's rows a bounded page per statement and returns how many went.
/// Re-runnable: a second call finds nothing.
pub async fn purge_retired_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<u64, ApiError> {
    let mut purged = 0;
    for (table, predicate) in retired_bot_rows() {
        let meta = fk_graph().table(table).ok_or_else(|| {
            ApiError::internal_error(anyhow!(
                "{table} is not registered with the deletion planner"
            ))
        })?;
        let limit = page_size(meta);
        loop {
            let keys = select_page(db, meta, &predicate, bot_id, limit).await?;
            if keys.is_empty() {
                break;
            }
            let deleted = delete_page(db, meta, &keys, &predicate, bot_id).await?;
            purged += deleted;
            if keys.len() < limit || deleted == 0 {
                break;
            }
        }
    }
    Ok(purged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{
        bot_host_heartbeat, bot_permission, bot_tool_grant, bot_webhook, conversation_segment,
    };
    use chrono::{DateTime, FixedOffset};
    use sea_orm::{
        ActiveValue::Set, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
        PaginatorTrait, QueryFilter, Schema, sea_query::Table,
    };

    /// Each row is a foreign key onto the bot, so a bot's id selects exactly its own rows.
    #[test]
    fn every_retired_bot_row_is_a_foreign_key_onto_the_bot() {
        for (table, predicate) in retired_bot_rows() {
            let (owner, column) = predicate.root_hop(table);
            assert!(
                fk_graph().edges().iter().any(|edge| edge.child == owner
                    && edge.column == column
                    && edge.parent == "Bot"
                    && edge.parent_column == "id"),
                "{owner}.{column} is not a foreign key onto Bot.id"
            );
        }
    }

    fn at(seconds: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp(seconds, 0).unwrap().fixed_offset()
    }

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(bot_permission::Entity),
            schema.create_table_from_entity(bot_host_heartbeat::Entity),
            schema.create_table_from_entity(conversation_segment::Entity),
            schema.create_table_from_entity(bot_tool_grant::Entity),
            schema.create_table_from_entity(bot_webhook::Entity),
            schema.create_table_from_entity(crate::entity::bot_skill::Entity),
            schema.create_table_from_entity(crate::entity::bot_connector::Entity),
            schema.create_table_from_entity(crate::entity::bot_mail_thread::Entity),
            schema.create_table_from_entity(crate::entity::bot_mail_message::Entity),
            schema.create_table_from_entity(crate::entity::bot_turn::Entity),
            schema.create_table_from_entity(crate::entity::bot_turn_checkpoint::Entity),
            schema.create_table_from_entity(crate::entity::bot_decision_wait::Entity),
            schema.create_table_from_entity(crate::entity::bot_note::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    fn grant(id: &str, from: &str, to: &str) -> bot_permission::ActiveModel {
        bot_permission::ActiveModel {
            id: Set(id.into()),
            owner_id: Set("user-a".into()),
            from_bot_id: Set(from.into()),
            to_bot_id: Set(to.into()),
            granted_at: Set(at(0)),
            granted_by: Set("user-a".into()),
        }
    }

    fn heartbeat(bot_id: &str, host: usize) -> bot_host_heartbeat::ActiveModel {
        bot_host_heartbeat::ActiveModel {
            bot_id: Set(bot_id.into()),
            host_id: Set(format!("host-{host}")),
            host_kind: Set("DESKTOP".into()),
            last_seen_at: Set(at(1)),
        }
    }

    fn segment(
        id: &str,
        conversation_id: &str,
        author_id: &str,
    ) -> conversation_segment::ActiveModel {
        conversation_segment::ActiveModel {
            id: Set(id.into()),
            conversation_id: Set(conversation_id.into()),
            author_id: Set(author_id.into()),
            level: Set(0),
            seq_from: Set(1),
            seq_to: Set(40),
            t_from: Set(at(0)),
            t_to: Set(at(3)),
            title: Set(None),
            summary: Set("summary".into()),
            parent_id: Set(None),
            version: Set(1),
            created_at: Set(at(4)),
        }
    }

    /// Grants both ways, every host's heartbeat and the summaries the bot wrote go; another
    /// bot's rows stay, its summaries of a conversation they shared included. Heartbeats span two
    /// pages.
    #[tokio::test]
    async fn a_deleted_bot_takes_its_grants_heartbeats_and_summaries_page_by_page() {
        let db = database().await;
        bot_permission::Entity::insert_many([
            grant("p1", "bot_1", "bot_2"),
            grant("p2", "bot_2", "bot_1"),
            grant("p3", "bot_2", "bot_3"),
        ])
        .exec_without_returning(&db)
        .await
        .unwrap();
        let limit = page_size(fk_graph().table("BotHostHeartbeat").unwrap());
        let hosts = limit + 2;
        bot_host_heartbeat::Entity::insert_many(
            (0..hosts)
                .map(|host| heartbeat("bot_1", host))
                .chain([heartbeat("bot_2", 0)]),
        )
        .exec_without_returning(&db)
        .await
        .unwrap();
        conversation_segment::Entity::insert_many([
            segment("s1", "direct", "bot_1"),
            segment("s2", "shared", "bot_1"),
            segment("s3", "shared", "bot_2"),
            segment("s4", "other", "bot_2"),
        ])
        .exec_without_returning(&db)
        .await
        .unwrap();

        let purged = purge_retired_bot(&db, "bot_1").await.unwrap();
        assert_eq!(purged, 2 + hosts as u64 + 2);

        let grants: Vec<String> = bot_permission::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(grants, ["p3"]);
        let heartbeats = bot_host_heartbeat::Entity::find()
            .filter(bot_host_heartbeat::Column::BotId.eq("bot_1"))
            .count(&db)
            .await
            .unwrap();
        assert_eq!(heartbeats, 0);
        assert_eq!(
            bot_host_heartbeat::Entity::find().count(&db).await.unwrap(),
            1
        );
        let mut segments: Vec<String> = conversation_segment::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        segments.sort();
        assert_eq!(segments, ["s3", "s4"]);

        assert_eq!(purge_retired_bot(&db, "bot_1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_deleted_bot_takes_its_decision_waits() {
        use crate::entity::bot_decision_wait;
        let wait = |bot: &str, room: &str| bot_decision_wait::ActiveModel {
            bot_id: Set(bot.into()),
            conversation_id: Set(room.into()),
            participant_id: Set("user-b".into()),
            trigger_message_id: Set("m1".into()),
            after_seq: Set(1),
            expires_at: Set(at(100)),
            created_at: Set(at(0)),
        };
        let db = database().await;
        bot_decision_wait::Entity::insert_many([
            wait("bot_1", "c1"),
            wait("bot_1", "c2"),
            wait("bot_2", "c1"),
        ])
        .exec_without_returning(&db)
        .await
        .unwrap();
        assert_eq!(purge_retired_bot(&db, "bot_1").await.unwrap(), 2);
        let left = bot_decision_wait::Entity::find().all(&db).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].bot_id, "bot_2");
    }
}
