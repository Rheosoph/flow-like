//! What starts an org bot's work from outside its chats: its mail address with the week's mail
//! in and out, its webhooks and its calendars. One query per table for the whole org.

use crate::{
    bot_tools::connectors::calendar_provider_of,
    entity::{bot, bot_connector, bot_mail_message, bot_webhook},
    error::ApiError,
    mail_ingress::bot_address::addresses_of,
};
use flow_like_types::bot_team::{ORG_MAIL_WINDOW_DAYS, OrgBotInputs, OrgBotMail, OrgBotWebhook};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::HashMap;

const EMAIL: &str = "email";
const WEBHOOK: &str = "webhook";
const CALENDAR: &str = "calendar";

fn has(row: &bot::Model, capability: &str) -> bool {
    row.capabilities.iter().any(|id| id == capability)
}

fn ids_with<'a>(rows: impl Iterator<Item = &'a bot::Model>, capability: &str) -> Vec<String> {
    rows.filter(|row| has(row, capability))
        .map(|row| row.id.clone())
        .collect()
}

async fn mail_counts<C: ConnectionTrait>(
    db: &C,
    bot_ids: Vec<String>,
) -> Result<HashMap<String, (u32, u32)>, ApiError> {
    if bot_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let since = (chrono::Utc::now() - chrono::Duration::days(ORG_MAIL_WINDOW_DAYS)).fixed_offset();
    let rows: Vec<(String, String, i64)> = bot_mail_message::Entity::find()
        .select_only()
        .column(bot_mail_message::Column::BotId)
        .column(bot_mail_message::Column::Direction)
        .column_as(bot_mail_message::Column::Id.count(), "count")
        .filter(bot_mail_message::Column::BotId.is_in(bot_ids))
        .filter(bot_mail_message::Column::CreatedAt.gte(since))
        .group_by(bot_mail_message::Column::BotId)
        .group_by(bot_mail_message::Column::Direction)
        .into_tuple()
        .all(db)
        .await?;
    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    for (bot_id, direction, count) in rows {
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        let entry = counts.entry(bot_id).or_default();
        if direction.eq_ignore_ascii_case("in") {
            entry.0 = count;
        } else if direction.eq_ignore_ascii_case("out") {
            entry.1 = count;
        }
    }
    Ok(counts)
}

