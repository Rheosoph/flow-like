use super::*;
use crate::codes;
use flow_like_types::inbox::{
    ActivityBody, ConversationEncryption, ConversationKind, InboxEntryKind, PrincipalKind,
    direct_pair_key,
};

fn member_ids(conversation: &Conversation) -> Vec<String> {
    let mut ids: Vec<String> = conversation
        .members
        .iter()
        .map(|member| member.principal_id.clone())
        .collect();
    ids.sort();
    ids
}

fn ids(of: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = of.iter().map(|id| id.to_string()).collect();
    ids.sort();
    ids
}

fn unread(f: &Fixture, conversation_id: &str) -> u64 {
    f.visible(conversation_id).unread_count
}

#[test]
fn a_direct_conversation_is_one_per_bot_and_has_no_join_events() {
    let f = fixture();
    let bot = f.bot("Scout");
    let first = f.direct(&bot);
    assert_eq!(first.kind, ConversationKind::Direct);
    assert_eq!(first.encryption, ConversationEncryption::Plain);
    assert_eq!(
        (first.created_by.as_str(), first.title.as_deref()),
        ("local", None)
    );
    assert_eq!((first.unread_count, first.last_entry.as_ref()), (0, None));
    assert_eq!(first.last_activity_at, first.created_at);
    assert!(first.local);
    let person = first
        .members
        .iter()
        .find(|member| member.principal_id == "local")
        .unwrap();
    assert_eq!(
        (person.kind, person.added_by.as_deref()),
        (PrincipalKind::User, None)
    );
    let partner = first
        .members
        .iter()
        .find(|member| member.principal_id == bot.id)
        .unwrap();
    assert_eq!(
        (partner.kind, partner.added_by.as_deref()),
        (PrincipalKind::Bot, Some("local"))
    );
    assert!(f.all_entries(&first.id).is_empty());

    let stored_key: String = f
        .raw()
        .query_row("SELECT direct_key FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        stored_key,
        direct_pair_key("local", &bot.id),
        "the API's pair key"
    );

    f.changes.take();
    let again = f
        .store
        .create_conversation(conversation(
            json!({ "kind": "direct", "members": [format!(" {} ", bot.id), "local"] }),
        ))
        .unwrap();
    assert_eq!(again.id, first.id);
    assert!(
        f.changes.take().is_empty(),
        "the existing conversation comes back without a write"
    );
}

#[test]
fn conversations_serialize_like_the_api_plus_local() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    f.text(&direct.id, &bot.id, "m_1");
    let wire = serde_json::to_value(f.visible(&direct.id)).unwrap();
    assert_eq!(
        sorted_keys(&wire),
        [
            "created_at",
            "created_by",
            "encryption",
            "id",
            "image_url",
            "kind",
            "last_activity_at",
            "last_entry",
            "local",
            "members",
            "pinned_at",
            "title",
            "unread_count"
        ]
    );
    assert_eq!(
        (wire["kind"].as_str(), wire["encryption"].as_str()),
        (Some("direct"), Some("plain"))
    );
    assert_eq!(wire["local"], true);
    assert_eq!(
        sorted_keys(&wire["members"][0]),
        ["added_by", "joined_at", "kind", "principal_id"]
    );
    let entry = &wire["last_entry"];
    assert_eq!(
        sorted_keys(entry),
        [
            "conversation_id",
            "created_at",
            "cursor",
            "envelope",
            "id",
            "kind",
            "message_id",
            "sender_id",
            "seq"
        ]
    );
    assert_eq!(entry["kind"], "message");
    assert_eq!(
        entry["envelope"],
        json!({ "v": 1, "enc": "none", "body": { "text": "m_1" } })
    );
}

#[test]
fn members_must_be_local_bots_and_end_to_end_encryption_needs_an_account() {
    let f = fixture();
    let bot = f.bot("Scout");
    let person = f.store.create_conversation(conversation(
        json!({ "kind": "direct", "members": ["user-b"] }),
    ));
    assert_eq!(code(person), codes::INBOX_MEMBER_NOT_ALLOWED);
    let stranger = f.store.create_conversation(conversation(
        json!({ "kind": "group", "members": [bot.id, "bot_elsewhere"] }),
    ));
    let error = stranger.unwrap_err();
    assert_eq!(error.code(), codes::INBOX_MEMBER_NOT_ALLOWED);
    assert!(error.message().contains("'bot_elsewhere'"), "{error}");
    assert_eq!(
        f.store.conversations().unwrap().len(),
        0,
        "nothing was stored"
    );

    let sealed = f.store.create_conversation(conversation(
        json!({ "kind": "direct", "members": [bot.id], "encryption": "e2e" }),
    ));
    let error = sealed.unwrap_err();
    assert_eq!(error.code(), codes::ACCOUNT_REQUIRED);
    assert_eq!(error.message(), "End-to-end encryption needs an account");

    for body in [
        json!({ "kind": "direct", "members": [bot.id, "bot_b"] }),
        json!({ "kind": "direct", "members": [bot.id], "title": "Named" }),
        json!({ "kind": "group", "members": ["local", " "] }),
    ] {
        let refused = f.store.create_conversation(conversation(body.clone()));
        assert_eq!(code(refused), codes::INVALID, "{body}");
    }

    let group = f.group(&[&bot]);
    let added = f.store.add_members(&group.id, vec!["user-b".into()]);
    assert_eq!(code(added), codes::INBOX_MEMBER_NOT_ALLOWED);
}

