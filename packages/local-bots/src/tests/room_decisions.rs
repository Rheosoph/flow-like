use super::*;
use crate::RoomWaitOutcome;

#[test]
fn participant_wait_survives_reopening_and_ignores_other_speakers() {
    let f = fixture();
    let (ada, bee, cy) = (f.bot("Ada"), f.bot("Bee"), f.bot("Cy"));
    let room = f.group(&[&ada, &bee, &cy]);
    let trigger = f.text(&room.id, "local", "question");
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, &trigger.message_id, Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::Parked,
    );
    f.text(&room.id, &cy.id, "unrelated");
    let reopened = f.open_again(TestKey::default()).unwrap();
    let pending = reopened.room_decision_state(&ada.id, &room.id).unwrap();
    assert_eq!(pending.participant_id.as_deref(), Some(bee.id.as_str()));
    assert!(!pending.ready);
    f.text(&room.id, &bee.id, "answer");
    assert!(
        reopened
            .room_decision_state(&ada.id, &room.id)
            .unwrap()
            .ready
    );
}

#[test]
fn no_answer_checkpoint_survives_reopening_and_clearing_a_wait() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    let trigger = f.text(&room.id, "local", "question");
    f.store
        .settle_room_decision(&ada.id, &room.id, &trigger.message_id, None)
        .unwrap();
    let reopened = f.open_again(TestKey::default()).unwrap();
    reopened.clear_room_wait(&ada.id, &room.id).unwrap();
    let state = reopened.room_decision_state(&ada.id, &room.id).unwrap();
    assert_eq!(state.handled_through, trigger.seq.unwrap());
    assert_eq!(state.participant_id, None);
}

#[test]
fn a_reply_arriving_during_classification_does_not_create_a_wait() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    f.text(&room.id, "local", "question");
    f.text(&room.id, &bee.id, "already-answered");
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, "question", Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::AlreadyAnswered,
    );
    assert_eq!(
        f.store
            .room_decision_state(&ada.id, &room.id)
            .unwrap()
            .participant_id,
        None
    );
}

#[test]
fn waits_reject_cycles_self_and_nonmembers_and_release_on_departure() {
    let f = fixture();
    let (ada, bee, outside) = (f.bot("Ada"), f.bot("Bee"), f.bot("Outside"));
    let room = f.group(&[&ada, &bee]);
    f.text(&room.id, "local", "question");
    for target in [&ada.id, &outside.id] {
        assert_eq!(
            f.store
                .settle_room_decision(&ada.id, &room.id, "question", Some(target))
                .unwrap(),
            RoomWaitOutcome::Unavailable
        );
    }
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, "question", Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::Parked
    );
    assert_eq!(
        f.store
            .settle_room_decision(&bee.id, &room.id, "question", Some(&ada.id))
            .unwrap(),
        RoomWaitOutcome::Unavailable
    );
    f.store.remove_member(&room.id, &bee.id).unwrap();
    assert!(
        f.store
            .room_decision_state(&ada.id, &room.id)
            .unwrap()
            .ready
    );
}

#[test]
fn expired_waits_release_without_losing_the_processed_position() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    let trigger = f.text(&room.id, "local", "question");
    f.store
        .settle_room_decision(&ada.id, &room.id, "question", Some(&bee.id))
        .unwrap();
    f.raw()
        .execute("UPDATE room_decisions SET expires_at = 0", [])
        .unwrap();
    let state = f.store.room_decision_state(&ada.id, &room.id).unwrap();
    assert!(state.ready);
    assert_eq!(state.handled_through, trigger.seq.unwrap());
}

#[test]
fn direct_call_silence_is_durable_but_direct_participant_waits_are_not_created() {
    let f = fixture();
    let ada = f.bot("Ada");
    let direct = f.direct(&ada);
    let utterance = f
        .send(
            &direct.id,
            "local",
            "utterance",
            "transcript",
            json!({
                "call": "call-1", "text": "I am thinking", "offset_ms": 0, "duration_ms": 1500,
            }),
        )
        .unwrap();
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &direct.id, &utterance.message_id, Some("local"))
            .unwrap(),
        RoomWaitOutcome::Unavailable
    );
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &direct.id, &utterance.message_id, None)
            .unwrap(),
        RoomWaitOutcome::Parked
    );
    let reopened = f.open_again(TestKey::default()).unwrap();
    assert_eq!(
        reopened
            .room_decision_state(&ada.id, &direct.id)
            .unwrap()
            .handled_through,
        utterance.seq.unwrap()
    );
}

#[test]
fn a_new_topic_clears_the_wait_and_checkpoints_the_previous_conversation() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    f.text(&room.id, "local", "old-question");
    f.store
        .settle_room_decision(&ada.id, &room.id, "old-question", Some(&bee.id))
        .unwrap();
    let old_answer = f.text(&room.id, &bee.id, "old-answer");
    let topic = f
        .send(
            &room.id,
            "local",
            "new-topic",
            "topic",
            json!({ "title": "New work" }),
        )
        .unwrap();
    let state = f.store.room_decision_state(&ada.id, &room.id).unwrap();
    assert_eq!(state.participant_id, None);
    assert_eq!(state.handled_through, topic.seq.unwrap());
    assert!(old_answer.seq.unwrap() <= state.handled_through);
    let stored: (i64, Option<String>, Option<i64>) = f.raw().query_row(
        "SELECT handled_through, participant_id, expires_at FROM room_decisions WHERE bot_id = ?1 AND conversation_id = ?2",
        rusqlite::params![ada.id, room.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(stored, (topic.seq.unwrap(), None, None));
    let next = f.text(&room.id, "local", "new-question");
    let reopened = f.open_again(TestKey::default()).unwrap();
    let state = reopened.room_decision_state(&ada.id, &room.id).unwrap();
    assert!(state.ready);
    assert!(next.seq.unwrap() > state.handled_through);
    assert_eq!(state.participant_id, None);
}

#[test]
fn a_topic_arriving_during_classification_supersedes_the_old_decision() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    f.text(&room.id, "local", "old-question");
    let topic = f
        .send(&room.id, "local", "new-topic", "topic", json!({}))
        .unwrap();
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, "old-question", Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::Superseded,
    );
    let state = f.store.room_decision_state(&ada.id, &room.id).unwrap();
    assert_eq!(state.handled_through, topic.seq.unwrap());
    assert_eq!(state.participant_id, None);
    assert!(state.ready);

    f.text(&room.id, "local", "new-question");
    f.store
        .settle_room_decision(&ada.id, &room.id, "new-question", Some(&bee.id))
        .unwrap();
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, "old-question", Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::Superseded,
    );
    assert_eq!(
        f.store
            .room_decision_state(&ada.id, &room.id)
            .unwrap()
            .participant_id
            .as_deref(),
        Some(bee.id.as_str())
    );
}

#[test]
fn an_older_decision_does_not_overwrite_a_newer_one() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let room = f.group(&[&ada, &bee]);
    let older = f.text(&room.id, "local", "first");
    let newer = f.text(&room.id, "local", "second");
    f.store
        .settle_room_decision(&ada.id, &room.id, &newer.message_id, None)
        .unwrap();
    assert_eq!(
        f.store
            .settle_room_decision(&ada.id, &room.id, &older.message_id, Some(&bee.id))
            .unwrap(),
        RoomWaitOutcome::Superseded,
    );
    let state = f.store.room_decision_state(&ada.id, &room.id).unwrap();
    assert_eq!(state.handled_through, newer.seq.unwrap());
    assert_eq!(state.participant_id, None);
}
