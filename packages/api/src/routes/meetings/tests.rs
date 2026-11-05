use super::{model::*, store::*};
use crate::{
    entity::{meeting, meeting_message, meeting_participant},
    routes::inbox::fixture,
};
use chrono::Duration;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Schema, sea_query::Table};

async fn database() -> DatabaseConnection {
    let db = fixture::database().await;
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(meeting::Entity),
        schema.create_table_from_entity(meeting_participant::Entity),
        schema.create_table_from_entity(meeting_message::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    db.execute_unprepared(r#"INSERT INTO public."Conversation" ("id","kind","encryption","createdBy","lastActivityAt","createdAt","updatedAt")
        VALUES ('room','GROUP','PLAIN','host','2023-11-14T22:13:20+00:00','2023-11-14T22:13:20+00:00','2023-11-14T22:13:20+00:00')"#).await.unwrap();
    db.execute_unprepared(r#"INSERT INTO public."ConversationMember" ("conversationId","principalId","principalKind","joinedAt","status")
        VALUES ('room','host','USER','2023-11-14T22:13:20+00:00','ACTIVE'), ('room','member','USER','2023-11-14T22:13:20+00:00','ACTIVE')"#).await.unwrap();
    db
}

fn request() -> CreateMeeting {
    CreateMeeting {
        conversation_id: "room".into(),
        title: "Review".into(),
        starts_at: fixture::at(0),
        ends_at: fixture::at(3600),
        guests_allowed: true,
    }
}

async fn meeting(db: &DatabaseConnection) -> meeting::Model {
    create_in(db, "host", &request(), fixture::at(0))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_direct_chat_block_stops_meeting_members_and_guest_sessions() {
    let db = database().await;
    db.execute_unprepared(
        r#"UPDATE public."Conversation" SET "kind" = 'DIRECT' WHERE "id" = 'room'"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name") VALUES ('host', 'Host'), ('member', 'Member')"#,
    )
    .await
    .unwrap();
    let row = meeting(&db).await;
    let guest = join_in(&db, &row, None, "Guest", fixture::at(0))
        .await
        .unwrap();
    crate::routes::inbox::blocks::block_in(&db, "member", "host", fixture::at(1))
        .await
        .unwrap();
    assert!(member(&db, &row, "host").await.is_err());
    assert!(
        create_in(&db, "host", &request(), fixture::at(2))
            .await
            .is_err()
    );
    assert!(
        join_in(&db, &row, None, "Another guest", fixture::at(2))
            .await
            .is_err()
    );
    assert!(session_in(&db, &row, &guest.session_token).await.is_err());
}

#[tokio::test]
async fn waiting_guests_cannot_read_chat_publish_or_join_media_and_never_join_parent_room() {
    let db = database().await;
    let row = meeting(&db).await;
    let host = join_in(&db, &row, Some("host"), "Host", fixture::at(0))
        .await
        .unwrap();
    let host = session_in(&db, &row, &host.session_token).await.unwrap();
    send_in(&db, &row, &host, "Members are here", fixture::at(1))
        .await
        .unwrap();
    let guest = join_in(&db, &row, None, "Guest", fixture::at(2))
        .await
        .unwrap();
    let guest = session_in(&db, &row, &guest.session_token).await.unwrap();
    assert_eq!(guest.status, WAITING);
    assert!(admitted(&row, &guest, fixture::at(2)).is_err());
    assert!(
        send_in(&db, &row, &guest, "intrusion", fixture::at(2))
            .await
            .is_err()
    );
    assert!(member(&db, &row, &guest.id).await.is_err());
    let waiting_view = snapshot(&db, &row, &guest, fixture::at(2)).await.unwrap();
    assert!(waiting_view.messages.is_empty());
    assert_eq!(waiting_view.participants.len(), 1);
    change_participant_in(&db, &row, &guest.id, true, fixture::at(3))
        .await
        .unwrap();
    let guest = meeting_participant::Entity::find_by_id(&guest.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(admitted(&row, &guest, fixture::at(3)).is_ok());
    assert_eq!(
        snapshot(&db, &row, &guest, fixture::at(3))
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(member(&db, &row, &guest.id).await.is_err());
}

#[tokio::test]
async fn account_members_and_guest_sessions_cannot_take_host_actions() {
    let db = database().await;
    let row = meeting(&db).await;
    assert!(host_in(&db, &row, Some("host"), None).await.is_ok());
    assert!(host_in(&db, &row, Some("member"), None).await.is_err());
    assert!(host_in(&db, &row, Some("outsider"), None).await.is_err());
    let guest = join_in(&db, &row, None, "Host", fixture::at(0))
        .await
        .unwrap();
    change_participant_in(&db, &row, &guest.participant_id, true, fixture::at(1))
        .await
        .unwrap();
    assert!(
        host_in(&db, &row, None, Some(&guest.session_token))
            .await
            .is_err()
    );
    assert!(
        create_in(&db, "outsider", &request(), fixture::at(0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn session_credentials_are_hashed_scoped_and_rotated_on_rejoin() {
    let db = database().await;
    let row = meeting(&db).await;
    let first = join_in(&db, &row, Some("host"), "Host", fixture::at(0))
        .await
        .unwrap();
    let stored = session_in(&db, &row, &first.session_token).await.unwrap();
    assert!(!stored.session_hash.contains(&first.session_token));
    assert!(
        session_in(
            &db,
            &row,
            &format!("{}.{}", first.participant_id, "0".repeat(64))
        )
        .await
        .is_err()
    );
    let other = meeting(&db).await;
    assert!(session_in(&db, &other, &first.session_token).await.is_err());
    let second = join_in(&db, &row, Some("host"), "Host", fixture::at(1))
        .await
        .unwrap();
    assert_eq!(first.participant_id, second.participant_id);
    assert!(session_in(&db, &row, &first.session_token).await.is_err());
    assert!(session_in(&db, &row, &second.session_token).await.is_ok());
    assert_eq!(find(&db, &row.id).await.unwrap().room_epoch, 1);
}

#[tokio::test]
async fn invite_rotation_and_guest_disable_revoke_links() {
    let db = database().await;
    let row = meeting(&db).await;
    let key = [7; 32];
    let token = invite_token(&key, &row);
    assert!(valid_invite(&key, &row, &token));
    assert!(!valid_invite(&[8; 32], &row, &token));
    let mut changed = row.clone();
    changed.invite_nonce = random_secret().unwrap();
    assert!(!valid_invite(&key, &changed, &token));
    changed = row.clone();
    changed.guests_allowed = false;
    assert!(!valid_invite(&key, &changed, &token));
    assert!(
        join_in(&db, &changed, None, "Guest", fixture::at(0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn schedule_enforces_early_window_deadline_and_host_end() {
    let db = database().await;
    let row = meeting(&db).await;
    assert!(open(&row, fixture::at(-901)).is_err());
    assert!(open(&row, fixture::at(-900)).is_ok());
    assert!(open(&row, fixture::at(3600)).is_err());
    let joined = join_in(&db, &row, Some("host"), "Host", fixture::at(0))
        .await
        .unwrap();
    let participant = session_in(&db, &row, &joined.session_token).await.unwrap();
    end_in(&db, &row, fixture::at(1)).await.unwrap();
    let ended = find(&db, &row.id).await.unwrap();
    assert!(admitted(&ended, &participant, fixture::at(2)).is_err());
    assert!(
        send_in(&db, &ended, &participant, "late", fixture::at(2))
            .await
            .is_err()
    );
    let snapshot = snapshot(&db, &ended, &participant, fixture::at(2))
        .await
        .unwrap();
    assert!(snapshot.meeting.ended_at.is_some());
    assert!(snapshot.messages.is_empty());
    let mut invalid = request();
    invalid.ends_at = invalid.starts_at;
    assert!(validate_create(&invalid, fixture::at(0)).is_err());
    invalid.ends_at = invalid.starts_at + Duration::hours(25);
    assert!(validate_create(&invalid, fixture::at(0)).is_err());
}

#[tokio::test]
async fn admission_reserves_host_slot_and_serialized_recheck_rejects_ninth_participant() {
    let db = database().await;
    let row = meeting(&db).await;
    for n in 0..7 {
        let guest = join_in(&db, &row, None, &format!("Guest {n}"), fixture::at(0))
            .await
            .unwrap();
        change_participant_in(&db, &row, &guest.participant_id, true, fixture::at(0))
            .await
            .unwrap();
    }
    let eighth = join_in(&db, &row, None, "Waiting", fixture::at(0))
        .await
        .unwrap();
    assert!(
        change_participant_in(&db, &row, &eighth.participant_id, true, fixture::at(0))
            .await
            .is_err()
    );
    join_in(&db, &row, Some("host"), "Host", fixture::at(0))
        .await
        .unwrap();
    assert!(
        join_in(&db, &row, Some("member"), "Member", fixture::at(0))
            .await
            .is_err()
    );
    assert!(
        change_participant_in(&db, &row, &eighth.participant_id, true, fixture::at(0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn removed_or_expired_participants_lose_media_and_chat_and_capacity_is_recovered() {
    let db = database().await;
    let row = meeting(&db).await;
    let joined = join_in(&db, &row, Some("member"), "Member", fixture::at(0))
        .await
        .unwrap();
    change_participant_in(&db, &row, &joined.participant_id, false, fixture::at(1))
        .await
        .unwrap();
    let removed = session_in(&db, &row, &joined.session_token).await.unwrap();
    assert_eq!(removed.status, REMOVED);
    assert!(admitted(&row, &removed, fixture::at(1)).is_err());
    assert!(
        join_in(&db, &row, Some("member"), "Member", fixture::at(2))
            .await
            .is_err()
    );
    let joined = join_in(&db, &row, Some("host"), "Host", fixture::at(0))
        .await
        .unwrap();
    let participant = session_in(&db, &row, &joined.session_token).await.unwrap();
    assert!(admitted(&row, &participant, fixture::at(91)).is_err());
    reap_in(&db, &row, fixture::at(91)).await.unwrap();
    let expired = session_in(&db, &row, &joined.session_token).await.unwrap();
    assert_eq!(expired.status, LEFT);
    let renewed = heartbeat_in(&db, &row, expired, fixture::at(92))
        .await
        .unwrap();
    assert_eq!(renewed.status, LEFT);
    join_in(&db, &row, Some("host"), "Host", fixture::at(92))
        .await
        .unwrap();
}

#[tokio::test]
async fn losing_room_membership_revokes_account_meeting_session() {
    let db = database().await;
    let row = meeting(&db).await;
    let joined = join_in(&db, &row, Some("member"), "Member", fixture::at(0))
        .await
        .unwrap();
    db.execute_unprepared(r#"UPDATE public."ConversationMember" SET "status"='DECLINED' WHERE "principalId"='member'"#).await.unwrap();
    assert!(session_in(&db, &row, &joined.session_token).await.is_err());
    assert!(host_in(&db, &row, Some("member"), None).await.is_err());
}
