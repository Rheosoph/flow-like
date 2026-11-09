use super::{
    conversations::{NewConversation, start_group},
    entries::send_in,
    fixture::{at, bot_principal, database, insert_bot, outgoing, user},
    members::add_in,
};
use crate::entity::{bot, json_types::StringList};
use flow_like_types::{
    bot_user_context::*,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection};
use serde_json::{Value, json};

async fn setup() -> (DatabaseConnection, UserContextRequest) {
    let db = database().await;
    insert_bot(&db, "bot", "owner", "SERVER").await;
    insert_bot(&db, "other-bot", "owner", "SERVER").await;
    bot::ActiveModel {
        id: Set("bot".into()),
        capabilities: Set(StringList(vec!["user_context".into()])),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("Team".into()),
        members: vec![],
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        &db,
        "room",
        "owner",
        &plan,
        &[
            user("guest"),
            bot_principal("bot"),
            bot_principal("other-bot"),
        ],
        at(0),
    )
    .await
    .unwrap();
    let request = UserContextRequest::new(
        "ctx1".into(),
        "bot".into(),
        "room".into(),
        UserContextRequestArgs {
            subject_id: "guest".into(),
            fields: vec![UserContextField::Timezone],
            purpose: "Arrange a suitable time".into(),
            precision: UserContextPrecision::Approximate,
            status_change: None,
        },
        UserContextAudience::Group,
        vec![
            "owner".into(),
            "guest".into(),
            "bot".into(),
            "other-bot".into(),
        ],
        at(1).timestamp_millis(),
    );
    (db, request)
}

fn response(request: &UserContextRequest) -> Value {
    let now = at(2).timestamp_millis();
    json!({"text":"Shared context.","reply_to":request.id,"mentions":[request.bot_id],RESPONSE_FIELD:{"request_id":request.id,"bot_id":request.bot_id,"subject_id":request.subject_id,"conversation_id":request.conversation_id,"decision":"once","captured_at":now,"expires_at":now+300_000,"source":{"session_id":"s1","platform":"web"},"values":{"timezone":{"name":"Europe/Berlin"}},"unavailable":{}}})
}