#[test]
fn a_group_writes_a_join_event_per_bot_and_adding_skips_members() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let c = f.bot("C");
    let group = f
        .store
        .create_conversation(conversation(
            json!({ "kind": "group", "title": "  Crew  ", "members": [a.id, b.id, a.id] }),
        ))
        .unwrap();
    assert_eq!(group.title.as_deref(), Some("Crew"));
    assert_eq!(member_ids(&group), ids(&["local", &a.id, &b.id]));
    let mut joins = f.member_bodies(&group.id);
    joins.sort_by_key(|body| body["principal_id"].to_string());
    let mut expected = vec![
        json!({ "op": "join", "principal_id": a.id, "by": "local" }),
        json!({ "op": "join", "principal_id": b.id, "by": "local" }),
    ];
    expected.sort_by_key(|body| body["principal_id"].to_string());
    assert_eq!(joins, expected);
    assert!(
        f.all_entries(&group.id)
            .iter()
            .all(|entry| entry.sender_id == "local" && entry.message_id.starts_with("member:"))
    );
    assert_eq!(group.unread_count, 0, "member events are never unread");

    let grown = f
        .store
        .add_members(&group.id, vec![b.id.clone(), c.id.clone()])
        .unwrap();
    assert_eq!(member_ids(&grown), ids(&["local", &a.id, &b.id, &c.id]));
    assert_eq!(
        f.member_bodies(&group.id).last(),
        Some(&json!({ "op": "join", "principal_id": c.id, "by": "local" }))
    );
    f.changes.take();
    let same = f.store.add_members(&group.id, vec![c.id.clone()]).unwrap();
    assert_eq!(same.members.len(), 4);
    assert!(f.changes.take().is_empty(), "current members are skipped");

    let direct = f.direct(&a);
    assert_eq!(
        code(f.store.add_members(&direct.id, vec![b.id.clone()])),
        codes::INBOX_DIRECT_FIXED
    );
    assert_eq!(
        code(
            f.store
                .rename_conversation(&direct.id, Some("Named".into()))
        ),
        codes::INBOX_DIRECT_FIXED
    );
    let renamed = f
        .store
        .rename_conversation(&group.id, Some(" Team ".into()))
        .unwrap();
    assert_eq!(renamed.title.as_deref(), Some("Team"));
    let untitled = f
        .store
        .rename_conversation(&group.id, Some("  ".into()))
        .unwrap();
    assert_eq!(untitled.title, None);
    let long = f
        .store
        .rename_conversation(&group.id, Some("t".repeat(121)));
    assert_eq!(code(long), codes::INVALID);
    assert_eq!(
        code(f.store.rename_conversation("missing", None)),
        codes::NOT_FOUND
    );
}

fn picture(width: u32, height: u32) -> String {
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
    let mut jpeg = std::io::Cursor::new(Vec::new());
    flow_like_types::images::image::DynamicImage::new_rgb8(width, height)
        .write_to(&mut jpeg, flow_like_types::images::image::ImageFormat::Jpeg)
        .unwrap();
    format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(jpeg.into_inner())
    )
}

