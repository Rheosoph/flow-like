use super::{
    cap::WINDOW_HOURS,
    fixture::{at, database, delivery, insert_bot, recipient},
    ledger::enqueue_in,
};
use crate::entity::{bot_turn, notification};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{bot::BotLocation, inbox::ConversationKind};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};

const FREE_LIMIT: usize = 50;

async fn send(
    db: &DatabaseConnection,
    owner: &str,
    bots: &[&str],
    trigger: &str,
    hops: u32,
    now: DateTime<FixedOffset>,
) -> u64 {
    let recipients = bots
        .iter()
        .map(|bot| {
            let mut recipient = recipient(bot, owner, BotLocation::Server);
            recipient.trigger_message_id = trigger.into();
            recipient.hops = hops;
            recipient
        })
        .collect();
    enqueue_in(db, &delivery(ConversationKind::Direct, recipients), now)
        .await
        .unwrap()
}

async fn fill(
    db: &DatabaseConnection,
    owner: &str,
    bot: &str,
    count: usize,
    now: DateTime<FixedOffset>,
) {
    for index in 0..count {
        assert_eq!(
            send(db, owner, &[bot], &format!("fill-{owner}-{index}"), 1, now).await,
            1
        );
    }
}

async fn notices(db: &DatabaseConnection, owner: &str) -> Vec<notification::Model> {
    notification::Entity::find()
        .filter(notification::Column::UserId.eq(owner))
        .all(db)
        .await
        .unwrap()
}

async fn turns_of(db: &DatabaseConnection, owner: &str) -> u64 {
    bot_turn::Entity::find()
        .filter(bot_turn::Column::OwnerId.eq(owner))
        .count(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn bot_started_turns_queue_up_to_the_plan_cap_and_the_rest_are_dropped() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_2", "user-a", "SERVER").await;
    let now = at(100_000);
    fill(&db, "user-a", "bot_1", FREE_LIMIT - 1, now).await;
    assert!(notices(&db, "user-a").await.is_empty());

    let queued = send(&db, "user-a", &["bot_1", "bot_2"], "last", 2, now).await;
    assert_eq!(queued, 1);
    assert_eq!(turns_of(&db, "user-a").await, FREE_LIMIT as u64);

    assert_eq!(send(&db, "user-a", &["bot_1"], "over", 1, now).await, 0);
    assert_eq!(turns_of(&db, "user-a").await, FREE_LIMIT as u64);
}

#[tokio::test]
async fn the_owner_gets_one_notice_a_day_when_the_cap_is_hit() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    let now = at(100_000);
    fill(&db, "user-a", "bot_1", FREE_LIMIT, now).await;
    assert!(notices(&db, "user-a").await.is_empty());

    for trigger in ["over-1", "over-2", "over-3"] {
        assert_eq!(send(&db, "user-a", &["bot_1"], trigger, 1, now).await, 0);
    }
    let found = notices(&db, "user-a").await;
    assert_eq!(found.len(), 1);
    assert!(found[0].description.as_deref().unwrap().contains("50"));

    let next_day = now + Duration::hours(30);
    assert_eq!(
        send(&db, "user-a", &["bot_1"], "later", 1, next_day).await,
        1
    );
}

#[tokio::test]
async fn a_persons_messages_ignore_the_cap_and_other_owners_are_unaffected() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_9", "user-b", "SERVER").await;
    let now = at(100_000);
    fill(&db, "user-a", "bot_1", FREE_LIMIT, now).await;

    assert_eq!(send(&db, "user-a", &["bot_1"], "person", 0, now).await, 1);
    assert_eq!(send(&db, "user-b", &["bot_9"], "other", 1, now).await, 1);
    assert!(notices(&db, "user-b").await.is_empty());
}

#[tokio::test]
async fn turns_older_than_a_day_stop_counting() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    let old = at(100_000);
    fill(&db, "user-a", "bot_1", FREE_LIMIT, old).await;

    let just_inside = old + Duration::hours(WINDOW_HOURS) - Duration::seconds(1);
    assert_eq!(
        send(&db, "user-a", &["bot_1"], "inside", 1, just_inside).await,
        0
    );
    let after = old + Duration::hours(WINDOW_HOURS) + Duration::seconds(1);
    assert_eq!(send(&db, "user-a", &["bot_1"], "after", 1, after).await, 1);
}
