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

fn room_request(
    sender: &str,
    trigger: &str,
    kind: ConversationKind,
) -> crate::routes::inbox::delivery::BotDelivery {
    let mut recipient = recipient("bot_1", "user-a", BotLocation::Server);
    recipient.trigger_message_id = trigger.into();
    recipient.mentioned = true;
    let mut found = delivery(kind, vec![recipient]);
    found.sender_id = sender.into();
    found
}

async fn turn_of(db: &DatabaseConnection, trigger: &str) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(flow_like_types::bot_turn::turn_id("bot_1", trigger))
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_turn_records_who_woke_it_and_whether_the_conversation_is_shared() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_2", "user-a", "SERVER").await;
    let now = at(100_000);
    let asks = [
        (
            "user-c",
            "from-member",
            ConversationKind::Group,
            "user-c",
            "ROOM",
        ),
        (
            "user-a",
            "from-owner",
            ConversationKind::Group,
            "user-a",
            "ROOM",
        ),
        (
            "user-a",
            "direct",
            ConversationKind::Direct,
            "user-a",
            "OWNER",
        ),
        (
            "bot_2",
            "own-bot",
            ConversationKind::Direct,
            "user-a",
            "OWNER",
        ),
        (
            "bot_2",
            "bot-in-room",
            ConversationKind::Group,
            "bot_2",
            "ROOM",
        ),
    ];
    for (sender, trigger, kind, requester, audience) in asks {
        let request = room_request(sender, trigger, kind);
        assert_eq!(
            enqueue_in(&db, &request, now).await.unwrap(),
            1,
            "{trigger}"
        );
        let stored = turn_of(&db, trigger).await;
        assert_eq!(stored.requester_id.as_deref(), Some(requester), "{trigger}");
        assert_eq!(stored.audience, audience, "{trigger}");
    }
}

#[tokio::test]
async fn a_requester_who_is_not_the_owner_has_a_daily_limit_and_the_room_is_told() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    super::fixture::conversation(&db, "c1", "GROUP", "PLAIN", &["user-a", "user-c", "bot_1"]).await;
    let now = at(100_000);
    for index in 0..super::cap::REQUESTER_DAILY_TURNS {
        let request = room_request("user-c", &format!("c-{index}"), ConversationKind::Group);
        assert_eq!(enqueue_in(&db, &request, now).await.unwrap(), 1);
    }
    let over = room_request("user-c", "c-over", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &over, now).await.unwrap(), 0);
    assert!(
        bot_turn::Entity::find_by_id(flow_like_types::bot_turn::turn_id("bot_1", "c-over"))
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    let notice = crate::entity::inbox_entry::Entity::find()
        .filter(crate::entity::inbox_entry::Column::MessageId.eq(
            flow_like_types::bot_turn::rate_limit_message_id("bot_1", "c-over"),
        ))
        .filter(crate::entity::inbox_entry::Column::InboxId.eq("user-c"))
        .one(&db)
        .await
        .unwrap();
    assert!(notice.is_some());

    let owner = room_request("user-a", "owner-asks", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &owner, now).await.unwrap(), 1);
    let other = room_request("user-d", "d-asks", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &other, now).await.unwrap(), 1);
    let tomorrow = now + Duration::hours(WINDOW_HOURS + 1);
    let later = room_request("user-c", "c-later", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &later, tomorrow).await.unwrap(), 1);
}

#[tokio::test]
async fn reading_along_over_the_requesters_limit_stops_quietly_while_a_mention_is_still_told() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    super::fixture::conversation(&db, "c1", "GROUP", "PLAIN", &["user-a", "user-c", "bot_1"]).await;
    let now = at(100_000);
    for index in 0..super::cap::REQUESTER_DAILY_TURNS {
        let request = room_request("user-c", &format!("c-{index}"), ConversationKind::Group);
        assert_eq!(enqueue_in(&db, &request, now).await.unwrap(), 1);
    }
    let mut unaddressed = room_request("user-c", "c-read", ConversationKind::Group);
    unaddressed.recipients[0].mentioned = false;
    unaddressed.recipients[0].read_along = true;
    assert_eq!(enqueue_in(&db, &unaddressed, now).await.unwrap(), 0);
    let notice = |trigger: &'static str| {
        crate::entity::inbox_entry::Entity::find()
            .filter(crate::entity::inbox_entry::Column::MessageId.eq(
                flow_like_types::bot_turn::rate_limit_message_id("bot_1", trigger),
            ))
            .one(&db)
    };
    assert!(notice("c-read").await.unwrap().is_none());

    let named = room_request("user-c", "c-named", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &named, now).await.unwrap(), 0);
    assert!(notice("c-named").await.unwrap().is_some());
}

#[tokio::test]
async fn read_along_and_reaction_turns_do_not_use_up_a_requesters_limit() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    super::fixture::conversation(&db, "c1", "GROUP", "PLAIN", &["user-a", "user-c", "bot_1"]).await;
    let now = at(100_000);
    for index in 0..super::cap::REQUESTER_DAILY_TURNS + 3 {
        let mut chatter = room_request("user-c", &format!("c-{index}"), ConversationKind::Group);
        chatter.recipients[0].mentioned = false;
        chatter.recipients[0].read_along = true;
        assert_eq!(enqueue_in(&db, &chatter, now).await.unwrap(), 1);
    }
    assert_eq!(
        turn_of(&db, "c-0").await.unprompted,
        Some(true),
        "read-along turns are marked"
    );
    let named = room_request("user-c", "c-named", ConversationKind::Group);
    assert_eq!(enqueue_in(&db, &named, now).await.unwrap(), 1);
    assert_eq!(turn_of(&db, "c-named").await.unprompted, None);
}