#[test]
fn a_group_keeps_its_picture_as_a_small_png() {
    let f = fixture();
    let a = f.bot("A");
    let group = f.group(&[&a]);
    assert_eq!(group.image_url, None);

    f.changes.take();
    let pictured = f
        .store
        .set_conversation_image(&group.id, Some(picture(600, 300)))
        .unwrap();
    let stored = pictured.image_url.clone().unwrap();
    assert!(stored.starts_with("data:image/png;base64,"));
    let png = flow_like_types::images::image::load_from_memory(
        &flow_like_types::base64::Engine::decode(
            &flow_like_types::base64::engine::general_purpose::STANDARD,
            stored.trim_start_matches("data:image/png;base64,"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!((png.width(), png.height()), (256, 128));
    assert_eq!(f.visible(&group.id).image_url, Some(stored.clone()));
    assert!(!f.changes.take().is_empty());

    let same = f
        .store
        .set_conversation_image(&group.id, Some(picture(600, 300)))
        .unwrap();
    assert_eq!(same.image_url, Some(stored));
    assert!(
        f.changes.take().is_empty(),
        "the same picture changes nothing"
    );

    assert_eq!(
        code(
            f.store
                .set_conversation_image(&group.id, Some("data:text/html;base64,PGI+".into()))
        ),
        codes::INVALID
    );
    let direct = f.direct(&a);
    assert_eq!(
        code(
            f.store
                .set_conversation_image(&direct.id, Some(picture(8, 8)))
        ),
        codes::INBOX_DIRECT_FIXED
    );

    let cleared = f.store.set_conversation_image(&group.id, None).unwrap();
    assert_eq!(cleared.image_url, None);
    f.store
        .rename_conversation(&group.id, Some("Crew".into()))
        .unwrap();

    let mut changes: Vec<Value> = f
        .all_entries(&group.id)
        .into_iter()
        .filter(|entry| entry.kind == InboxEntryKind::Room)
        .map(|entry| {
            assert!(entry.message_id.starts_with("room:"));
            entry.envelope["body"].clone()
        })
        .collect();
    changes.reverse();
    assert_eq!(
        changes,
        [
            json!({ "change": "image", "by": "local" }),
            json!({ "change": "image", "removed": true, "by": "local" }),
            json!({ "change": "title", "title": "Crew", "by": "local" }),
        ],
        "one line per change, none for the unchanged picture"
    );
    assert_eq!(f.visible(&group.id).unread_count, 0);
}

#[test]
fn removing_writes_leave_events_and_the_person_takes_their_bots_along() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let group = f.group(&[&a, &b]);
    f.store.remove_member(&group.id, &a.id).unwrap();
    assert_eq!(
        f.member_bodies(&group.id).last(),
        Some(&json!({ "op": "leave", "principal_id": a.id, "by": "local" }))
    );
    assert_eq!(member_ids(&f.visible(&group.id)), ids(&["local", &b.id]));
    assert_eq!(
        code(f.store.remove_member(&group.id, &a.id)),
        codes::NOT_FOUND
    );

    let back = f.store.add_members(&group.id, vec![a.id.clone()]).unwrap();
    assert_eq!(member_ids(&back), ids(&["local", &a.id, &b.id]));

    let before = f.all_entries(&group.id).len();
    f.store.remove_member(&group.id, "local").unwrap();
    let leaves: Vec<Value> = f
        .raw()
        .prepare("SELECT envelope FROM entries WHERE conversation_id = ?1 ORDER BY created_at, id")
        .unwrap()
        .query_map([&group.id], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|text| serde_json::from_str::<Value>(&text.unwrap()).unwrap()["body"].clone())
        .skip(before)
        .collect();
    let mut left: Vec<String> = leaves
        .iter()
        .map(|body| {
            assert_eq!(
                (body["op"].as_str(), body["by"].as_str()),
                (Some("leave"), Some("local"))
            );
            body["principal_id"].as_str().unwrap().to_string()
        })
        .collect();
    left.sort();
    assert_eq!(left, ids(&["local", &a.id, &b.id]));
    assert_eq!(
        f.store.conversation(&group.id).unwrap(),
        None,
        "a group the person left is gone"
    );
    assert!(f.store.conversations().unwrap().is_empty());
    assert_eq!(
        code(f.store.entries(&group.id, &EntriesQuery::default())),
        codes::NOT_FOUND
    );

    let direct = f.direct(&a);
    assert_eq!(
        code(f.store.remove_member(&direct.id, &a.id)),
        codes::INBOX_DIRECT_FIXED
    );
}

#[test]
fn a_conversation_holds_at_most_fifty_members() {
    let f = fixture();
    let bots: Vec<Bot> = (0..50).map(|n| f.bot(&format!("Bot {n}"))).collect();
    let all: Vec<&str> = bots.iter().map(|bot| bot.id.as_str()).collect();
    let crowd = f
        .store
        .create_conversation(conversation(json!({ "kind": "group", "members": all })));
    assert_eq!(code(crowd), codes::INBOX_MEMBER_LIMIT);

    let group = f
        .store
        .create_conversation(conversation(
            json!({ "kind": "group", "members": all[..49] }),
        ))
        .unwrap();
    assert_eq!(group.members.len(), 50);
    let full = f.store.add_members(&group.id, vec![bots[49].id.clone()]);
    assert_eq!(code(full), codes::INBOX_MEMBER_LIMIT);
    let asked: Vec<String> = all.iter().map(|id| id.to_string()).collect();
    assert_eq!(
        code(f.store.add_members(&group.id, asked)),
        codes::INBOX_MEMBER_LIMIT
    );
    f.store.remove_member(&group.id, &bots[0].id).unwrap();
    let room = f
        .store
        .add_members(&group.id, vec![bots[49].id.clone()])
        .unwrap();
    assert_eq!(room.members.len(), 50);
}

#[test]
fn sends_take_valid_ids_known_kinds_and_one_plain_envelope() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let send = |body: Value| f.store.send(&direct.id, "local", entry_request(body));
    let text = json!({ "v": 1, "enc": "none", "body": { "text": "Hi" } });
    for message_id in ["", "with space", "member:abc", &"x".repeat(65)] {
        let refused =
            send(json!({ "message_id": message_id, "kind": "message", "envelope": text }));
        assert_eq!(code(refused), codes::INVALID, "{message_id}");
    }
    let member = send(json!({ "message_id": "m_1", "kind": "member", "envelope": text }));
    assert_eq!(code(member), codes::INVALID);
    let sealed = json!({ "v": 1, "enc": "hpke-x25519-sha256-chacha20poly1305", "key_id": "k", "enc_key": "ZW5j", "ct": "Y3Q=" });
    let envelopes =
        send(json!({ "message_id": "m_1", "kind": "message", "envelopes": { "local": sealed } }));
    assert_eq!(code(envelopes), codes::ACCOUNT_REQUIRED);
    let sealed_one = send(json!({ "message_id": "m_1", "kind": "message", "envelope": sealed }));
    assert_eq!(code(sealed_one), codes::ACCOUNT_REQUIRED);
    let version = json!({ "v": 2, "enc": "none", "body": {} });
    assert_eq!(
        code(send(
            json!({ "message_id": "m_1", "kind": "message", "envelope": version })
        )),
        codes::INVALID
    );
    assert_eq!(
        code(send(json!({ "message_id": "m_1", "kind": "message" }))),
        codes::INVALID
    );
    let untargeted = json!({ "v": 1, "enc": "none", "body": { "emoji": "👍" } });
    let reaction = send(json!({ "message_id": "m_1", "kind": "reaction", "envelope": untargeted }));
    assert_eq!(code(reaction), codes::INVALID);
    let huge = json!({ "v": 1, "enc": "none", "body": { "text": "x".repeat(64 * 1024) } });
    assert_eq!(
        code(send(
            json!({ "message_id": "m_1", "kind": "message", "envelope": huge })
        )),
        codes::INVALID
    );

    let outsider = f.send(
        &direct.id,
        "bot_stranger",
        "m_2",
        "message",
        json!({ "text": "Hi" }),
    );
    assert_eq!(code(outsider), codes::NOT_FOUND);
    let missing = f.send(
        "missing",
        "local",
        "m_3",
        "message",
        json!({ "text": "Hi" }),
    );
    assert_eq!(code(missing), codes::NOT_FOUND);
    assert!(f.all_entries(&direct.id).is_empty());
}

#[test]
fn a_repeated_message_id_returns_the_first_entry() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let other = f.group(&[&bot]);
    f.changes.take();
    let first = f.text(&direct.id, "local", "m_1");
    assert_eq!(f.changes.take(), [Change::Inbox(Some(direct.id.clone()))]);

    let again = f
        .send(
            &direct.id,
            "local",
            "m_1",
            "message",
            json!({ "text": "changed" }),
        )
        .unwrap();
    assert_eq!(again, first);
    assert!(f.changes.take().is_empty(), "a repeat writes nothing");
    assert_eq!(f.all_entries(&direct.id).len(), 1);

    let by_bot = f.send(
        &direct.id,
        &bot.id,
        "m_1",
        "message",
        json!({ "text": "Hi" }),
    );
    assert_eq!(code(by_bot), codes::INBOX_MESSAGE_ID_TAKEN);
    let elsewhere = f.send(
        &other.id,
        "local",
        "m_1",
        "message",
        json!({ "text": "Hi" }),
    );
    assert_eq!(code(elsewhere), codes::INBOX_MESSAGE_ID_TAKEN);
}

