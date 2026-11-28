use super::{
    people::remove_person_in,
    teams::delete_team_in,
    tests::{create, join, own_bot, patch, team, world},
    threads::thread_of,
};
use crate::{
    bot_notify::Reply,
    entity::{bot_team_thread, bot_turn, conversation_member, inbox_entry},
    routes::inbox::{
        conversations::start_direct,
        exchange::post_as_bot_in,
        fixture::{at, bot_principal, group, outgoing, user},
        team_request::{
            Origin, plan_answer, request_message_id, resolve_teams, route_request_in,
            write_answer_in,
        },
    },
};
use flow_like_types::{
    bot_message::MessageBody,
    bot_team::TeamRequestRef,
    bot_turn::{failure_message_id, reply_message_id, turn_id},
    inbox::{ConversationEncryption, InboxEntryKind},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::json;
use std::collections::BTreeSet;

use super::model::PatchTeamRequest;

async fn send_in(
    db: &DatabaseConnection,
    conversation_id: &str,
    sender: &str,
    outgoing: &crate::routes::inbox::entries::Outgoing,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<crate::routes::inbox::entries::Sent, crate::error::ApiError> {
    post_as_bot_in(db, conversation_id, sender, 0, outgoing, now).await
}

fn lead(bot_id: &str) -> PatchTeamRequest {
    PatchTeamRequest {
        lead_bot_id: Some(Some(bot_id.into())),
        ..Default::default()
    }
}

/// Kestrel with a Security team: mara admin, jonas and anna members, jonas-bot leading.
async fn security(db: &DatabaseConnection) -> String {
    let kestrel = create(db, "mara", "Kestrel", None).await.unwrap();
    let security = create(db, "mara", "Security", Some(&kestrel))
        .await
        .unwrap();
    join(db, "mara", &security, "jonas").await;
    join(db, "mara", &security, "anna").await;
    own_bot(db, "jonas", &security, "jonas-bot").await;
    patch(db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    super::lead::accept_lead_in(db, "jonas", &security, "jonas-bot", at(5))
        .await
        .unwrap();
    security
}

async fn members(db: &DatabaseConnection, conversation_id: &str) -> BTreeSet<String> {
    conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::LeftAt.is_null())
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|member| member.principal_id)
        .collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

async fn thread_members(db: &DatabaseConnection, team_id: &str, user_id: &str) -> BTreeSet<String> {
    let thread = thread_of(db, team_id, user_id).await.unwrap().unwrap();
    members(db, &thread.conversation_id).await
}

async fn turns(db: &DatabaseConnection) -> Vec<bot_turn::Model> {
    bot_turn::Entity::find().all(db).await.unwrap()
}

async fn resolved(db: &DatabaseConnection, who: &str, text: &str) -> Vec<String> {
    resolve_teams(db, who, text)
        .await
        .unwrap()
        .into_iter()
        .map(|team| team.id)
        .collect()
}

fn origin(conversation_id: &str, message_id: &str, text: &str) -> Origin {
    Origin {
        conversation_id: conversation_id.into(),
        message_id: message_id.into(),
        text: text.into(),
    }
}

#[tokio::test]
async fn only_leading_tags_of_a_persons_own_orgs_with_a_live_lead_resolve() {
    let db = world().await;
    let security = security(&db).await;

    assert_eq!(
        resolved(&db, "mara", "#security is the VPN down?").await,
        [security.clone()]
    );
    assert!(
        resolved(&db, "mara", "is the VPN down? #Security")
            .await
            .is_empty()
    );
    assert!(resolved(&db, "mara", "#Nowhere hello").await.is_empty());
    assert!(
        resolved(&db, "outsider", "#Security hello")
            .await
            .is_empty(),
        "a person outside the org reaches no team"
    );
    patch(
        &db,
        "mara",
        &security,
        PatchTeamRequest {
            lead_bot_id: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        resolved(&db, "mara", "#Security hello").await.is_empty(),
        "a team without a lead is ignored"
    );
}

#[tokio::test]
async fn the_oldest_team_of_a_tag_wins() {
    let db = world().await;
    let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
    let first = create(&db, "mara", "Ops", Some(&kestrel)).await.unwrap();
    let second = create(&db, "mara", "Ops", Some(&kestrel)).await.unwrap();
    own_bot(&db, "mara", &first, "mara-bot").await;
    own_bot(&db, "mara", &second, "mara-bot-2").await;
    patch(&db, "mara", &first, lead("mara-bot")).await.unwrap();
    patch(&db, "mara", &second, lead("mara-bot-2"))
        .await
        .unwrap();

    let (a, b) = (team(&db, &first).await, team(&db, &second).await);
    let older = if (a.created_at, &a.id) <= (b.created_at, &b.id) {
        a.id
    } else {
        b.id
    };
    assert_eq!(resolved(&db, "mara", "#Ops go").await, [older]);
}

#[tokio::test]
async fn a_team_addressed_message_wakes_no_bot_of_its_own_chat() {
    let db = world().await;
    security(&db).await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();

    let plain = outgoing("m1", InboxEntryKind::Message, json!({ "text": "hello" }));
    let sent = send_in(&db, &chat, "anna", &plain, at(10)).await.unwrap();
    assert_eq!(sent.bots.len(), 1);
    assert_eq!(
        turns(&db).await.len(),
        1,
        "an ordinary message wakes the bot"
    );

    let tagged = outgoing(
        "m2",
        InboxEntryKind::Message,
        json!({ "text": "#Security is the VPN down?" }),
    );
    send_in(&db, &chat, "anna", &tagged, at(11)).await.unwrap();
    assert_eq!(
        turns(&db).await.len(),
        1,
        "a team request wakes nobody here"
    );
    let stored = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .filter(inbox_entry::Column::MessageId.eq("m2"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let body = MessageBody::parse(&stored.envelope["body"]).unwrap();
    assert_eq!(body.teams.len(), 1);

    let forged = outgoing(
        "m3",
        InboxEntryKind::Message,
        json!({ "text": "hello again", "teams": ["whatever"] }),
    );
    send_in(&db, &chat, "anna", &forged, at(12)).await.unwrap();
    assert_eq!(
        turns(&db).await.len(),
        2,
        "a teams list a client wrote is dropped"
    );
}

#[tokio::test]
async fn a_request_lands_in_a_thread_with_the_person_the_lead_and_its_owner() {
    let db = world().await;
    let security = security(&db).await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();

    let routed = route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m1", "#Security vpn?"),
        at(10),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        thread_members(&db, &security, "anna").await,
        set(&["anna", "jonas", "jonas-bot"])
    );
    let thread = thread_of(&db, &security, "anna").await.unwrap().unwrap();
    let message_id = request_message_id(&chat, "m1", &security);
    let copy = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .filter(inbox_entry::Column::MessageId.eq(&message_id))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(copy.conversation_id, thread.conversation_id);
    let body = MessageBody::parse(&copy.envelope["body"]).unwrap();
    assert_eq!(body.text, "#Security vpn?");
    assert_eq!(body.mentions, ["jonas-bot"]);
    assert_eq!(
        body.team_request,
        Some(TeamRequestRef {
            team_id: security.clone(),
            origin_conversation_id: chat.clone(),
            origin_message_id: "m1".into(),
            answer_here: true,
        })
    );
    assert!(body.teams.is_empty());
    assert_eq!(routed.sent.bots.len(), 1);
    assert_eq!(turns(&db).await.len(), 1);

    route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m2", "#Security again"),
        at(11),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        bot_team_thread::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .len(),
        1,
        "the second request reuses the thread"
    );

    route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m2", "#Security again"),
        at(12),
    )
    .await
    .unwrap()
    .unwrap();
    let copies = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .filter(inbox_entry::Column::MessageId.eq(request_message_id(&chat, "m2", &security)))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(copies.len(), 1, "the same origin message posts once");
}

#[tokio::test]
async fn a_lead_change_swaps_the_bot_and_its_owner_in_every_thread() {
    let db = world().await;
    let security = security(&db).await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m1", "#Security x"),
        at(10),
    )
    .await
    .unwrap();

    own_bot(&db, "mara", &security, "mara-bot").await;
    patch(&db, "mara", &security, lead("mara-bot"))
        .await
        .unwrap();
    assert_eq!(
        thread_members(&db, &security, "anna").await,
        set(&["anna", "mara", "mara-bot"])
    );
}

#[tokio::test]
async fn a_person_who_leaves_the_org_takes_the_lead_out_of_their_thread() {
    let db = world().await;
    let security = security(&db).await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m1", "#Security x"),
        at(10),
    )
    .await
    .unwrap();

    remove_person_in(&db, "anna", &security, "anna", at(20))
        .await
        .unwrap();
    assert_eq!(thread_members(&db, &security, "anna").await, set(&["anna"]));
    assert!(
        route_request_in(
            &db,
            "anna",
            &security,
            &origin(&chat, "m2", "#Security y"),
            at(21)
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn deleting_a_team_takes_the_lead_out_and_keeps_the_history() {
    let db = world().await;
    let security = security(&db).await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    route_request_in(
        &db,
        "anna",
        &security,
        &origin(&chat, "m1", "#Security x"),
        at(10),
    )
    .await
    .unwrap();
    let thread = thread_of(&db, &security, "anna").await.unwrap().unwrap();

    delete_team_in(&db, "mara", &security, at(30))
        .await
        .unwrap();
    assert_eq!(members(&db, &thread.conversation_id).await, set(&["anna"]));
    assert!(thread_of(&db, &security, "anna").await.unwrap().is_none());
}

struct Answering {
    chat: String,
    trigger: String,
    turn: String,
    reply: String,
}

async fn answering(db: &DatabaseConnection, origin_chat: &str) -> (String, Answering) {
    let security = security(db).await;
    let text = outgoing(
        "m1",
        InboxEntryKind::Message,
        json!({ "text": "#Security vpn?" }),
    );
    send_in(db, origin_chat, "anna", &text, at(5))
        .await
        .unwrap();
    route_request_in(
        db,
        "anna",
        &security,
        &origin(origin_chat, "m1", "#Security vpn?"),
        at(10),
    )
    .await
    .unwrap()
    .unwrap();
    let trigger = request_message_id(origin_chat, "m1", &security);
    let found = Answering {
        chat: origin_chat.into(),
        turn: turn_id("jonas-bot", &trigger),
        reply: reply_message_id("jonas-bot", &trigger),
        trigger,
    };
    (security, found)
}

async fn answer_in_thread(
    db: &DatabaseConnection,
    team_id: &str,
    found: &Answering,
    message_id: &str,
) {
    let thread = thread_of(db, team_id, "anna").await.unwrap().unwrap();
    let reply = outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({ "text": "Yes, it is." }),
    );
    post_as_bot_in(db, &thread.conversation_id, "jonas-bot", 1, &reply, at(15))
        .await
        .unwrap();
    assert!(!found.turn.is_empty());
}

async fn activities(db: &DatabaseConnection, chat: &str) -> Vec<inbox_entry::Model> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .filter(inbox_entry::Column::ConversationId.eq(chat))
        .filter(inbox_entry::Column::Kind.eq("ACTIVITY"))
        .all(db)
        .await
        .unwrap()
}

fn reply_of<'a>(found: &'a Answering, message_id: &'a str) -> Reply<'a> {
    Reply {
        turn_id: &found.turn,
        bot_id: "jonas-bot",
        message_id,
        text: "Yes, it is.",
        resumed: false,
    }
}

#[tokio::test]
async fn an_answer_to_a_request_from_the_own_bot_chat_is_copied_back_once() {
    let db = world().await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    let (security, found) = answering(&db, &chat).await;
    answer_in_thread(&db, &security, &found, &found.reply).await;

    let plan = plan_answer(&db, &reply_of(&found, &found.reply))
        .await
        .unwrap()
        .unwrap();
    write_answer_in(&db, &plan, at(16)).await.unwrap();
    write_answer_in(&db, &plan, at(17)).await.unwrap();
    let written = activities(&db, &found.chat).await;
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].envelope["body"]["type"], "team_answer");
    assert_eq!(written[0].envelope["body"]["text"], "Yes, it is.");
    assert_eq!(written[0].envelope["body"]["request_message_id"], "m1");
}

#[tokio::test]
async fn a_failure_reply_is_not_copied_back() {
    let db = world().await;
    let chat = start_direct(
        &db,
        "anna",
        &bot_principal("anna-bot"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    let (_, found) = answering(&db, &chat).await;
    let failure = failure_message_id("jonas-bot", &found.trigger);
    assert!(
        plan_answer(&db, &reply_of(&found, &failure))
            .await
            .unwrap()
            .is_none()
    );
    assert!(activities(&db, &found.chat).await.is_empty());
}

#[tokio::test]
async fn an_answer_stays_in_the_thread_when_the_request_came_from_elsewhere() {
    let db = world().await;
    group(
        &db,
        "friends",
        "anna",
        &[user("mara")],
        ConversationEncryption::Plain,
    )
    .await;
    let (security, found) = answering(&db, "friends").await;
    answer_in_thread(&db, &security, &found, &found.reply).await;
    assert!(
        plan_answer(&db, &reply_of(&found, &found.reply))
            .await
            .unwrap()
            .is_none()
    );
    assert!(activities(&db, "friends").await.is_empty());
}
