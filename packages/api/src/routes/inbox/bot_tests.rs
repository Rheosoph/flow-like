use super::{
    BOT_TURN_NOT_CLAIMED, Reader,
    conversations::{NewConversation, member_row, start_group},
    delivery::BotRecipient,
    entries::{SendAs, send_as, send_in},
    fixture::{
        assert_code, at, bot_principal, copies, database, delete_bot, insert_bot, insert_flux,
        outgoing, running_turn, says, text, turn, user,
    },
    members::add_in,
    model::TurnClaim,
    reader_membership,
    retirement::leave,
    turns::{TURN_DONE, desktop_bot, reply_in},
    view::conversations_for,
    watched_inboxes,
};
use crate::entity::{conversation, conversation_member, inbox_entry};
use axum::http::StatusCode;
use flow_like_types::{
    bot::BotLocation,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::json;
use std::collections::BTreeSet;

/// `team`: user-a, user-b, user-a's desktop bot_1 and user-b's server bot_2.
async fn team(db: &DatabaseConnection) {
    insert_bot(db, "bot_1", "user-a", "DESKTOP").await;
    insert_bot(db, "bot_2", "user-b", "SERVER").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("team".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    let others = [
        user("user-b"),
        bot_principal("bot_1"),
        bot_principal("bot_2"),
    ];
    start_group(db, "team", "user-a", &plan, &others, at(0))
        .await
        .unwrap();
}

/// A direct conversation between two bots, as a bot-to-bot exchange holds it: no person in it.
async fn bots_only(db: &DatabaseConnection, id: &str, first: &str, second: &str) {
    conversation::ActiveModel {
        id: Set(id.into()),
        kind: Set("DIRECT".into()),
        title: Set(None),
        encryption: Set("PLAIN".into()),
        direct_key: Set(None),
        created_by: Set(first.into()),
        last_activity_at: Set(at(0)),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        import_key: Set(None),
        last_seq: Set(None),
        image: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
    for bot in [first, second] {
        member_row(id, &bot_principal(bot), None, at(0))
            .insert(db)
            .await
            .unwrap();
    }
}

fn claim(id: &str, host_id: &str) -> TurnClaim {
    TurnClaim {
        id: id.into(),
        host_id: host_id.into(),
    }
}

#[test]
fn as_and_turn_come_together() {
    assert_eq!(send_as(None, None).unwrap(), SendAs::Person);
    assert_eq!(send_as(Some("  "), None).unwrap(), SendAs::Person);
    assert_eq!(
        send_as(Some(" bot_1 "), Some(claim("turn_1", "host-1"))).unwrap(),
        SendAs::Bot {
            bot_id: "bot_1".into(),
            claim: claim("turn_1", "host-1")
        }
    );
    for (as_bot, turn) in [
        (None, Some(claim("turn_1", "host-1"))),
        (Some("bot_1"), None),
    ] {
        let error = send_as(as_bot, turn).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn only_bots_the_owner_s_desktop_answers_are_sent_as_over_http() {
    let db = database().await;
    insert_bot(&db, "bot_desk", "user-a", "DESKTOP").await;
    insert_flux(&db, "bot_flux", "user-a").await;
    insert_bot(&db, "bot_srv", "user-a", "SERVER").await;
    insert_bot(&db, "bot_dev", "user-a", "DEVICE").await;
    insert_bot(&db, "bot_gone", "user-a", "DESKTOP").await;
    delete_bot(&db, "bot_gone").await;

    for bot in ["bot_desk", "bot_flux"] {
        assert_eq!(desktop_bot(&db, "user-a", bot).await.unwrap().id, bot);
    }
    for (owner, bot) in [
        ("user-b", "bot_desk"),
        ("user-a", "bot_gone"),
        ("user-a", "nobody"),
    ] {
        let error = desktop_bot(&db, owner, bot).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{owner} {bot}");
    }
    for (bot, host) in [("bot_srv", "the server"), ("bot_dev", "a device")] {
        let error = desktop_bot(&db, "user-a", bot).await.unwrap_err();
        assert_code(&error, StatusCode::CONFLICT, BOT_TURN_NOT_CLAIMED);
        assert!(says(&error, host), "{error:?}");
    }
}

#[tokio::test]
async fn a_reply_completes_the_turn_its_host_claimed_together_with_the_message() {
    let db = database().await;
    team(&db).await;
    running_turn(&db, "turn_1", "bot_1", "team", "host-1", at(100)).await;
    let reply = outgoing(
        "re_1",
        InboxEntryKind::Message,
        json!({ "text": "Done.", "hops": 2 }),
    );

    let sent = reply_in(
        &db,
        "team",
        "bot_1",
        &claim("turn_1", "host-1"),
        &reply,
        at(50),
    )
    .await
    .unwrap();
    assert_eq!(
        (sent.entry.sender_id.as_str(), sent.entry.seq),
        ("bot_1", Some(4))
    );
    let done = turn(&db, "turn_1").await;
    assert_eq!(done.status, TURN_DONE);
    assert_eq!(done.reply_message_id.as_deref(), Some("re_1"));
    assert_eq!((done.lease_until, done.updated_at), (None, at(50)));
    assert_eq!(copies(&db, "re_1").await.len(), 4);
    assert_eq!(sent.wake, ["user-a", "user-b"]);
    let to_bot_2: Vec<(&str, u32)> = sent
        .bots
        .iter()
        .map(|bot| (bot.bot_id.as_str(), bot.hops))
        .collect();
    assert_eq!(
        to_bot_2,
        [("bot_2", 2)],
        "the sender gets no delivery; hops follow the turn, not the body"
    );

    let again = reply_in(
        &db,
        "team",
        "bot_1",
        &claim("turn_1", "host-1"),
        &reply,
        at(60),
    )
    .await
    .unwrap();
    assert_eq!(again.entry, sent.entry, "a repeat returns the first copy");
    assert!(again.wake.is_empty() && again.bots.is_empty());
}

#[tokio::test]
async fn a_reply_without_a_live_claim_writes_nothing() {
    let db = database().await;
    team(&db).await;
    bots_only(&db, "elsewhere", "bot_1", "bot_9").await;
    running_turn(&db, "turn_live", "bot_1", "team", "host-1", at(100)).await;
    running_turn(&db, "turn_old", "bot_1", "team", "host-1", at(40)).await;
    running_turn(&db, "turn_other_bot", "bot_2", "team", "host-1", at(100)).await;
    running_turn(
        &db,
        "turn_other_room",
        "bot_1",
        "elsewhere",
        "host-1",
        at(100),
    )
    .await;
    running_turn(&db, "turn_done", "bot_1", "team", "host-1", at(100)).await;
    let reply = |id: &str| text(id);
    reply_in(
        &db,
        "team",
        "bot_1",
        &claim("turn_done", "host-1"),
        &reply("re_0"),
        at(50),
    )
    .await
    .unwrap();

    for (turn_id, host, reason) in [
        ("turn_missing", "host-1", "not this bot's turn"),
        ("turn_other_bot", "host-1", "not this bot's turn"),
        ("turn_other_room", "host-1", "not this bot's turn"),
        ("turn_done", "host-1", "it is done"),
        ("turn_live", "host-2", "another host claimed it"),
        ("turn_old", "host-1", "lease ran out"),
    ] {
        let message_id = format!("re_{turn_id}");
        let error = reply_in(
            &db,
            "team",
            "bot_1",
            &claim(turn_id, host),
            &reply(&message_id),
            at(50),
        )
        .await
        .unwrap_err();
        assert_code(&error, StatusCode::CONFLICT, BOT_TURN_NOT_CLAIMED);
        assert!(says(&error, reason), "{turn_id}: {error:?}");
        assert!(copies(&db, &message_id).await.is_empty(), "{turn_id}");
    }
    assert_eq!(turn(&db, "turn_live").await.status, "RUNNING");
    let reaction = outgoing(
        "re_react",
        InboxEntryKind::Reaction,
        json!({ "target": "re_0", "emoji": "👍", "op": "add" }),
    );
    let error = reply_in(
        &db,
        "team",
        "bot_1",
        &claim("turn_live", "host-1"),
        &reaction,
        at(50),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    let outsider = reply_in(
        &db,
        "elsewhere",
        "bot_1",
        &claim("turn_other_room", "host-1"),
        &reply("re_x"),
        at(50),
    )
    .await
    .unwrap();
    assert_eq!(outsider.entry.conversation_id, "elsewhere");
}

#[tokio::test]
async fn the_bots_that_receive_an_entry_are_handed_to_their_hosts() {
    let db = database().await;
    team(&db).await;
    insert_bot(&db, "bot_3", "user-a", "DEVICE").await;
    insert_bot(&db, "bot_gone", "user-a", "DESKTOP").await;
    add_in(
        &db,
        "team",
        "user-a",
        &[bot_principal("bot_3"), bot_principal("bot_gone")],
        at(1),
    )
    .await
    .unwrap();
    delete_bot(&db, "bot_gone").await;
    send_in(&db, "team", "bot_2", &text("from_bot_2"), at(2))
        .await
        .unwrap();

    let addressed = outgoing(
        "ask_1",
        InboxEntryKind::Message,
        json!({ "text": "@bot_1 what did bot_2 mean?", "mentions": ["bot_1"], "reply_to": "from_bot_2" }),
    );
    let sent = send_in(&db, "team", "user-b", &addressed, at(3))
        .await
        .unwrap();
    let recipient = |bot_id: &str, owner_id: &str, location, mentioned, replied_to| BotRecipient {
        bot_id: bot_id.into(),
        owner_id: owner_id.into(),
        location,
        device_id: (location == BotLocation::Device).then(|| format!("device-of-{bot_id}")),
        hands_off: false,
        mentioned,
        replied_to,
        read_along: true,
        reacted: false,
        conversation_id: "team".into(),
        trigger_message_id: "ask_1".into(),
        hops: 0,
        interactive: false,
    };
    assert_eq!(
        sent.bots,
        [
            recipient("bot_1", "user-a", BotLocation::Desktop, true, false),
            recipient("bot_2", "user-b", BotLocation::Server, false, true),
            recipient("bot_3", "user-a", BotLocation::Device, false, false),
        ],
        "live bots in member order; a deleted bot gets no turn"
    );
    let unknown_reply = outgoing(
        "ask_2",
        InboxEntryKind::Message,
        json!({ "text": "?", "reply_to": "never_sent" }),
    );
    let sent = send_in(&db, "team", "user-b", &unknown_reply, at(4))
        .await
        .unwrap();
    assert!(
        sent.bots
            .iter()
            .all(|bot| !bot.replied_to && !bot.mentioned)
    );
}

#[tokio::test]
async fn owners_are_woken_for_what_reaches_their_bots() {
    let db = database().await;
    team(&db).await;
    insert_bot(&db, "bot_5", "user-a", "DESKTOP").await;
    insert_bot(&db, "bot_c", "user-c", "SERVER").await;
    bots_only(&db, "pair", "bot_1", "bot_5").await;

    let between_bots = send_in(&db, "pair", "bot_1", &text("hi_5"), at(1))
        .await
        .unwrap();
    assert_eq!(
        between_bots.wake,
        ["user-a"],
        "no person is in it, its bots' owner is woken"
    );
    assert_eq!(between_bots.bots.len(), 1);
    let joined = add_in(&db, "team", "user-a", &[bot_principal("bot_c")], at(2))
        .await
        .unwrap();
    assert_eq!(joined, ["user-a", "user-b", "user-c"]);
    let left = leave(&db, "user-a", "bot_1", &["pair".to_string()], at(3))
        .await
        .unwrap();
    assert_eq!(left.into_iter().collect::<Vec<_>>(), ["user-a"]);
}

#[tokio::test]
async fn owners_read_every_conversation_their_bots_are_in() {
    let db = database().await;
    team(&db).await;
    insert_bot(&db, "bot_5", "user-a", "DESKTOP").await;
    bots_only(&db, "pair", "bot_1", "bot_5").await;
    let as_bot_1 = Reader {
        inbox: "bot_1".into(),
        owner: Some("user-a".into()),
    };
    reader_membership(&db, "pair", &as_bot_1).await.unwrap();
    let listed: BTreeSet<String> = conversations_for(&db, &as_bot_1, None)
        .await
        .unwrap()
        .into_iter()
        .map(|conversation| conversation.id)
        .collect();
    assert_eq!(
        listed,
        BTreeSet::from(["pair".to_string(), "team".to_string()])
    );
    let own: Vec<String> = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap()
        .into_iter()
        .map(|conversation| conversation.id)
        .collect();
    assert_eq!(own, ["team"], "the owner isn't in the bots' exchange");
    let as_bot_2 = Reader {
        inbox: "bot_2".into(),
        owner: Some("user-b".into()),
    };
    let error = reader_membership(&db, "pair", &as_bot_2).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_fingerprint_watches_a_person_s_inbox_and_their_live_bots() {
    let db = database().await;
    team(&db).await;
    insert_bot(&db, "bot_gone", "user-a", "DESKTOP").await;
    delete_bot(&db, "bot_gone").await;
    bots_only(&db, "pair", "bot_1", "bot_gone").await;
    send_in(&db, "pair", "bot_1", &text("hi"), at(1))
        .await
        .unwrap();

    let watched: BTreeSet<String> = inbox_entry::Entity::find()
        .filter(watched_inboxes(inbox_entry::Column::InboxId, "user-a"))
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.inbox_id)
        .collect();
    assert_eq!(
        watched,
        BTreeSet::from(["bot_1".to_string(), "user-a".to_string()])
    );
    let markers: BTreeSet<String> = conversation_member::Entity::find()
        .filter(watched_inboxes(
            conversation_member::Column::PrincipalId,
            "user-b",
        ))
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|member| member.principal_id)
        .collect();
    assert_eq!(
        markers,
        BTreeSet::from(["bot_2".to_string(), "user-b".to_string()])
    );
}

#[tokio::test]
async fn a_persons_unaddressed_message_lets_every_bot_member_read_along_unless_limited_to_mentions()
{
    use super::invitations::set_limits_in;
    use flow_like_types::bot_effective::BotRoomLimits;

    let db = database().await;
    team(&db).await;
    let read_along = |sent: &super::entries::Sent| -> Vec<(String, bool)> {
        sent.bots
            .iter()
            .map(|bot| (bot.bot_id.clone(), bot.read_along))
            .collect()
    };

    let first = send_in(&db, "team", "user-b", &text("plain_1"), at(1))
        .await
        .unwrap();
    assert_eq!(
        read_along(&first),
        [("bot_1".to_string(), true), ("bot_2".to_string(), true)]
    );

    let limits = BotRoomLimits {
        mentioned_only: true,
        ..Default::default()
    };
    set_limits_in(&db, "team", "bot_2", "user-b", Some(&limits))
        .await
        .unwrap();
    let second = send_in(&db, "team", "user-a", &text("plain_2"), at(2))
        .await
        .unwrap();
    assert_eq!(
        read_along(&second),
        [("bot_1".to_string(), true), ("bot_2".to_string(), false)]
    );

    let mention = outgoing(
        "named_1",
        InboxEntryKind::Message,
        json!({ "text": "bot_2?", "mentions": ["bot_2"] }),
    );
    let named = send_in(&db, "team", "user-a", &mention, at(3))
        .await
        .unwrap();
    let bot_2 = named.bots.iter().find(|bot| bot.bot_id == "bot_2").unwrap();
    assert!(bot_2.mentioned && !bot_2.read_along);

    let from_bot = send_in(&db, "team", "bot_1", &text("bot_say"), at(4))
        .await
        .unwrap();
    assert!(
        from_bot.bots.iter().all(|bot| !bot.read_along),
        "a bot's message never wakes the others unaddressed"
    );
}

#[tokio::test]
async fn only_a_persons_new_reaction_to_a_bots_own_message_wakes_that_bot() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "bot_1", &text("b1"), at(1))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &text("p1"), at(2))
        .await
        .unwrap();
    let react = |id: &str, target: &str, op: &str| {
        outgoing(
            id,
            InboxEntryKind::Reaction,
            json!({ "target": target, "emoji": "👎", "op": op }),
        )
    };
    let reacted = |sent: &super::entries::Sent| -> Vec<String> {
        sent.bots
            .iter()
            .filter(|bot| bot.reacted)
            .map(|bot| bot.bot_id.clone())
            .collect()
    };

    let own = send_in(&db, "team", "user-b", &react("r1", "b1", "add"), at(3))
        .await
        .unwrap();
    assert_eq!(reacted(&own), ["bot_1"]);

    let others = send_in(&db, "team", "user-b", &react("r2", "p1", "add"), at(4))
        .await
        .unwrap();
    assert!(
        reacted(&others).is_empty(),
        "a person's message wakes no bot"
    );

    let by_bot = send_in(&db, "team", "bot_2", &react("r3", "b1", "add"), at(5))
        .await
        .unwrap();
    assert!(reacted(&by_bot).is_empty(), "a bot's reaction wakes nobody");

    let removal = send_in(&db, "team", "user-b", &react("r4", "b1", "remove"), at(6))
        .await
        .unwrap();
    assert!(
        reacted(&removal).is_empty(),
        "taking a reaction back wakes nobody"
    );
}
