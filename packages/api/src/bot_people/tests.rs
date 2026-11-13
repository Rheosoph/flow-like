use super::*;
use crate::{
    entity::{conversation, conversation_member},
    routes::inbox::{
        conversations::{start_direct, start_direct_as},
        exchange::post_as_bot_in,
        fixture::{at, bot_principal, copies, database, insert_bot, user},
        outreach::{OUTREACH_LIMITS, outreach_key, start_outreach_in},
        person_push::set_muted_in,
    },
};
use flow_like_types::inbox::ConversationEncryption;
use sea_orm::{DatabaseConnection, PaginatorTrait};
use std::sync::Mutex;

const OWNER: &str = "user-a";
const ANNA: &str = "user-anna";
const BEN: &str = "user-ben";
const DORA: &str = "user-dora";
const BOT: &str = "bot_1";

struct DbRooms(DatabaseConnection);

#[async_trait]
impl Rooms for DbRooms {
    async fn open(
        &self,
        owner_id: &str,
        bot_id: &str,
        person_id: &str,
        title: String,
    ) -> Result<String> {
        start_outreach_in(&self.0, owner_id, bot_id, person_id, title, at(100))
            .await
            .map_err(|error| anyhow!("{error}"))
    }

    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let outgoing = message_outgoing(message_id, serde_json::json!({ "text": text, "hops": 0 }))
            .map_err(|error| anyhow!("{error}"))?;
        post_as_bot_in(&self.0, conversation_id, bot_id, 0, &outgoing, at(101))
            .await
            .map(|_| ())
            .map_err(|error| anyhow!("{error}"))
    }
}

#[derive(Default)]
struct Pushes(Mutex<Vec<(String, DispatchNotificationInput)>>);

#[async_trait]
impl NoticeSink for Pushes {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()> {
        self.0.lock().unwrap().push((id.to_string(), input));
        Ok(())
    }
}

async fn setup() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."User" ("id", "name", "preferredUsername") VALUES
        ('{OWNER}', 'Olga', NULL), ('{ANNA}', 'Anna', NULL), ('{BEN}', 'Ben', NULL),
        ('{DORA}', NULL, 'dora')"#
    ))
    .await
    .unwrap();
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    start_direct(
        &db,
        OWNER,
        &user(ANNA),
        ConversationEncryption::Plain,
        at(10),
    )
    .await
    .unwrap();
    start_direct(
        &db,
        DORA,
        &user(OWNER),
        ConversationEncryption::Plain,
        at(20),
    )
    .await
    .unwrap();
    start_direct_as(
        &db,
        OWNER,
        &user(BEN),
        ConversationEncryption::Plain,
        true,
        at(30),
    )
    .await
    .unwrap();
    start_direct(
        &db,
        OWNER,
        &bot_principal(BOT),
        ConversationEncryption::Plain,
        at(40),
    )
    .await
    .unwrap();
    db
}

async fn send(db: &DatabaseConnection, pushes: &Pushes, person: &str, text: &str) -> Delivery {
    deliver_person_message(db, &DbRooms(db.clone()), pushes, BOT, person, text)
        .await
        .unwrap()
}

fn sent(delivery: Delivery) -> PersonMessage {
    match delivery {
        Delivery::Sent(message) => message,
        other => panic!("expected a sent message, got {other:?}"),
    }
}

async fn room_members(db: &DatabaseConnection, room: &str) -> Vec<conversation_member::Model> {
    conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(room))
        .all(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn contacts_are_accepted_direct_chats_with_people_newest_first() {
    let db = setup().await;
    let people = contacts(&db, OWNER, None).await.unwrap();
    assert_eq!(
        people,
        [
            Contact {
                id: DORA.into(),
                name: "dora".into()
            },
            Contact {
                id: ANNA.into(),
                name: "Anna".into()
            },
        ]
    );
    let found = contacts(&db, OWNER, Some(" ANN ")).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, ANNA);
    assert!(contacts(&db, ANNA, None).await.unwrap()[0].id == OWNER);
}

