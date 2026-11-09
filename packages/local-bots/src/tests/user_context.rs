use super::*;
use flow_like_types::bot_user_context::*;

fn make_request(room: &Conversation, bot: &Bot, id: &str) -> UserContextRequest {
    UserContextRequest::new(
        id.into(),
        bot.id.clone(),
        room.id.clone(),
        UserContextRequestArgs {
            subject_id: "local".into(),
            fields: vec![UserContextField::Timezone],
            purpose: "Arrange a suitable time".into(),
            precision: UserContextPrecision::Approximate,
            status_change: None,
        },
        if room.kind == flow_like_types::inbox::ConversationKind::Direct {
            UserContextAudience::Direct
        } else {
            UserContextAudience::Group
        },
        room.members
            .iter()
            .map(|m| m.principal_id.clone())
            .collect(),
        chrono::Utc::now().timestamp_millis(),
    )
}

fn response_body(request: &UserContextRequest) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    json!({"text":"Shared requested context.","reply_to":request.id,"mentions":[request.bot_id],RESPONSE_FIELD:{"request_id":request.id,"bot_id":request.bot_id,"subject_id":request.subject_id,"conversation_id":request.conversation_id,"decision":"once","captured_at":now,"expires_at":now+300_000,"source":{"session_id":"s1","platform":"web"},"values":{"timezone":{"name":"Europe/Berlin"}},"unavailable":{}}})
}

fn enabled_bot(f: &Fixture, name: &str) -> Bot {
    f.store
        .create_bot(create(
            json!({"name":name,"appearance":{"kind":"pip"},"capabilities":["user_context"]}),
        ))
        .unwrap()
}

#[test]
fn user_context_requires_capability_and_subject_and_cannot_be_edited() {
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    let request = make_request(&room, &bot, "ctx1");
    assert!(
        f.send(
            &room.id,
            &bot.id,
            &request.id,
            "message",
            json!({REQUEST_FIELD:request})
        )
        .is_err()
    );
    let bot = enabled_bot(&f, "Guide");
    let room = f.direct(&bot);
    let request = make_request(&room, &bot, "ctx2");
    assert!(
        f.send(
            &room.id,
            "local",
            &request.id,
            "message",
            json!({REQUEST_FIELD:request})
        )
        .is_err()
    );
    f.send(
        &room.id,
        &bot.id,
        &request.id,
        "message",
        json!({REQUEST_FIELD:request}),
    )
    .unwrap();
    assert!(
        f.send(
            &room.id,
            &bot.id,
            "edit1",
            "edit",
            json!({"target":request.id,"text":"changed"})
        )
        .is_err()
    );
    assert!(
        f.send(
            &room.id,
            &bot.id,
            &request.response_message_id(),
            "message",
            response_body(&request)
        )
        .is_err()
    );
    let answer = f
        .send(
            &room.id,
            "local",
            &request.response_message_id(),
            "message",
            response_body(&request),
        )
        .unwrap();
    let repeated = f
        .send(
            &room.id,
            "local",
            &request.response_message_id(),
            "message",
            response_body(&request),
        )
        .unwrap();
    assert_eq!(answer.id, repeated.id);
    let reply = f
        .send(
            &room.id,
            &bot.id,
            "reply1",
            "message",
            json!({"reply_to":answer.message_id,"text":"Tomorrow at 10."}),
        )
        .unwrap();
    assert_eq!(reply.envelope["body"][DERIVED_FIELD], request.id);
    assert!(
        f.send(
            &room.id,
            &bot.id,
            "edit2",
            "edit",
            json!({"target":"reply1","text":"changed"})
        )
        .is_err()
    );
}

#[test]
fn user_context_rejects_a_changed_group_audience() {
    let f = fixture();
    let bot = enabled_bot(&f, "Scout");
    let other = f.bot("Other");
    let room = f.group(&[&bot]);
    let request = make_request(&room, &bot, "ctx1");
    f.send(
        &room.id,
        &bot.id,
        &request.id,
        "message",
        json!({REQUEST_FIELD:request}),
    )
    .unwrap();
    f.store.add_members(&room.id, vec![other.id]).unwrap();
    assert!(
        f.send(
            &room.id,
            "local",
            &request.response_message_id(),
            "message",
            response_body(&request)
        )
        .is_err()
    );
}

