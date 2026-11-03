use super::*;
use crate::{
    codes,
    inbox::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, page_bounds},
};
use flow_like_types::inbox::EntryCursor;

/// Five messages of a bot in a group, after its join event; their cursors, oldest first.
fn five_messages(f: &Fixture) -> (String, Vec<String>) {
    let bot = f.bot("Talker");
    let group = f.group(&[&bot]);
    let cursors = (1..=5)
        .map(|n| f.text(&group.id, &bot.id, &format!("m_{n}")).cursor)
        .collect();
    (group.id, cursors)
}

fn page(
    f: &Fixture,
    conversation_id: &str,
    before: Option<&String>,
    after: Option<&String>,
    limit: u64,
) -> (Vec<String>, Option<String>) {
    let query = EntriesQuery {
        before: before.cloned(),
        after: after.cloned(),
        limit: Some(limit),
    };
    let page = f.store.entries(conversation_id, &query).unwrap();
    let ids = page
        .entries
        .into_iter()
        .filter(|entry| entry.kind.as_str() == "message")
        .map(|entry| entry.message_id)
        .collect();
    (ids, page.next_before)
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn pages_walk_back_with_before() {
    let f = fixture();
    let (group, c) = five_messages(&f);
    assert_eq!(
        page(&f, &group, None, None, 2),
        (ids(&["m_5", "m_4"]), Some(c[3].clone()))
    );
    assert_eq!(
        page(&f, &group, Some(&c[3]), None, 2),
        (ids(&["m_3", "m_2"]), Some(c[1].clone()))
    );
    let (oldest, next) = page(&f, &group, Some(&c[1]), None, 1);
    assert_eq!(oldest, ids(&["m_1"]));
    assert!(next.is_some(), "the join event is older still");
    assert_eq!(
        page(&f, &group, Some(&c[1]), None, 2),
        (ids(&["m_1"]), None)
    );
}

#[test]
fn polls_with_after_say_when_entries_remain_in_between() {
    let f = fixture();
    let (group, c) = five_messages(&f);
    assert_eq!(
        page(&f, &group, None, Some(&c[0]), 2),
        (ids(&["m_5", "m_4"]), Some(c[3].clone())),
        "entries between `after` and the page stay to be loaded"
    );
    assert_eq!(
        page(&f, &group, None, Some(&c[2]), 2),
        (ids(&["m_5", "m_4"]), None)
    );
    assert_eq!(
        page(&f, &group, Some(&c[4]), Some(&c[1]), 5),
        (ids(&["m_4", "m_3"]), None)
    );
    assert_eq!(page(&f, &group, None, Some(&c[4]), 5), (Vec::new(), None));
}

#[test]
fn events_of_one_write_share_a_time_and_order_by_id() {
    let f = fixture();
    let bots: Vec<Bot> = (0..3).map(|n| f.bot(&format!("Bot {n}"))).collect();
    let group = f.group(&bots.iter().collect::<Vec<_>>());
    let mut joins: Vec<EntryCursor> = f
        .all_entries(&group.id)
        .iter()
        .map(|entry| EntryCursor::parse(&entry.cursor).unwrap())
        .collect();
    assert_eq!(joins.len(), 3);
    assert!(
        joins
            .iter()
            .all(|cursor| cursor.created_at_micros == joins[0].created_at_micros)
    );
    joins.sort_by(|a, b| b.cmp(a));

    let mut walked = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..3 {
        let query = EntriesQuery {
            before: before.clone(),
            after: None,
            limit: Some(1),
        };
        let entry = f.store.entries(&group.id, &query).unwrap().entries[0].clone();
        walked.push(EntryCursor::parse(&entry.cursor).unwrap());
        before = Some(entry.cursor);
    }
    assert_eq!(walked, joins);
}

#[test]
fn every_write_lands_after_the_newest_event() {
    let f = fixture();
    let bot = f.bot("Fast");
    let direct = f.direct(&bot);
    let sent: Vec<InboxEntry> = (0..20)
        .map(|n| f.text(&direct.id, &bot.id, &format!("burst_{n}")))
        .collect();
    for pair in sent.windows(2) {
        assert!(
            pair[1].created_at > pair[0].created_at,
            "{} then {}",
            pair[0].cursor,
            pair[1].cursor
        );
    }
    for entry in &sent {
        let cursor = EntryCursor::parse(&entry.cursor).unwrap();
        assert_eq!(
            cursor.created_at_micros,
            entry.created_at.timestamp_micros()
        );
        assert_eq!(cursor.entry_id, entry.id);
        assert_eq!(cursor.created_at_micros % 1_000, 0, "millisecond precision");
    }
    let tail = sent[9].cursor.clone();
    let polled = page(&f, &direct.id, None, Some(&tail), 100).0;
    let expected: Vec<String> = (10..20).rev().map(|n| format!("burst_{n}")).collect();
    assert_eq!(
        polled, expected,
        "a poll from any cursor sees every later write"
    );
}

#[test]
fn page_bounds_clamp_the_limit_and_refuse_malformed_cursors() {
    let query = |limit: Option<u64>, before: Option<&str>| EntriesQuery {
        before: before.map(str::to_string),
        after: None,
        limit,
    };
    assert_eq!(
        page_bounds(&query(None, None)).unwrap().limit,
        DEFAULT_PAGE_SIZE
    );
    assert_eq!(page_bounds(&query(Some(0), None)).unwrap().limit, 1);
    assert_eq!(
        page_bounds(&query(Some(500), None)).unwrap().limit,
        MAX_PAGE_SIZE
    );
    assert_eq!(page_bounds(&query(None, Some(" "))).unwrap().before, None);
    let bounds = page_bounds(&query(None, Some("1700000000000000:abc"))).unwrap();
    assert_eq!(
        bounds.before,
        Some(EntryCursor::new(1_700_000_000_000_000, "abc"))
    );
    for bad in [
        "abc",
        "12:",
        "9999999999999999999:abc",
        "9223372036854775807:abc",
    ] {
        let error = page_bounds(&query(None, Some(bad))).unwrap_err();
        assert_eq!(error.code(), codes::INVALID, "{bad}");
        assert!(error.message().contains("before"), "{bad}: {error}");
    }
    let after = EntriesQuery {
        after: Some("nope".into()),
        ..EntriesQuery::default()
    };
    assert!(page_bounds(&after).unwrap_err().message().contains("after"));
}

#[test]
fn reading_entries_needs_an_active_membership() {
    let f = fixture();
    assert_eq!(
        code(f.store.entries("missing", &EntriesQuery::default())),
        codes::NOT_FOUND
    );
    let bad = EntriesQuery {
        before: Some("garbage".into()),
        ..EntriesQuery::default()
    };
    assert_eq!(code(f.store.entries("missing", &bad)), codes::INVALID);
}