#[test]
fn reactions_act_on_any_message_but_edits_and_deletions_only_on_your_own() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let other = f.group(&[&bot]);
    f.text(&direct.id, &bot.id, "b_1");
    f.text(&direct.id, "local", "l_1");
    f.text(&other.id, "local", "o_1");
    let act = |sender: &str, message_id: &str, kind: &str, body: Value| {
        f.send(&direct.id, sender, message_id, kind, body)
    };

    let reaction = act(
        "local",
        "r_1",
        "reaction",
        json!({ "target": "b_1", "emoji": "👍", "op": "add" }),
    );
    assert_eq!(reaction.unwrap().kind, InboxEntryKind::Reaction);
    let edit = act(
        "local",
        "e_1",
        "edit",
        json!({ "target": "b_1", "text": "mine now" }),
    );
    assert_eq!(code(edit), codes::INBOX_NOT_YOUR_MESSAGE);
    let delete = act("local", "d_1", "delete", json!({ "target": "b_1" }));
    assert_eq!(code(delete), codes::INBOX_NOT_YOUR_MESSAGE);
    assert!(
        act(
            "local",
            "e_2",
            "edit",
            json!({ "target": "l_1", "text": "fixed" })
        )
        .is_ok()
    );
    assert!(
        act(
            &bot.id,
            "e_3",
            "edit",
            json!({ "target": "b_1", "text": "fixed" })
        )
        .is_ok()
    );
    assert!(act("local", "d_2", "delete", json!({ "target": "l_1" })).is_ok());

    for target in ["missing", "r_1", "o_1"] {
        let refused = act(
            "local",
            "r_2",
            "reaction",
            json!({ "target": target, "emoji": "👍", "op": "add" }),
        );
        assert_eq!(code(refused), codes::INBOX_TARGET_NOT_FOUND, "{target}");
    }
}