#[tokio::test]
async fn a_pending_request_a_bot_or_a_stranger_is_no_contact_and_gets_no_room() {
    let db = setup().await;
    let pushes = Pushes::default();
    for person in [BEN, BOT, "user-zoe", OWNER] {
        assert_eq!(
            send(&db, &pushes, person, "hi").await,
            Delivery::NotAContact,
            "{person}"
        );
    }
    let rooms = conversation::Entity::find()
        .filter(conversation::Column::DirectKey.starts_with("outreach:"))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(rooms, 0);
    assert!(pushes.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_first_message_opens_one_room_without_apps_and_pushes_the_person() {
    let db = setup().await;
    let pushes = Pushes::default();
    let first = sent(send(&db, &pushes, ANNA, "Felix asks whether Friday works.").await);
    assert_eq!(first.name, "Anna");
    assert!(first.notified);
    assert!(first.message_id.starts_with(PERSON_MESSAGE_PREFIX));
    let room = conversation::Entity::find_by_id(first.conversation_id.as_str())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(room.kind, "GROUP");
    assert_eq!(room.title.as_deref(), Some("bot_1 ↔ Anna"));
    assert_eq!(
        room.direct_key.as_deref(),
        Some(outreach_key(BOT, ANNA).as_str())
    );
    let members = room_members(&db, &room.id).await;
    assert_eq!(members.len(), 3);
    for member in &members {
        assert_eq!(member.status, "ACTIVE", "{}", member.principal_id);
        let expected = (member.principal_id == BOT)
            .then(|| serde_json::from_str::<serde_json::Value>(OUTREACH_LIMITS).unwrap());
        assert_eq!(member.limits, expected, "{}", member.principal_id);
    }
    let mut inboxes: Vec<String> = copies(&db, &first.message_id)
        .await
        .into_iter()
        .map(|copy| copy.inbox_id)
        .collect();
    inboxes.sort();
    assert_eq!(inboxes, [BOT, OWNER, ANNA]);
    let pushed = pushes.0.lock().unwrap().clone();
    assert_eq!(pushed.len(), 1);
    let (_, push) = &pushed[0];
    assert_eq!(push.user_id, ANNA);
    assert_eq!(push.title, "bot_1 (Olga's bot)");
    assert_eq!(
        push.description.as_deref(),
        Some("Felix asks whether Friday works.")
    );
    assert_eq!(push.link, Some(chat_link(&room.id)));

    let second = sent(send(&db, &pushes, ANNA, "Thanks!").await);
    assert_eq!(second.conversation_id, first.conversation_id);
    assert_eq!(count_person_messages(&db, BOT, 0).await.unwrap(), 2);
    assert_eq!(
        count_person_messages(&db, BOT, at(102).timestamp_millis())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_person_who_left_the_room_is_not_written_to_again() {
    let db = setup().await;
    let pushes = Pushes::default();
    let first = sent(send(&db, &pushes, ANNA, "hi").await);
    db.execute_unprepared(&format!(
        r#"UPDATE public."ConversationMember" SET "leftAt" = '2023-11-14 22:15:00+00:00'
        WHERE "conversationId" = '{}' AND "principalId" = '{ANNA}'"#,
        first.conversation_id
    ))
    .await
    .unwrap();
    assert_eq!(
        send(&db, &pushes, ANNA, "are you there?").await,
        Delivery::Left {
            name: "Anna".into()
        }
    );
    assert_eq!(count_person_messages(&db, BOT, 0).await.unwrap(), 1);
    assert_eq!(pushes.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_muted_room_gets_the_message_without_a_push() {
    let db = setup().await;
    let pushes = Pushes::default();
    let first = sent(send(&db, &pushes, ANNA, "hi").await);
    set_muted_in(&db, &first.conversation_id, ANNA, true)
        .await
        .unwrap();
    let second = sent(send(&db, &pushes, ANNA, "one more thing").await);
    assert!(!second.notified);
    assert_eq!(copies(&db, &second.message_id).await.len(), 3);
    assert_eq!(pushes.0.lock().unwrap().len(), 1);
}
