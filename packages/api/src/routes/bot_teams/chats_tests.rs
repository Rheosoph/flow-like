use super::{
    access::{TEAM_LIMIT, TEAM_NOT_FOUND},
    chats::{
        MAX_PENDING_CHATS, TEAM_CHAT_DECLINED, TEAM_CHAT_EXISTS, TEAM_CHAT_LEAD, accept_chat_in,
        decline_chat_in, end_chat_in, request_chat_in, sync_chats_of,
    },
    effects::Effects,
    model::{AcceptChatRequest, ChatRequestBody},
    people::remove_person_in,
    tests::{create, join, own_bot, world},
    threads::{TEAM_CLASSIFIED, TEAM_NO_LEAD, open_thread_in},
};
use crate::{
    entity::{
        bot, bot_org_classification, bot_team, bot_team_bot, bot_team_chat, bot_turn,
        conversation_member, inbox_entry,
    },
    routes::inbox::{
        entries::fan_out,
        fixture::{assert_code, at, insert_bot, text},
        person_push::{is_muted, is_watching, set_muted_in},
    },
};
use axum::http::StatusCode;
use flow_like_types::{bot_effective::BotRoomLimits, bot_team::BotChatStatus};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    sea_query::Expr,
};
use std::collections::BTreeSet;

const DAY: i64 = 86_400;

async fn org(db: &DatabaseConnection) -> String {
    let kestrel = create(db, "mara", "Kestrel", None).await.unwrap();
    join(db, "mara", &kestrel, "jonas").await;
    join(db, "mara", &kestrel, "anna").await;
    own_bot(db, "jonas", &kestrel, "jonas-bot").await;
    own_bot(db, "mara", &kestrel, "mara-bot").await;
    kestrel
}

async fn lead(db: &DatabaseConnection, team_id: &str, bot_id: &str) {
    bot_team::Entity::update_many()
        .col_expr(bot_team::Column::LeadBotId, Expr::value(bot_id.to_string()))
        .filter(bot_team::Column::Id.eq(team_id))
        .exec(db)
        .await
        .unwrap();
}

async fn classify_bot(db: &DatabaseConnection, bot_id: &str, level: &str) {
    bot::Entity::update_many()
        .col_expr(bot::Column::Classification, Expr::value(level.to_string()))
        .filter(bot::Column::Id.eq(bot_id))
        .exec(db)
        .await
        .unwrap();
}

async fn classify_person(db: &DatabaseConnection, org_id: &str, user_id: &str, level: &str) {
    bot_org_classification::Entity::delete_by_id((org_id.to_string(), user_id.to_string()))
        .exec(db)
        .await
        .unwrap();
    bot_org_classification::ActiveModel {
        org_id: Set(org_id.into()),
        user_id: Set(user_id.into()),
        classification: Set(level.into()),
        set_by: Set("mara".into()),
        updated_at: Set(at(1)),
    }
    .insert(db)
    .await
    .unwrap();
}

fn ask(bot_id: &str) -> ChatRequestBody {
    ChatRequestBody {
        bot_id: bot_id.into(),
        note: None,
    }
}