#[tokio::test]
async fn user_context_status_approval_is_subject_owned_and_replay_cannot_restore_it() {
    use crate::routes::user::availability::{load, update};
    use flow_like_types::user_availability::AvailabilityStatus;
    let (db, mut request) = setup().await;
    db.execute_unprepared(r#"INSERT INTO public."User" ("id") VALUES ('guest')"#)
        .await
        .unwrap();
    request.fields = vec![UserContextField::Availability];
    request.status_change = Some(UserContextStatusChange {
        status: UserContextStatus::Focus,
        duration_minutes: 60,
    });
    let card = outgoing(
        &request.id,
        InboxEntryKind::Message,
        json!({REQUEST_FIELD:request}),
    );
    send_in(&db, "room", "bot", &card, at(1)).await.unwrap();
    let until = at(2).timestamp_millis() + 3_600_000;
    let mut body = response(&request);
    body[RESPONSE_FIELD]["values"] =
        json!({"availability":{"status":"focus","source":"bot","until":until}});
    let answer = outgoing(
        &request.response_message_id(),
        InboxEntryKind::Message,
        body,
    );
    assert!(send_in(&db, "room", "owner", &answer, at(2)).await.is_err());
    assert!(load(&db, "guest").await.unwrap().status_override.is_none());
    send_in(&db, "room", "guest", &answer, at(2)).await.unwrap();
    let status = load(&db, "guest")
        .await
        .unwrap()
        .current(at(2).timestamp_millis());
    assert_eq!(status.status, AvailabilityStatus::Focus);
    assert_eq!(status.until, Some(until));
    update(&db, "guest", |stored| {
        stored.status_override = None;
        Ok(())
    })
    .await
    .unwrap();
    send_in(&db, "room", "guest", &answer, at(3)).await.unwrap();
    assert!(
        load(&db, "guest").await.unwrap().status_override.is_none(),
        "replaying a saved approval must not restore a cleared status"
    );
}

#[tokio::test]
async fn user_context_asks_subject_and_wakes_only_requesting_bot() {
    let (db, request) = setup().await;
    let card = outgoing(
        &request.id,
        InboxEntryKind::Message,
        json!({REQUEST_FIELD:request}),
    );
    assert!(send_in(&db, "room", "owner", &card, at(1)).await.is_err());
    let sent = send_in(&db, "room", "bot", &card, at(1)).await.unwrap();
    assert!(
        sent.bots.is_empty(),
        "a context question must not wake group bots"
    );
    let answer = outgoing(
        &request.response_message_id(),
        InboxEntryKind::Message,
        response(&request),
    );
    assert!(
        send_in(&db, "room", "owner", &answer, at(2)).await.is_err(),
        "the bot owner cannot approve for a guest"
    );
    let sent = send_in(&db, "room", "guest", &answer, at(2)).await.unwrap();
    assert_eq!(
        sent.bots
            .iter()
            .map(|bot| bot.bot_id.as_str())
            .collect::<Vec<_>>(),
        vec!["bot"]
    );
    let repeated = send_in(&db, "room", "guest", &answer, at(3)).await.unwrap();
    assert_eq!(sent.entry.id, repeated.entry.id);
    assert!(repeated.bots.is_empty());
    let reply = outgoing(
        "reply1",
        InboxEntryKind::Message,
        json!({"text":"Suggested time","reply_to":request.response_message_id()}),
    );
    let sent = send_in(&db, "room", "bot", &reply, at(3)).await.unwrap();
    assert_eq!(sent.entry.envelope["body"][DERIVED_FIELD], request.id);
    assert!(
        sent.bots.is_empty(),
        "derived answers must not wake another bot"
    );
    let edit = outgoing(
        "edit1",
        InboxEntryKind::Edit,
        json!({"text":"Changed","target":"reply1"}),
    );
    assert!(send_in(&db, "room", "bot", &edit, at(4)).await.is_err());
}

#[tokio::test]
async fn user_context_rechecks_audience_and_capability_at_response() {
    let (db, request) = setup().await;
    let card = outgoing(
        &request.id,
        InboxEntryKind::Message,
        json!({REQUEST_FIELD:request}),
    );
    send_in(&db, "room", "bot", &card, at(1)).await.unwrap();
    let answer = outgoing(
        &request.response_message_id(),
        InboxEntryKind::Message,
        response(&request),
    );
    bot::ActiveModel {
        id: Set("bot".into()),
        capabilities: Set(StringList(vec![])),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert!(send_in(&db, "room", "guest", &answer, at(2)).await.is_err());
    bot::ActiveModel {
        id: Set("bot".into()),
        capabilities: Set(StringList(vec!["user_context".into()])),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    add_in(&db, "room", "owner", &[user("new-person")], at(2))
        .await
        .unwrap();
    assert!(send_in(&db, "room", "guest", &answer, at(2)).await.is_err());
}

#[tokio::test]
async fn user_context_rechecks_audience_before_derived_answer() {
    let (db, request) = setup().await;
    let card = outgoing(
        &request.id,
        InboxEntryKind::Message,
        json!({REQUEST_FIELD:request}),
    );
    send_in(&db, "room", "bot", &card, at(1)).await.unwrap();
    let answer = outgoing(
        &request.response_message_id(),
        InboxEntryKind::Message,
        response(&request),
    );
    send_in(&db, "room", "guest", &answer, at(2)).await.unwrap();
    add_in(&db, "room", "owner", &[user("new-person")], at(3))
        .await
        .unwrap();
    let reply = outgoing(
        "reply1",
        InboxEntryKind::Message,
        json!({"text":"Suggested time","reply_to":request.response_message_id()}),
    );
    assert!(send_in(&db, "room", "bot", &reply, at(4)).await.is_err());
}

#[tokio::test]
async fn user_context_deleted_response_refuses_derived_answer() {
    let (db, request) = setup().await;
    let card = outgoing(
        &request.id,
        InboxEntryKind::Message,
        json!({REQUEST_FIELD:request}),
    );
    send_in(&db, "room", "bot", &card, at(1)).await.unwrap();
    let answer = outgoing(
        &request.response_message_id(),
        InboxEntryKind::Message,
        response(&request),
    );
    send_in(&db, "room", "guest", &answer, at(2)).await.unwrap();
    let deleted = outgoing(
        "delete1",
        InboxEntryKind::Delete,
        json!({"target":request.response_message_id()}),
    );
    send_in(&db, "room", "guest", &deleted, at(3))
        .await
        .unwrap();
    let reply = outgoing(
        "reply1",
        InboxEntryKind::Message,
        json!({"text":"Suggested time","reply_to":request.response_message_id()}),
    );
    assert!(send_in(&db, "room", "bot", &reply, at(4)).await.is_err());
}

#[tokio::test]
async fn user_context_unearned_derived_marker_is_rejected() {
    let (db, request) = setup().await;
    let plain = outgoing(
        "plain1",
        InboxEntryKind::Message,
        json!({"text":"Hello",DERIVED_FIELD:request.id}),
    );
    assert!(send_in(&db, "room", "bot", &plain, at(1)).await.is_err());
    let unrelated = outgoing(
        "plain2",
        InboxEntryKind::Message,
        json!({"text":"Hello","reply_to":"missing",DERIVED_FIELD:request.id}),
    );
    assert!(send_in(&db, "room", "bot", &unrelated, at(2)).await.is_err());
}