#[test]
fn unread_counts_messages_from_others_after_the_read_marker() {
    let f = fixture();
    let bot = f.bot("Scout");
    let group = f.group(&[&bot]);
    let a_1 = f.text(&group.id, &bot.id, "a_1");
    f.text(&group.id, &bot.id, "a_2");
    let mine = f.text(&group.id, "local", "l_1");
    let reaction = f
        .send(
            &group.id,
            &bot.id,
            "r_1",
            "reaction",
            json!({ "target": "l_1", "emoji": "👍", "op": "add" }),
        )
        .unwrap();
    let listed = f.visible(&group.id);
    assert_eq!(listed.unread_count, 2, "only others' messages count");
    assert_eq!(
        listed.last_entry,
        Some(mine.clone()),
        "the newest message, not the newest event"
    );

    f.changes.take();
    let marked = f.store.mark_read(&group.id, Some(&a_1.cursor)).unwrap();
    assert_eq!(marked.last_read_at, a_1.created_at);
    assert_eq!(unread(&f, &group.id), 1);
    assert_eq!(f.changes.take(), [Change::Inbox(Some(group.id.clone()))]);

    let all = f.store.mark_read(&group.id, None).unwrap();
    assert_eq!(unread(&f, &group.id), 0);
    assert_eq!(
        all.last_read_at, reaction.created_at,
        "everything means up to the newest event, a reaction too"
    );
    assert_eq!(f.visible(&group.id).last_activity_at, mine.created_at);
    f.changes.take();
    let stale = f.store.mark_read(&group.id, Some(&a_1.cursor)).unwrap();
    assert_eq!(
        stale.last_read_at, all.last_read_at,
        "a marker never moves back"
    );
    assert!(f.changes.take().is_empty());

    let elsewhere = f.direct(&bot);
    let foreign = f.text(&elsewhere.id, &bot.id, "x_1");
    assert_eq!(
        code(f.store.mark_read(&group.id, Some(&foreign.cursor))),
        codes::NOT_FOUND
    );
    assert_eq!(
        code(f.store.mark_read(&group.id, Some("not-a-cursor"))),
        codes::INVALID
    );
    assert_eq!(code(f.store.mark_read("missing", None)), codes::NOT_FOUND);

    f.text(&group.id, &bot.id, "a_3");
    assert_eq!(
        unread(&f, &group.id),
        1,
        "a message right after marking stays unread"
    );
    assert_eq!(unread(&f, &elsewhere.id), 1);
}

#[test]
fn the_list_puts_the_newest_activity_first() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    later();
    let group = f.group(&[&a, &b]);
    let order = || -> Vec<String> {
        f.store
            .conversations()
            .unwrap()
            .into_iter()
            .map(|conversation| conversation.id)
            .collect()
    };
    assert_eq!(order(), [group.id.clone(), direct.id.clone()]);

    later();
    let sent = f.text(&direct.id, &a.id, "m_1");
    assert_eq!(order(), [direct.id.clone(), group.id.clone()]);
    let listed = f.visible(&direct.id);
    assert_eq!(listed.last_activity_at, sent.created_at);

    later();
    f.store.add_members(&group.id, vec![]).unwrap();
    assert_eq!(order()[0], direct.id, "a no-op moves nothing");
    f.store.remove_member(&group.id, &b.id).unwrap();
    assert_eq!(
        order()[0],
        direct.id,
        "member events move nothing, like the API"
    );
    later();
    f.text(&group.id, &a.id, "m_2");
    assert_eq!(order()[0], group.id, "a message moves the conversation up");
}

#[test]
fn only_messages_move_the_activity_while_every_event_keeps_the_clock() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    let group = f.group(&[&a]);
    let message = f.text(&group.id, &a.id, "m_1");
    later();
    let reply = f.text(&direct.id, &a.id, "d_1");
    let reaction = f
        .send(
            &group.id,
            "local",
            "r_1",
            "reaction",
            json!({ "target": "m_1", "emoji": "👍", "op": "add" }),
        )
        .unwrap();
    let edit = f
        .send(
            &group.id,
            &a.id,
            "e_1",
            "edit",
            json!({ "target": "m_1", "text": "fixed" }),
        )
        .unwrap();
    f.store.add_members(&group.id, vec![b.id.clone()]).unwrap();
    let join = f.all_entries(&group.id)[0].clone();
    assert_eq!(join.kind, InboxEntryKind::Member);

    let times = [
        message.created_at,
        reaction.created_at,
        edit.created_at,
        join.created_at,
    ];
    assert!(
        times.windows(2).all(|pair| pair[0] < pair[1]),
        "every event lands after the newest one: {times:?}"
    );
    assert_eq!(
        f.visible(&group.id).last_activity_at,
        message.created_at,
        "reactions, edits and joins leave the activity at the last message"
    );
    let listed: Vec<String> = f
        .store
        .conversations()
        .unwrap()
        .into_iter()
        .map(|conversation| conversation.id)
        .collect();
    assert_eq!(listed, [direct.id.clone(), group.id.clone()]);
    assert_eq!(f.visible(&direct.id).last_activity_at, reply.created_at);

    let marked = f.store.mark_read(&group.id, None).unwrap();
    assert_eq!(
        marked.last_read_at, join.created_at,
        "read up to the newest event"
    );
    let next = f.text(&group.id, &a.id, "m_2");
    assert!(next.created_at > join.created_at);
    assert_eq!(unread(&f, &group.id), 1, "a message after it stays unread");
    assert_eq!(f.visible(&group.id).last_activity_at, next.created_at);
}