async fn row(db: &DatabaseConnection, bot_id: &str, user_id: &str) -> bot_team_chat::Model {
    bot_team_chat::Entity::find_by_id((bot_id.to_string(), user_id.to_string()))
        .one(db)
        .await
        .unwrap()
        .unwrap()
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

async fn member(
    db: &DatabaseConnection,
    conversation_id: &str,
    principal: &str,
) -> conversation_member::Model {
    conversation_member::Entity::find_by_id((conversation_id.to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn open_chat(db: &DatabaseConnection) -> String {
    request_chat_in(db, "anna", &ask("jonas-bot"), at(10))
        .await
        .unwrap();
    let (chat, _) = accept_chat_in(
        db,
        "jonas",
        "jonas-bot",
        "anna",
        &AcceptChatRequest::default(),
        at(11),
    )
    .await
    .unwrap();
    chat.conversation_id.unwrap()
}

async fn turns(db: &DatabaseConnection) -> Vec<bot_turn::Model> {
    bot_turn::Entity::find().all(db).await.unwrap()
}

async fn notices(db: &DatabaseConnection, conversation_id: &str, bot_id: &str) -> usize {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::SenderId.eq(bot_id))
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .all(db)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn a_request_waits_for_the_owner_who_is_told() {
    let db = world().await;
    org(&db).await;
    let (chat, effects) = request_chat_in(
        &db,
        "anna",
        &ChatRequestBody {
            bot_id: "jonas-bot".into(),
            note: Some("  about the audit  ".into()),
        },
        at(10),
    )
    .await
    .unwrap();
    assert_eq!(chat.status, BotChatStatus::Pending);
    assert_eq!(chat.owner_id, "jonas");
    assert_eq!(chat.note.as_deref(), Some("about the audit"));
    assert_eq!(effects.notices.len(), 1);
    assert_eq!(effects.notices[0].user_id, "jonas");

    let (again, effects) = request_chat_in(&db, "anna", &ask("jonas-bot"), at(11))
        .await
        .unwrap();
    assert_eq!(again, chat);
    assert!(effects.notices.is_empty());
}

#[tokio::test]
async fn a_request_is_refused_for_every_bot_that_cannot_be_asked() {
    let db = world().await;
    let kestrel = org(&db).await;
    let outsider = request_chat_in(&db, "outsider", &ask("jonas-bot"), at(10))
        .await
        .unwrap_err();
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
    let unknown = request_chat_in(&db, "anna", &ask("ghost"), at(10))
        .await
        .unwrap_err();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let outside_bot = request_chat_in(&db, "anna", &ask("anna-bot"), at(10))
        .await
        .unwrap_err();
    assert_eq!(outside_bot.status(), StatusCode::NOT_FOUND);

    let own = request_chat_in(&db, "jonas", &ask("jonas-bot"), at(10))
        .await
        .unwrap_err();
    assert_eq!(own.status(), StatusCode::BAD_REQUEST);

    lead(&db, &kestrel, "jonas-bot").await;
    let led = request_chat_in(&db, "anna", &ask("jonas-bot"), at(10))
        .await
        .unwrap_err();
    assert_code(&led, StatusCode::CONFLICT, TEAM_CHAT_LEAD);

    classify_bot(&db, "mara-bot", "confidential").await;
    let above = request_chat_in(&db, "anna", &ask("mara-bot"), at(10))
        .await
        .unwrap_err();
    assert_code(&above, StatusCode::FORBIDDEN, TEAM_CLASSIFIED);
    assert!(above.public_message().unwrap().contains("confidential"));
    classify_person(&db, &kestrel, "anna", "confidential").await;
    request_chat_in(&db, "anna", &ask("mara-bot"), at(10))
        .await
        .unwrap();

    lead(&db, &kestrel, "mara-bot").await;
    let long = request_chat_in(
        &db,
        "mara",
        &ChatRequestBody {
            bot_id: "jonas-bot".into(),
            note: Some("x".repeat(281)),
        },
        at(10),
    )
    .await
    .unwrap_err();
    assert_eq!(long.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accepting_opens_a_conversation_of_the_person_the_bot_and_the_watching_owner() {
    let db = world().await;
    org(&db).await;
    request_chat_in(&db, "anna", &ask("jonas-bot"), at(10))
        .await
        .unwrap();
    let stranger = accept_chat_in(
        &db,
        "anna",
        "jonas-bot",
        "anna",
        &AcceptChatRequest::default(),
        at(11),
    )
    .await
    .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    let limits = BotRoomLimits {
        mask: Some(4),
        ..Default::default()
    };
    let (chat, _) = accept_chat_in(
        &db,
        "jonas",
        "jonas-bot",
        "anna",
        &AcceptChatRequest {
            limits: Some(limits),
        },
        at(11),
    )
    .await
    .unwrap();
    assert_eq!(chat.status, BotChatStatus::Active);
    let conversation = chat.conversation_id.unwrap();
    assert_eq!(
        members(&db, &conversation).await,
        set(&["anna", "jonas-bot", "jonas"])
    );
    assert!(is_watching(
        member(&db, &conversation, "jonas").await.limits.as_ref()
    ));
    assert!(!is_watching(
        member(&db, &conversation, "anna").await.limits.as_ref()
    ));
    let bot_limits: BotRoomLimits = serde_json::from_value(
        member(&db, &conversation, "jonas-bot")
            .await
            .limits
            .unwrap(),
    )
    .unwrap();
    assert!(!bot_limits.mentioned_only);
    assert_eq!(bot_limits.mask, Some(4));

    let existing = request_chat_in(&db, "anna", &ask("jonas-bot"), at(12))
        .await
        .unwrap_err();
    assert_code(&existing, StatusCode::CONFLICT, TEAM_CHAT_EXISTS);
}

#[tokio::test]
async fn a_declined_request_waits_a_week_and_an_ended_chat_can_be_asked_again() {
    let db = world().await;
    org(&db).await;
    request_chat_in(&db, "anna", &ask("jonas-bot"), at(10))
        .await
        .unwrap();
    assert_eq!(
        decline_chat_in(&db, "anna", "jonas-bot", "anna", at(11))
            .await
            .unwrap_err()
            .status(),
        StatusCode::NOT_FOUND
    );
    decline_chat_in(&db, "jonas", "jonas-bot", "anna", at(11))
        .await
        .unwrap();
    assert_eq!(row(&db, "jonas-bot", "anna").await.status, "DECLINED");
    let early = request_chat_in(&db, "anna", &ask("jonas-bot"), at(11 + 6 * DAY))
        .await
        .unwrap_err();
    assert_code(&early, StatusCode::CONFLICT, TEAM_CHAT_DECLINED);

    let (renewed, _) = request_chat_in(
        &db,
        "anna",
        &ChatRequestBody {
            bot_id: "jonas-bot".into(),
            note: Some("again".into()),
        },
        at(11 + 8 * DAY),
    )
    .await
    .unwrap();
    assert_eq!(renewed.status, BotChatStatus::Pending);
    assert_eq!(renewed.decided_at, None);
    assert_eq!(renewed.note.as_deref(), Some("again"));

    let conversation = {
        accept_chat_in(
            &db,
            "jonas",
            "jonas-bot",
            "anna",
            &AcceptChatRequest::default(),
            at(12 + 8 * DAY),
        )
        .await
        .unwrap()
        .0
        .conversation_id
        .unwrap()
    };
    end_chat_in(&db, "anna", "jonas-bot", "anna", at(13 + 8 * DAY))
        .await
        .unwrap();
    assert_eq!(members(&db, &conversation).await, set(&["anna"]));
    let (again, _) = request_chat_in(&db, "anna", &ask("jonas-bot"), at(14 + 8 * DAY))
        .await
        .unwrap();
    assert_eq!(again.status, BotChatStatus::Pending);
    let (reopened, _) = accept_chat_in(
        &db,
        "jonas",
        "jonas-bot",
        "anna",
        &AcceptChatRequest::default(),
        at(15 + 8 * DAY),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened.conversation_id.as_deref(),
        Some(conversation.as_str())
    );
    assert_eq!(
        members(&db, &conversation).await,
        set(&["anna", "jonas-bot", "jonas"])
    );
}

#[tokio::test]
async fn a_person_has_at_most_ten_requests_waiting() {
    let db = world().await;
    let kestrel = org(&db).await;
    for index in 0..MAX_PENDING_CHATS {
        let id = format!("extra-{index}");
        insert_bot(&db, &id, "jonas", "SERVER").await;
        own_bot(&db, "jonas", &kestrel, &id).await;
        request_chat_in(&db, "anna", &ask(&id), at(10))
            .await
            .unwrap();
    }
    let over = request_chat_in(&db, "anna", &ask("jonas-bot"), at(11))
        .await
        .unwrap_err();
    assert_code(&over, StatusCode::CONFLICT, TEAM_LIMIT);
}

#[tokio::test]
async fn ending_a_chat_leaves_the_person_with_it_and_only_the_two_sides_may_end_it() {
    let db = world().await;
    org(&db).await;
    let conversation = open_chat(&db).await;
    let stranger = end_chat_in(&db, "mara", "jonas-bot", "anna", at(12))
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);
    end_chat_in(&db, "jonas", "jonas-bot", "anna", at(12))
        .await
        .unwrap();
    let ended = row(&db, "jonas-bot", "anna").await;
    assert_eq!(ended.status, "ENDED");
    assert!(ended.ended_at.is_some());
    assert_eq!(members(&db, &conversation).await, set(&["anna"]));
    let twice = end_chat_in(&db, "jonas", "jonas-bot", "anna", at(13))
        .await
        .unwrap_err();
    assert_eq!(twice.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_person_withdraws_a_pending_request_and_the_owner_cannot() {
    let db = world().await;
    org(&db).await;
    request_chat_in(&db, "anna", &ask("jonas-bot"), at(10))
        .await
        .unwrap();
    let owner = end_chat_in(&db, "jonas", "jonas-bot", "anna", at(11))
        .await
        .unwrap_err();
    assert_eq!(owner.status(), StatusCode::NOT_FOUND);
    end_chat_in(&db, "anna", "jonas-bot", "anna", at(11))
        .await
        .unwrap();
    assert!(
        bot_team_chat::Entity::find_by_id(("jonas-bot".to_string(), "anna".to_string()))
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_bot_leaving_its_team_ends_chats_and_drops_requests() {
    let db = world().await;
    let kestrel = org(&db).await;
    insert_bot(&db, "pending-bot", "jonas", "SERVER").await;
    own_bot(&db, "jonas", &kestrel, "pending-bot").await;
    let conversation = open_chat(&db).await;
    request_chat_in(&db, "anna", &ask("pending-bot"), at(12))
        .await
        .unwrap();
    super::bots::remove_bot_in(&db, "jonas", &kestrel, "jonas-bot", at(13))
        .await
        .unwrap();
    super::bots::remove_bot_in(&db, "jonas", &kestrel, "pending-bot", at(13))
        .await
        .unwrap();
    assert_eq!(row(&db, "jonas-bot", "anna").await.status, "ENDED");
    assert_eq!(members(&db, &conversation).await, set(&["anna"]));
    assert!(
        bot_team_chat::Entity::find_by_id(("pending-bot".to_string(), "anna".to_string()))
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_person_leaving_the_org_ends_their_chat_and_a_bot_staying_keeps_it_seated() {
    let db = world().await;
    let kestrel = org(&db).await;
    let conversation = open_chat(&db).await;
    let mut effects = Effects::default();
    sync_chats_of(
        &db,
        &["jonas-bot".into()],
        &[],
        "mara",
        at(12),
        &mut effects,
    )
    .await
    .unwrap();
    assert_eq!(
        members(&db, &conversation).await,
        set(&["anna", "jonas-bot", "jonas"])
    );
    remove_person_in(&db, "mara", &kestrel, "anna", at(13))
        .await
        .unwrap();
    assert_eq!(row(&db, "jonas-bot", "anna").await.status, "ENDED");
    assert_eq!(members(&db, &conversation).await, set(&["anna"]));
}

#[tokio::test]
async fn a_thread_opens_with_the_lead_and_its_watching_owner() {
    let db = world().await;
    let kestrel = org(&db).await;
    let none = open_thread_in(&db, "anna", &kestrel, at(10))
        .await
        .unwrap_err();
    assert_code(&none, StatusCode::CONFLICT, TEAM_NO_LEAD);
    assert!(none.public_message().unwrap().contains("Kestrel"));
    lead(&db, &kestrel, "jonas-bot").await;
    let hidden = open_thread_in(&db, "outsider", &kestrel, at(10))
        .await
        .unwrap_err();
    assert_code(&hidden, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);

    classify_bot(&db, "jonas-bot", "restricted").await;
    let above = open_thread_in(&db, "anna", &kestrel, at(10))
        .await
        .unwrap_err();
    assert_code(&above, StatusCode::FORBIDDEN, TEAM_CLASSIFIED);
    assert!(above.public_message().unwrap().contains("restricted"));
    classify_person(&db, &kestrel, "anna", "restricted").await;

    let (thread, _) = open_thread_in(&db, "anna", &kestrel, at(11)).await.unwrap();
    assert_eq!(
        members(&db, &thread.conversation_id).await,
        set(&["anna", "jonas-bot", "jonas"])
    );
    assert!(is_watching(
        member(&db, &thread.conversation_id, "jonas")
            .await
            .limits
            .as_ref()
    ));
    let (again, _) = open_thread_in(&db, "anna", &kestrel, at(12)).await.unwrap();
    assert_eq!(again, thread);

    let (own, _) = open_thread_in(&db, "jonas", &kestrel, at(13))
        .await
        .unwrap();
    assert_eq!(
        members(&db, &own.conversation_id).await,
        set(&["jonas-bot", "jonas"])
    );
    assert!(!is_watching(
        member(&db, &own.conversation_id, "jonas")
            .await
            .limits
            .as_ref()
    ));
}

#[tokio::test]
async fn an_owner_seated_before_watching_existed_becomes_a_watcher_and_keeps_the_mute() {
    let db = world().await;
    let kestrel = org(&db).await;
    lead(&db, &kestrel, "jonas-bot").await;
    let (thread, _) = open_thread_in(&db, "anna", &kestrel, at(10)).await.unwrap();
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Limits,
            Expr::value(None::<serde_json::Value>),
        )
        .filter(conversation_member::Column::ConversationId.eq(&thread.conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq("jonas"))
        .exec(&db)
        .await
        .unwrap();
    open_thread_in(&db, "anna", &kestrel, at(11)).await.unwrap();
    let jonas = member(&db, &thread.conversation_id, "jonas").await;
    assert!(is_watching(jonas.limits.as_ref()));

    set_muted_in(&db, &thread.conversation_id, "jonas", true)
        .await
        .unwrap();
    let jonas = member(&db, &thread.conversation_id, "jonas").await;
    assert!(is_watching(jonas.limits.as_ref()) && is_muted(jonas.limits.as_ref()));
    open_thread_in(&db, "anna", &kestrel, at(12)).await.unwrap();
    let jonas = member(&db, &thread.conversation_id, "jonas").await;
    assert!(is_watching(jonas.limits.as_ref()) && is_muted(jonas.limits.as_ref()));
    set_muted_in(&db, &thread.conversation_id, "jonas", false)
        .await
        .unwrap();
    let jonas = member(&db, &thread.conversation_id, "jonas").await;
    assert!(is_watching(jonas.limits.as_ref()) && !is_muted(jonas.limits.as_ref()));
}

#[tokio::test]
async fn a_watching_owner_is_pushed_only_when_named() {
    use crate::{
        bot_notify::NoticeSink,
        push_notifications::DispatchNotificationInput,
        routes::inbox::{model::InboxEntry, person_push::notify_message},
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl NoticeSink for Recorder {
        async fn send(
            &self,
            _id: &str,
            input: DispatchNotificationInput,
        ) -> flow_like_types::Result<()> {
            self.0.lock().unwrap().push(input.user_id);
            Ok(())
        }
    }

    async fn push(
        db: &DatabaseConnection,
        conversation: &str,
        id: &str,
        mentions: &[&str],
    ) -> Vec<String> {
        let outgoing = crate::routes::inbox::fixture::outgoing(
            id,
            flow_like_types::inbox::InboxEntryKind::Message,
            serde_json::json!({ "text": id, "mentions": mentions }),
        );
        fan_out(db, conversation, "anna", 0, &outgoing, at(20))
            .await
            .unwrap();
        let copy = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq("anna"))
            .filter(inbox_entry::Column::MessageId.eq(id))
            .one(db)
            .await
            .unwrap()
            .unwrap();
        let sink = Recorder::default();
        notify_message(db, &sink, &InboxEntry::from(copy))
            .await
            .unwrap();
        let sent = sink.0.lock().unwrap().clone();
        sent
    }

    let db = world().await;
    org(&db).await;
    let conversation = open_chat(&db).await;
    assert!(push(&db, &conversation, "quiet", &[]).await.is_empty());
    assert_eq!(
        push(&db, &conversation, "named", &["jonas"]).await,
        vec!["jonas"]
    );
}

#[tokio::test]
async fn the_gate_silences_a_bot_above_a_person_once_and_lets_it_answer_otherwise() {
    let db = world().await;
    let kestrel = org(&db).await;
    let conversation = open_chat(&db).await;
    fan_out(&db, &conversation, "anna", 0, &text("m1"), at(20))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 1);

    classify_person(&db, &kestrel, "anna", "public").await;
    fan_out(&db, &conversation, "anna", 0, &text("m2"), at(21))
        .await
        .unwrap();
    fan_out(&db, &conversation, "anna", 0, &text("m3"), at(22))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 1);
    assert_eq!(notices(&db, &conversation, "jonas-bot").await, 1);
    let notice = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::ConversationId.eq(&conversation))
        .filter(inbox_entry::Column::SenderId.eq("jonas-bot"))
        .filter(inbox_entry::Column::InboxId.eq("anna"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let said = notice.envelope["body"]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        said.contains("doesn't answer here") && said.contains("internal"),
        "{said}"
    );

    classify_person(&db, &kestrel, "anna", "confidential").await;
    fan_out(&db, &conversation, "anna", 0, &text("m4"), at(23))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 2);
}

#[tokio::test]
async fn the_gate_ignores_a_bot_whose_owner_is_the_lowest_person_and_ended_chats() {
    let db = world().await;
    let kestrel = org(&db).await;
    let conversation = open_chat(&db).await;
    classify_person(&db, &kestrel, "jonas", "public").await;
    fan_out(&db, &conversation, "anna", 0, &text("m1"), at(20))
        .await
        .unwrap();
    assert_eq!(
        turns(&db).await.len(),
        1,
        "the owner never gates their own bot"
    );

    bot_team_chat::Entity::update_many()
        .col_expr(
            bot_team_chat::Column::Status,
            Expr::value("ENDED".to_string()),
        )
        .exec(&db)
        .await
        .unwrap();
    fan_out(&db, &conversation, "anna", 0, &text("m2"), at(21))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 1, "an ended chat takes no turn");
    let _ = bot_team_bot::Entity::find().all(&db).await.unwrap();
}

#[tokio::test]
async fn the_gate_covers_a_lead_thread_with_the_leads_owner_exempt() {
    let db = world().await;
    let kestrel = org(&db).await;
    lead(&db, &kestrel, "jonas-bot").await;
    let (thread, _) = open_thread_in(&db, "anna", &kestrel, at(10)).await.unwrap();
    fan_out(&db, &thread.conversation_id, "anna", 0, &text("t1"), at(20))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 1);
    classify_bot(&db, "jonas-bot", "restricted").await;
    fan_out(&db, &thread.conversation_id, "anna", 0, &text("t2"), at(21))
        .await
        .unwrap();
    assert_eq!(turns(&db).await.len(), 1);
    assert_eq!(notices(&db, &thread.conversation_id, "jonas-bot").await, 1);
}