#[test]
fn user_context_rejects_expiry_and_relabelled_responses() {
    let f = fixture();
    let bot = enabled_bot(&f, "Scout");
    let room = f.direct(&bot);
    let mut request = make_request(&room, &bot, "ctx1");
    request.created_at -= 400_000;
    request.expires_at -= 400_000;
    assert!(
        f.send(
            &room.id,
            &bot.id,
            &request.id,
            "message",
            json!({REQUEST_FIELD:request})
        )
        .is_err()
    );
    let request = make_request(&room, &bot, "ctx2");
    f.send(
        &room.id,
        &bot.id,
        &request.id,
        "message",
        json!({REQUEST_FIELD:request}),
    )
    .unwrap();
    assert!(
        f.send(
            &room.id,
            "local",
            "another_response",
            "message",
            response_body(&request)
        )
        .is_err()
    );
}

#[test]
fn user_context_answer_is_not_delivered_to_people_who_join_after_consent() {
    let f = fixture();
    let bot = enabled_bot(&f, "Scout");
    let other = f.bot("Other");
    let room = f.group(&[&bot]);
    let request = make_request(&room, &bot, "ctx1");
    f.send(
        &room.id,
        &bot.id,
        &request.id,
        "message",
        json!({REQUEST_FIELD:request}),
    )
    .unwrap();
    f.send(
        &room.id,
        "local",
        &request.response_message_id(),
        "message",
        response_body(&request),
    )
    .unwrap();
    f.store.add_members(&room.id, vec![other.id]).unwrap();
    assert!(
        f.send(
            &room.id,
            &bot.id,
            "reply1",
            "message",
            json!({"reply_to":request.response_message_id(),"text":"Private contextual answer"})
        )
        .is_err()
    );
}

#[test]
fn user_context_deleted_snapshot_does_not_leave_an_unmarked_answer() {
    let f = fixture();
    let bot = enabled_bot(&f, "Scout");
    let room = f.direct(&bot);
    let request = make_request(&room, &bot, "ctx1");
    f.send(
        &room.id,
        &bot.id,
        &request.id,
        "message",
        json!({REQUEST_FIELD:request}),
    )
    .unwrap();
    f.send(
        &room.id,
        "local",
        &request.response_message_id(),
        "message",
        response_body(&request),
    )
    .unwrap();
    f.send(
        &room.id,
        "local",
        "delete1",
        "delete",
        json!({"target":request.response_message_id()}),
    )
    .unwrap();
    assert!(
        f.send(
            &room.id,
            &bot.id,
            "reply1",
            "message",
            json!({"reply_to":request.response_message_id(),"text":"Private contextual answer"})
        )
        .is_err()
    );
}

#[test]
fn every_bot_message_of_a_context_turn_is_derived() {
    let f = fixture();
    let bot = enabled_bot(&f, "Guide");
    let room = f.direct(&bot);
    let request = make_request(&room, &bot, "ctx9");
    f.send(&room.id, &bot.id, &request.id, "message", json!({REQUEST_FIELD:request}))
        .unwrap();
    f.send(&room.id, "local", &request.response_message_id(), "message", response_body(&request))
        .unwrap();
    let first = f
        .send(&room.id, &bot.id, "plain1", "message", json!({"text":"One moment."}))
        .unwrap();
    let second = f
        .send(&room.id, &bot.id, "plain2", "message", json!({"text":"Done."}))
        .unwrap();
    assert_eq!(first.envelope["body"][DERIVED_FIELD], request.id);
    assert_eq!(second.envelope["body"][DERIVED_FIELD], request.id);
    f.send(&room.id, "local", "human1", "message", json!({"text":"Thanks"}))
        .unwrap();
    let later = f
        .send(&room.id, &bot.id, "plain3", "message", json!({"text":"Welcome."}))
        .unwrap();
    assert!(later.envelope["body"].get(DERIVED_FIELD).is_none());
}