#[test]
fn deleting_a_bot_removes_its_direct_chats_and_it_leaves_its_groups() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    let group = f.group(&[&a, &b]);
    f.text(&direct.id, &a.id, "d_1");
    f.text(&group.id, &a.id, "g_1");
    f.store.attach_app(&a.id, "app-1").unwrap();
    f.store.set_secret(&a.id, "API_KEY", "v", None).unwrap();
    f.changes.take();

    f.store.delete_bot(&a.id).unwrap();
    let mut changes = f.changes.take();
    changes.sort_by_key(|change| format!("{change:?}"));
    let mut expected = vec![
        Change::Bots(Some(a.id.clone())),
        Change::Inbox(Some(direct.id.clone())),
        Change::Inbox(Some(group.id.clone())),
    ];
    expected.sort_by_key(|change| format!("{change:?}"));
    assert_eq!(changes, expected);

    assert_eq!(f.store.get_bot(&a.id).unwrap(), None);
    assert_eq!(f.store.conversation(&direct.id).unwrap(), None);
    let leftovers: (i64, i64, i64) = f
        .raw()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM entries WHERE conversation_id = ?1),
                    (SELECT COUNT(*) FROM bot_apps), (SELECT COUNT(*) FROM bot_secrets)",
            [&direct.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(leftovers, (0, 0, 0));
    let remaining = f.visible(&group.id);
    assert_eq!(member_ids(&remaining), ids(&["local", &b.id]));
    assert_eq!(
        f.member_bodies(&group.id).last(),
        Some(&json!({ "op": "leave", "principal_id": a.id, "by": "local" }))
    );
    assert!(
        f.all_entries(&group.id)
            .iter()
            .any(|entry| entry.message_id == "g_1")
    );
    assert_eq!(code(f.store.delete_bot(&a.id)), codes::NOT_FOUND);
}

#[test]
fn forgetting_removes_only_what_moved_and_repeats_safely() {
    let f = fixture();
    let a = f.bot("A");
    let direct = f.direct(&a);
    let group = f.group(&[&a]);
    f.store.attach_app(&a.id, "app-1").unwrap();
    f.store.set_secret(&a.id, "API_KEY", "v", None).unwrap();
    f.changes.take();

    f.store.forget_bot(&a.id, None).unwrap();
    assert_eq!(f.changes.take(), [Change::Bots(Some(a.id.clone()))]);
    assert_eq!(f.store.get_bot(&a.id).unwrap(), None);
    let leftovers: (i64, i64) = f
        .raw()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM bot_apps), (SELECT COUNT(*) FROM bot_secrets)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(leftovers, (0, 0));
    for kept in [&direct, &group] {
        assert!(member_ids(&f.visible(&kept.id)).contains(&a.id));
    }
    f.store.forget_bot(&a.id, None).unwrap();
    assert!(f.changes.take().is_empty());

    f.store.forget_conversation(&direct.id, None).unwrap();
    assert_eq!(f.changes.take(), [Change::Inbox(Some(direct.id.clone()))]);
    assert_eq!(f.store.conversation(&direct.id).unwrap(), None);
    assert_eq!(f.store.conversation("never-existed").unwrap(), None);
    f.store.forget_conversation(&direct.id, None).unwrap();
    assert!(f.changes.take().is_empty());
    assert_eq!(f.store.conversations().unwrap().len(), 1);
}

#[test]
fn forgetting_a_conversation_refuses_once_it_holds_a_newer_event() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    let group = f.group(&[&a]);
    f.text(&direct.id, "local", "d_1");
    let exported = f.store.export_bot(&a.id).unwrap();
    let newest = |id: &str| {
        exported
            .conversations
            .iter()
            .find(|item| item.conversation.id == id)
            .and_then(|item| item.entries.last())
            .map(|entry| entry.cursor.clone())
            .unwrap()
    };
    let (direct_cursor, group_cursor) = (newest(&direct.id), newest(&group.id));

    f.text(&direct.id, &a.id, "d_2");
    f.store.add_members(&group.id, vec![b.id.clone()]).unwrap();
    f.changes.take();
    for (id, cursor) in [(&direct.id, &direct_cursor), (&group.id, &group_cursor)] {
        let refused = f.store.forget_conversation(id, Some(cursor)).unwrap_err();
        assert_eq!(refused.code(), codes::LOCAL_CHANGED, "{id}");
        assert!(
            f.store.conversation(id).unwrap().is_some(),
            "nothing deleted"
        );
    }
    assert!(f.changes.take().is_empty());

    let current = f.all_entries(&direct.id)[0].cursor.clone();
    f.store
        .forget_conversation(&direct.id, Some(&current))
        .unwrap();
    assert_eq!(f.store.conversation(&direct.id).unwrap(), None);
    assert_eq!(
        code(f.store.forget_conversation(&group.id, Some("not-a-cursor"))),
        codes::INVALID
    );

    let quiet = f.direct(&b);
    f.store.forget_conversation(&quiet.id, Some("0:0")).unwrap();
    assert_eq!(f.store.conversation(&quiet.id).unwrap(), None);
    let spoken = f.direct(&a);
    f.text(&spoken.id, "local", "s_1");
    assert_eq!(
        code(f.store.forget_conversation(&spoken.id, Some("0:0"))),
        codes::LOCAL_CHANGED,
        "0:0 names a conversation that moved without events"
    );
}