/// Each bot's inputs by id. Bots with none are left out.
pub(crate) async fn inputs_of<C: ConnectionTrait>(
    db: &C,
    rows: &[&bot::Model],
) -> Result<HashMap<String, OrgBotInputs>, ApiError> {
    let mut inputs: HashMap<String, OrgBotInputs> = HashMap::new();

    let addresses = addresses_of(db, &ids_with(rows.iter().copied(), EMAIL)).await?;
    let mut counts = mail_counts(db, addresses.keys().cloned().collect()).await?;
    for (bot_id, address) in addresses {
        let (received, sent) = counts.remove(&bot_id).unwrap_or_default();
        inputs.entry(bot_id).or_default().mail = Some(OrgBotMail {
            address,
            received,
            sent,
        });
    }

    let webhook_bots = ids_with(rows.iter().copied(), WEBHOOK);
    let hooks = if webhook_bots.is_empty() {
        Vec::new()
    } else {
        bot_webhook::Entity::find()
            .filter(bot_webhook::Column::BotId.is_in(webhook_bots))
            .filter(bot_webhook::Column::Enabled.eq(true))
            .order_by_asc(bot_webhook::Column::CreatedAt)
            .order_by_asc(bot_webhook::Column::Id)
            .all(db)
            .await?
    };
    for hook in hooks {
        inputs
            .entry(hook.bot_id)
            .or_default()
            .webhooks
            .push(OrgBotWebhook {
                name: hook.name,
                last_called_at: hook.last_called_at.map(|time| time.to_rfc3339()),
            });
    }

    let calendar_bots = ids_with(rows.iter().copied(), CALENDAR);
    let connectors = if calendar_bots.is_empty() {
        Vec::new()
    } else {
        bot_connector::Entity::find()
            .filter(bot_connector::Column::BotId.is_in(calendar_bots.clone()))
            .order_by_asc(bot_connector::Column::CreatedAt)
            .order_by_asc(bot_connector::Column::Id)
            .all(db)
            .await?
    };
    for bot_id in calendar_bots {
        inputs.entry(bot_id).or_default().own_calendar = true;
    }
    for row in &connectors {
        if let Some(provider) = calendar_provider_of(row) {
            inputs
                .entry(row.bot_id.clone())
                .or_default()
                .connected_calendars
                .push(provider.to_string());
        }
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::super::{
        tests::{create, join, own_bot, world},
        view::org_view,
    };
    use crate::{
        entity::{bot, bot_connector, bot_mail_message, bot_webhook, json_types::StringList},
        routes::inbox::fixture::at,
    };
    use chrono::{DateTime, Duration, FixedOffset, Utc};
    use flow_like_types::bot_team::{OrgBotInputs, OrgBotMail, OrgBotWebhook};
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, Schema,
        sea_query::Table,
    };
    use serde_json::json;

    async fn tables(db: &DatabaseConnection) {
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(bot_webhook::Entity),
            schema.create_table_from_entity(bot_connector::Entity),
            schema.create_table_from_entity(bot_mail_message::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db.execute_unprepared(
            r#"CREATE TABLE public."InboundMailAddress" ("address" TEXT PRIMARY KEY, "appId" TEXT NOT NULL, "eventId" TEXT NOT NULL, "kind" TEXT NOT NULL, "active" BOOLEAN NOT NULL)"#,
        )
        .await
        .unwrap();
    }

    async fn abilities(db: &DatabaseConnection, bot_id: &str, capabilities: &[&str]) {
        bot::ActiveModel {
            id: Set(bot_id.into()),
            capabilities: Set(StringList(
                capabilities.iter().map(|id| id.to_string()).collect(),
            )),
            ..Default::default()
        }
        .update(db)
        .await
        .unwrap();
    }

    async fn address(db: &DatabaseConnection, bot_id: &str, address: &str, active: bool) {
        db.execute_unprepared(&format!(
            r#"INSERT INTO public."InboundMailAddress" VALUES ('{address}','bot','{bot_id}','bot',{})"#,
            i32::from(active)
        ))
        .await
        .unwrap();
    }

    async fn hook(db: &DatabaseConnection, id: &str, bot_id: &str, name: &str, enabled: bool) {
        bot_webhook::ActiveModel {
            id: Set(id.into()),
            bot_id: Set(bot_id.into()),
            name: Set(name.into()),
            secret_hash: Set("hash".into()),
            secret_ciphertext: Set(None),
            instruction: Set("Never shown to the org".into()),
            enabled: Set(enabled),
            created_at: Set(at(1)),
            last_called_at: Set(enabled.then(|| at(9))),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn mail(
        db: &DatabaseConnection,
        id: &str,
        direction: &str,
        created_at: DateTime<FixedOffset>,
    ) {
        bot_mail_message::ActiveModel {
            id: Set(id.into()),
            thread_id: Set("thread".into()),
            bot_id: Set("mara-bot".into()),
            message_id: Set(format!("{id}@mail.test")),
            direction: Set(direction.into()),
            entry_message_id: Set(id.into()),
            created_at: Set(created_at),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn connector(
        db: &DatabaseConnection,
        id: &str,
        bot_id: &str,
        kind: &str,
        provider: Option<&str>,
        reads: &str,
    ) {
        let caldav = kind == "CALDAV";
        bot_connector::ActiveModel {
            id: Set(id.into()),
            bot_id: Set(bot_id.into()),
            kind: Set(kind.into()),
            label: Set(id.into()),
            provider_id: Set(provider.map(str::to_string)),
            url: Set(caldav.then(|| "https://dav.example.org/".into())),
            auth_kind: Set("NONE".into()),
            auth_ref: Set(None),
            grant_encrypted: Set(None),
            scopes: Set(json!([])),
            default_policy: Set("ask".into()),
            tool_policies: Set(json!({ "read": reads })),
            tools_snapshot: Set(json!([])),
            confirmed_hashes: Set(json!({})),
            calendar_url: Set(caldav.then(|| "https://dav.example.org/work/".into())),
            calendars: Set(caldav.then(|| {
                json!([{ "url": "https://dav.example.org/work/", "name": "Work", "writable": true }])
            })),
            created_at: Set(at(2)),
            updated_at: Set(at(2)),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_org_sees_each_bots_ways_in_while_their_abilities_are_on() {
        let db = world().await;
        tables(&db).await;
        let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
        join(&db, "mara", &kestrel, "jonas").await;
        own_bot(&db, "mara", &kestrel, "mara-bot").await;
        own_bot(&db, "mara", &kestrel, "mara-bot-2").await;
        abilities(&db, "mara-bot", &["email", "webhook", "calendar"]).await;

        address(&db, "mara-bot", "ledger@mail.test", true).await;
        address(&db, "mara-bot", "old-ledger@mail.test", false).await;
        address(&db, "mara-bot-2", "quiet@mail.test", true).await;
        hook(&db, "h1", "mara-bot", "Stripe payouts", true).await;
        hook(&db, "h2", "mara-bot", "Switched off", false).await;
        hook(&db, "h3", "mara-bot-2", "Without the ability", true).await;
        let now = Utc::now().fixed_offset();
        mail(&db, "m1", "IN", now).await;
        mail(&db, "m2", "IN", now - Duration::hours(30)).await;
        mail(&db, "m3", "OUT", now).await;
        mail(&db, "m4", "IN", now - Duration::days(30)).await;
        connector(&db, "c1", "mara-bot", "OAUTH", Some("google"), "ask").await;
        connector(&db, "c2", "mara-bot", "MCP", None, "ask").await;
        connector(&db, "c3", "mara-bot", "CALDAV", None, "read").await;
        connector(&db, "c4", "mara-bot", "OAUTH", Some("microsoft"), "never").await;
        connector(&db, "c5", "mara-bot-2", "OAUTH", Some("google"), "ask").await;

        let view = org_view(&db, "jonas").await.unwrap();
        let inputs = |id: &str| {
            view.bots
                .iter()
                .find(|bot| bot.id == id)
                .unwrap()
                .inputs
                .clone()
        };
        assert_eq!(
            inputs("mara-bot"),
            OrgBotInputs {
                mail: Some(OrgBotMail {
                    address: "ledger@mail.test".into(),
                    received: 2,
                    sent: 1,
                }),
                webhooks: vec![OrgBotWebhook {
                    name: "Stripe payouts".into(),
                    last_called_at: Some(at(9).to_rfc3339()),
                }],
                own_calendar: true,
                connected_calendars: vec!["google".into(), "caldav".into()],
            }
        );
        assert_eq!(inputs("mara-bot-2"), OrgBotInputs::default());
    }
}
