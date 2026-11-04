use super::*;
use crate::codes;

fn pinned_ids(f: &Fixture, conversation_id: &str) -> Vec<String> {
    f.store
        .pins(conversation_id)
        .unwrap()
        .into_iter()
        .map(|pin| pin.message_id)
        .collect()
}

fn star_ids(f: &Fixture, conversation_id: Option<&str>) -> Vec<String> {
    f.store
        .stars(conversation_id, None, None)
        .unwrap()
        .stars
        .into_iter()
        .map(|star| star.message_id)
        .collect()
}

fn count(f: &Fixture, table: &str) -> i64 {
    f.raw()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn pinned_conversations_list_first_newest_pin_first() {
    let f = fixture();
    let (a, b, c) = (f.bot("A"), f.bot("B"), f.bot("C"));
    let (first, second, third) = (f.direct(&a), f.direct(&b), f.direct(&c));
    let listed = |f: &Fixture| -> Vec<String> {
        f.store
            .conversations()
            .unwrap()
            .into_iter()
            .map(|conversation| conversation.id)
            .collect()
    };
    assert_eq!(
        listed(&f),
        [third.id.as_str(), second.id.as_str(), first.id.as_str()]
    );

    f.store.set_conversation_pinned(&first.id, true).unwrap();
    later();
    let state = f.store.set_conversation_pinned(&second.id, true).unwrap();
    assert!(state.pinned_at.is_some());
    f.text(&third.id, "local", "fresh");
    assert_eq!(
        listed(&f),
        [second.id.as_str(), first.id.as_str(), third.id.as_str()]
    );
    assert!(f.visible(&first.id).pinned_at.is_some());
    assert!(f.visible(&third.id).pinned_at.is_none());

    let kept = f.visible(&first.id).pinned_at;
    f.store.set_conversation_pinned(&first.id, true).unwrap();
    assert_eq!(
        f.visible(&first.id).pinned_at,
        kept,
        "pinning again keeps the pin time"
    );

    f.changes.take();
    let state = f.store.set_conversation_pinned(&first.id, false).unwrap();
    assert_eq!(
        (state.conversation_id.as_str(), state.pinned_at),
        (first.id.as_str(), None)
    );
    assert_eq!(f.changes.take(), [Change::Inbox(Some(first.id.clone()))]);
    assert_eq!(
        listed(&f),
        [second.id.as_str(), third.id.as_str(), first.id.as_str()]
    );
    f.store.set_conversation_pinned(&first.id, false).unwrap();
    assert!(
        f.changes.take().is_empty(),
        "unpinning twice changes nothing"
    );

    assert_eq!(
        code(f.store.set_conversation_pinned("missing", true)),
        codes::NOT_FOUND
    );
}

#[test]
fn a_pinned_conversation_never_falls_off_the_list() {
    let f = fixture();
    let old = f.bot("Old");
    let oldest = f.direct(&old);
    f.store.set_conversation_pinned(&oldest.id, true).unwrap();
    later();
    for index in 0..crate::inbox::MAX_LISTED_CONVERSATIONS + 5 {
        f.store
            .create_conversation(conversation(
                json!({ "kind": "group", "title": format!("Room {index}"), "members": [old.id] }),
            ))
            .unwrap();
    }
    let listed = f.store.conversations().unwrap();
    assert_eq!(listed[0].id, oldest.id);
}

#[test]
fn message_pins_are_idempotent_shared_and_capped() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    for index in 0..51 {
        f.text(&direct.id, "local", &format!("m{index}"));
    }
    f.changes.take();
    let pin = f.store.pin_message(&direct.id, "m0").unwrap();
    assert_eq!(pin.pinned_by, "local");
    assert_eq!(pin.entry.as_ref().unwrap().message_id, "m0");
    assert!(pin.latest_edit.is_none());
    assert_eq!(f.changes.take(), [Change::Inbox(Some(direct.id.clone()))]);
    let again = f.store.pin_message(&direct.id, "m0").unwrap();
    assert_eq!(again.pinned_at, pin.pinned_at);
    assert!(f.changes.take().is_empty());

    later();
    for index in 1..50 {
        f.store
            .pin_message(&direct.id, &format!("m{index}"))
            .unwrap();
    }
    assert_eq!(pinned_ids(&f, &direct.id).len(), 50);
    assert_eq!(pinned_ids(&f, &direct.id)[0], "m49", "newest pin first");
    assert_eq!(
        code(f.store.pin_message(&direct.id, "m50")),
        codes::INBOX_PIN_LIMIT
    );
    f.store.pin_message(&direct.id, "m3").unwrap();

    f.store.unpin_message(&direct.id, "m0").unwrap();
    f.store.unpin_message(&direct.id, "m0").unwrap();
    f.store.pin_message(&direct.id, "m50").unwrap();
    assert_eq!(pinned_ids(&f, &direct.id).len(), 50);

    assert_eq!(
        code(f.store.pin_message(&direct.id, "nope")),
        codes::INBOX_TARGET_NOT_FOUND
    );
    assert_eq!(code(f.store.pins("missing")), codes::NOT_FOUND);
}