#[test]
fn rebinding_points_what_stays_here_at_the_account_bot() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    let group = f.group(&[&a, &b]);
    let elsewhere = f.group(&[&b]);
    f.text(&direct.id, &a.id, "d_1");
    f.text(&group.id, &a.id, "g_1");
    f.text(&group.id, "local", "g_2");
    let cursors = |id: &str| -> Vec<String> {
        f.all_entries(id)
            .into_iter()
            .map(|entry| entry.cursor)
            .collect()
    };
    let (direct_cursors, group_cursors) = (cursors(&direct.id), cursors(&group.id));
    let before = f.store.get_bot(&a.id).unwrap().unwrap();
    let account = "bot_account00000000000000a";
    f.changes.take();

    assert_eq!(f.store.rebind_bot(&a.id, account).unwrap(), 2);
    let mut announced = f.changes.take();
    announced.sort_by_key(|change| format!("{change:?}"));
    let mut expected = vec![
        Change::Inbox(Some(direct.id.clone())),
        Change::Inbox(Some(group.id.clone())),
    ];
    expected.sort_by_key(|change| format!("{change:?}"));
    assert_eq!(announced, expected);

    assert_eq!(
        f.store.get_bot(&a.id).unwrap(),
        Some(before.clone()),
        "the bot, its updated_at included, stays as the move compares it"
    );
    assert_eq!(
        member_ids(&f.visible(&group.id)),
        ids(&["local", account, &b.id])
    );
    assert_eq!(member_ids(&f.visible(&direct.id)), ids(&["local", account]));
    assert_eq!(
        member_ids(&f.visible(&elsewhere.id)),
        ids(&["local", &b.id])
    );
    let senders = |id: &str| -> Vec<(String, String)> {
        f.all_entries(id)
            .into_iter()
            .filter(|entry| entry.kind == InboxEntryKind::Message)
            .map(|entry| (entry.message_id, entry.sender_id))
            .collect()
    };
    assert_eq!(senders(&direct.id), [("d_1".into(), account.to_string())]);
    assert_eq!(
        senders(&group.id),
        [
            ("g_2".into(), "local".to_string()),
            ("g_1".into(), account.to_string())
        ]
    );
    let joined: Vec<String> = f
        .member_bodies(&group.id)
        .iter()
        .map(|body| body["principal_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids(&joined.iter().map(String::as_str).collect::<Vec<_>>()),
        ids(&[account, &b.id])
    );
    let stored_key: String = f
        .raw()
        .query_row(
            "SELECT direct_key FROM conversations WHERE id = ?1",
            [&direct.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_key, direct_pair_key("local", account));
    assert_eq!(cursors(&direct.id), direct_cursors, "cursors stay");
    assert_eq!(cursors(&group.id), group_cursors);

    assert!(f.store.export_bot(&a.id).unwrap().conversations.is_empty());
    let with_b = f.store.export_bot(&b.id).unwrap();
    let group_export = with_b
        .conversations
        .iter()
        .find(|item| item.conversation.id == group.id)
        .unwrap();
    assert!(
        group_export
            .conversation
            .members
            .iter()
            .any(|member| member.principal_id == account)
    );

    assert_eq!(f.store.rebind_bot(&a.id, account).unwrap(), 0);
    assert!(
        f.changes.take().is_empty(),
        "a second rebind changes nothing"
    );
    let stamp = serde_json::to_value(before.updated_at).unwrap();
    f.store
        .forget_bot(&a.id, Some(stamp.as_str().unwrap()))
        .unwrap();

    for (local_id, account_id) in [
        (b.id.as_str(), "local"),
        (b.id.as_str(), " "),
        ("local", "bot_elsewhere"),
        ("", "bot_elsewhere"),
    ] {
        assert_eq!(
            code(f.store.rebind_bot(local_id, account_id)),
            codes::INVALID,
            "{local_id} -> {account_id}"
        );
    }
    let c = f.bot("C");
    assert_eq!(
        code(f.store.rebind_bot(&b.id, &c.id)),
        codes::INVALID,
        "an account id is never a bot on this device"
    );
    assert!(member_ids(&f.visible(&elsewhere.id)).contains(&b.id));
}

#[test]
fn the_bundle_holds_the_bot_its_apps_secrets_and_shared_conversations() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    let direct = f.direct(&a);
    f.text(&direct.id, "local", "d_1");
    f.text(&direct.id, &a.id, "d_2");
    later();
    let group = f.group(&[&a, &b]);
    f.text(&group.id, &b.id, "g_1");
    f.group(&[&b]);
    let left = f.group(&[&a, &b]);
    f.store.remove_member(&left.id, &a.id).unwrap();
    f.store.attach_app(&a.id, "app-1").unwrap();
    f.store
        .set_secret(&a.id, "API_KEY", "sk-1", Some(Some("OpenAI".into())))
        .unwrap();

    let bundle = f.store.export_bot(&a.id).unwrap();
    assert_eq!(Some(bundle.bot.clone()), f.store.get_bot(&a.id).unwrap());
    assert_eq!(bundle.app_ids, ["app-1"]);
    let shared: Vec<&str> = bundle
        .conversations
        .iter()
        .map(|exported| exported.conversation.id.as_str())
        .collect();
    assert_eq!(shared, [direct.id.as_str(), group.id.as_str()]);
    let direct_ids: Vec<&str> = bundle.conversations[0]
        .entries
        .iter()
        .map(|entry| entry.message_id.as_str())
        .collect();
    assert_eq!(direct_ids, ["d_1", "d_2"], "oldest first");
    let group_kinds: Vec<&str> = bundle.conversations[1]
        .entries
        .iter()
        .map(|entry| entry.kind.as_str())
        .collect();
    assert_eq!(group_kinds, ["member", "member", "message"]);

    let wire = serde_json::to_value(&bundle).unwrap();
    assert_eq!(
        sorted_keys(&wire),
        [
            "app_ids",
            "bot",
            "connectors",
            "conversations",
            "secrets",
            "skills"
        ]
    );
    assert_eq!(
        wire["secrets"],
        json!([{ "name": "API_KEY", "description": "OpenAI", "readable": true }])
    );
    assert_eq!(
        sorted_keys(&wire["conversations"][0]),
        ["conversation", "entries", "segments"]
    );
    assert_eq!(wire["conversations"][0]["conversation"]["local"], true);

    let quiet = f.bot("Quiet");
    let calls = f.key.calls().len();
    let empty = f.store.export_bot(&quiet.id).unwrap();
    assert!(empty.secrets.is_empty() && empty.conversations.is_empty() && empty.app_ids.is_empty());
    assert_eq!(f.key.calls().len(), calls, "no secrets, no keychain");
    assert_eq!(code(f.store.export_bot("bot_missing")), codes::NOT_FOUND);
}