#[test]
fn only_a_live_message_is_pinned_or_starred() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    f.text(&direct.id, "local", "m1");
    f.send(
        &direct.id,
        "local",
        "r1",
        "reaction",
        json!({ "target": "m1", "emoji": "x" }),
    )
    .ok();
    assert_eq!(
        code(f.store.star_message(&direct.id, "r1")),
        codes::INBOX_TARGET_NOT_FOUND
    );
    f.send(
        &direct.id,
        "local",
        "d1",
        "delete",
        json!({ "target": "m1" }),
    )
    .unwrap();
    assert_eq!(
        code(f.store.pin_message(&direct.id, "m1")),
        codes::INBOX_TARGET_NOT_FOUND
    );
    assert_eq!(
        code(f.store.star_message(&direct.id, "m1")),
        codes::INBOX_TARGET_NOT_FOUND
    );
}

#[test]
fn deleting_a_message_drops_its_pin_and_star() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    f.text(&direct.id, "local", "m1");
    f.text(&direct.id, "local", "m2");
    for id in ["m1", "m2"] {
        f.store.pin_message(&direct.id, id).unwrap();
        f.store.star_message(&direct.id, id).unwrap();
    }
    f.send(
        &direct.id,
        "local",
        "d1",
        "delete",
        json!({ "target": "m1" }),
    )
    .unwrap();
    assert_eq!(pinned_ids(&f, &direct.id), ["m2"]);
    assert_eq!(star_ids(&f, None), ["m2"]);
    assert_eq!(count(&f, "pinned_messages"), 1);
    assert_eq!(count(&f, "starred_messages"), 1);
}

#[test]
fn forgetting_a_conversation_removes_its_pins_and_stars() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    f.text(&direct.id, "local", "m1");
    f.store.pin_message(&direct.id, "m1").unwrap();
    f.store.star_message(&direct.id, "m1").unwrap();
    f.store.set_conversation_pinned(&direct.id, true).unwrap();
    f.store.forget_conversation(&direct.id, None).unwrap();
    for table in [
        "pinned_messages",
        "starred_messages",
        "pinned_conversations",
    ] {
        assert_eq!(count(&f, table), 0, "{table}");
    }
}

#[test]
fn a_pin_carries_the_newest_edit() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    f.text(&direct.id, "local", "m1");
    later();
    f.send(
        &direct.id,
        "local",
        "e1",
        "edit",
        json!({ "target": "m1", "text": "one" }),
    )
    .unwrap();
    later();
    f.send(
        &direct.id,
        "local",
        "e2",
        "edit",
        json!({ "target": "m1", "text": "two" }),
    )
    .unwrap();
    let pin = f.store.pin_message(&direct.id, "m1").unwrap();
    assert_eq!(pin.latest_edit.unwrap().message_id, "e2");
    let star = f.store.star_message(&direct.id, "m1").unwrap();
    assert_eq!(star.latest_edit.unwrap().message_id, "e2");
    assert_eq!(
        f.store.pins(&direct.id).unwrap()[0]
            .latest_edit
            .as_ref()
            .unwrap()
            .message_id,
        "e2"
    );
}

#[test]
fn stars_page_newest_first_across_and_within_conversations() {
    let f = fixture();
    let (a, b) = (f.bot("A"), f.bot("B"));
    let (one, two) = (f.direct(&a), f.direct(&b));
    for index in 0..3 {
        f.text(&one.id, "local", &format!("a{index}"));
        f.text(&two.id, "local", &format!("b{index}"));
    }
    let order = ["a0", "b0", "a1", "b1", "a2"];
    for id in order {
        let conversation = if id.starts_with('a') { &one } else { &two };
        f.store.star_message(&conversation.id, id).unwrap();
        later();
    }
    f.changes.take();
    let again = f.store.star_message(&one.id, "a0").unwrap();
    assert_eq!(again.conversation_id, one.id);
    assert!(
        f.changes.take().is_empty(),
        "starring again changes nothing"
    );

    let expected: Vec<&str> = order.iter().rev().copied().collect();
    assert_eq!(star_ids(&f, None), expected);
    assert_eq!(star_ids(&f, Some(&two.id)), ["b1", "b0"]);

    let first = f.store.stars(None, None, Some(2)).unwrap();
    let ids = |page: &crate::StarredMessagesPage| -> Vec<String> {
        page.stars
            .iter()
            .map(|star| star.message_id.clone())
            .collect()
    };
    assert_eq!(ids(&first), ["a2", "b1"]);
    assert_eq!(
        first.next_before.as_deref(),
        Some(first.stars[1].cursor.as_str())
    );
    let second = f
        .store
        .stars(None, first.next_before.as_deref(), Some(2))
        .unwrap();
    assert_eq!(ids(&second), ["a1", "b0"]);
    let last = f
        .store
        .stars(None, second.next_before.as_deref(), Some(2))
        .unwrap();
    assert_eq!(
        (ids(&last), last.next_before),
        (vec!["a0".to_string()], None)
    );

    assert_eq!(
        code(f.store.stars(None, Some("garbage"), None)),
        codes::INVALID
    );
    assert_eq!(
        code(f.store.stars(Some("missing"), None, None)),
        codes::NOT_FOUND
    );

    f.store.unstar_message(&one.id, "a2").unwrap();
    f.store.unstar_message(&one.id, "a2").unwrap();
    assert_eq!(star_ids(&f, None).len(), 4);
}