#[test]
fn room_limits_live_on_a_bots_membership_and_pending_members_do_not_count() {
    use flow_like_types::bot_effective::BotRoomLimits;
    let f = fixture();
    let (scout, ranger) = (f.bot("Scout"), f.bot("Ranger"));
    let group = f.group(&[&scout, &ranger]);
    assert_eq!(f.store.member_limits(&group.id, &scout.id).unwrap(), None);

    let limits = BotRoomLimits {
        mask: Some(1),
        ..Default::default()
    };
    f.store
        .set_member_limits(&group.id, &scout.id, Some(limits.clone()))
        .unwrap();
    assert_eq!(
        f.store.member_limits(&group.id, &scout.id).unwrap(),
        Some(limits)
    );
    assert_eq!(f.store.member_limits(&group.id, &ranger.id).unwrap(), None);
    assert!(f.store.set_member_limits(&group.id, "local", None).is_err());

    f.store
        .write(|tx, _| {
            tx.execute(
                "UPDATE conversation_members SET status = 'PENDING' WHERE principal_id = ?1",
                [&ranger.id],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        member_ids(&f.visible(&group.id)),
        ids(&["local", &scout.id])
    );
    assert!(f.store.member_limits(&group.id, &ranger.id).is_err());
}

fn owner_message(message_id: &str, text: &str) -> SendEntryRequest {
    entry_request(json!({
        "message_id": message_id,
        "kind": "message",
        "envelope": plain_envelope(json!({ "text": text })),
    }))
}

#[test]
fn a_bot_writes_to_its_owner_in_the_direct_chat_it_opens_when_missing() {
    let f = fixture();
    let bot = f.bot("Scout");
    let written = f
        .store
        .send_to_owner(
            &bot.id,
            Some("mail-thread"),
            owner_message("mo-1", "Mail from Ada"),
        )
        .unwrap()
        .expect("written");
    assert_eq!(written.sender_id, bot.id);
    let direct = f.direct(&bot);
    assert_eq!(written.conversation_id, direct.id);
    assert_eq!(f.all_entries(&direct.id).len(), 1);
    let again = f
        .store
        .send_to_owner(&bot.id, None, owner_message("mo-1", "Mail from Ada"))
        .unwrap()
        .expect("repeat");
    assert_eq!(again.id, written.id);
    assert_eq!(f.all_entries(&direct.id).len(), 1);
}

#[test]
fn a_bot_does_not_write_to_the_owner_chat_it_runs_in() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let refused = f
        .store
        .send_to_owner(&bot.id, Some(&direct.id), owner_message("mo-2", "Hi"))
        .unwrap();
    assert!(refused.is_none());
    assert!(f.all_entries(&direct.id).is_empty());
}

#[test]
fn an_activity_is_host_written_once_and_neither_unread_nor_a_preview() {
    let f = fixture();
    let scout = f.bot("Scout");
    let pebble = f.bot("Pebble");
    let chat = f.direct(&scout);
    let body = ActivityBody::bot_ask("ask-1", &scout.id, &pebble.id, "exchange-1", "How many?");

    let first = f
        .store
        .post_activity(&scout.id, "act-1", &body, false)
        .unwrap()
        .unwrap();
    assert_eq!(first.kind, InboxEntryKind::Activity);
    assert_eq!(first.sender_id, scout.id);
    let again = f
        .store
        .post_activity(&scout.id, "act-1", &body, false)
        .unwrap()
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(
        f.all_entries(&chat.id)
            .iter()
            .filter(|entry| entry.kind == InboxEntryKind::Activity)
            .count(),
        1
    );
    assert_eq!(unread(&f, &chat.id), 0);
    assert!(f.visible(&chat.id).last_entry.is_none());

    assert!(
        f.store
            .post_activity(&pebble.id, "act-2", &body, false)
            .unwrap()
            .is_none()
    );
    assert!(
        f.store
            .post_activity(&pebble.id, "act-2", &body, true)
            .unwrap()
            .is_some()
    );
}
