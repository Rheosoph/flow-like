use super::fixture;
use crate::{ParkedTurn, codes};
use flow_like_types::{
    bot_message::{ApprovalRequest, ApprovalResponse},
    tool_grant::{ApprovalDecision, ToolGrantKey, approval_message_id},
};
use rusqlite::params;

fn key(tool: &str, scope: &str) -> ToolGrantKey {
    ToolGrantKey {
        tool: tool.into(),
        scope: scope.into(),
    }
}

fn card(turn_id: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: approval_message_id(turn_id, "hash1"),
        turn_id: turn_id.into(),
        tool: "code_run".into(),
        summary: "Run code that reaches example.com".into(),
        scope: "host:example.com".into(),
        args_hash: "hash1".into(),
        requester_id: None,
        requester_name: None,
        room: None,
    }
}

#[test]
fn a_grant_is_stored_once_listed_and_revoked() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    let other = fixture.bot("Ledger");
    let grant = fixture
        .store
        .grant_tool(&bot.id, &key("code_run", "host:example.com"))
        .unwrap();
    assert_eq!(
        (grant.tool.as_str(), grant.scope.as_str()),
        ("code_run", "host:example.com")
    );
    fixture
        .store
        .grant_tool(&bot.id, &key("code_run", "host:example.com"))
        .unwrap();
    assert_eq!(fixture.store.tool_grants(&bot.id).unwrap().len(), 1);
    assert!(fixture.store.tool_grants(&other.id).unwrap().is_empty());
    assert!(
        fixture
            .store
            .revoke_tool(&bot.id, &key("code_run", "host:example.com"))
            .unwrap()
    );
    assert!(
        !fixture
            .store
            .revoke_tool(&bot.id, &key("code_run", "host:example.com"))
            .unwrap()
    );
    assert!(fixture.store.tool_grants(&bot.id).unwrap().is_empty());
}

#[test]
fn malformed_keys_tokens_and_unknown_bots_are_refused() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    for bad in [key("code run", "*"), key("code_run", "bogus")] {
        assert_eq!(
            fixture.store.grant_tool(&bot.id, &bad).unwrap_err().code(),
            codes::INVALID
        );
    }
    assert_eq!(
        fixture
            .store
            .grant_tool("bot_nobody", &key("code_run", "*"))
            .unwrap_err()
            .code(),
        codes::NOT_FOUND
    );
}

#[test]
fn tokens_are_kept_beside_grants_but_never_listed_and_expire_after_a_day() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    fixture
        .store
        .put_tool_key(&bot.id, "code_run", "once:fresh")
        .unwrap();
    fixture
        .store
        .put_tool_key(&bot.id, "code_run", "deny:old")
        .unwrap();
    fixture
        .store
        .put_tool_key(&bot.id, "code_run", "host:a.com")
        .unwrap();
    fixture
        .raw()
        .execute(
            "UPDATE tool_grants SET granted_at = granted_at - ?1 WHERE scope = 'deny:old'",
            params![25 * 3_600_000_000_i64],
        )
        .unwrap();
    assert_eq!(fixture.store.tool_grants(&bot.id).unwrap().len(), 1);
    let mut keys = fixture.store.tool_keys(&bot.id).unwrap();
    keys.sort();
    assert_eq!(
        keys,
        [
            ("code_run".to_string(), "host:a.com".to_string()),
            ("code_run".to_string(), "once:fresh".to_string())
        ]
    );
    assert!(
        fixture
            .store
            .delete_tool_key(&bot.id, "code_run", "once:fresh")
            .unwrap()
    );
    assert!(
        !fixture
            .store
            .delete_tool_key(&bot.id, "code_run", "once:fresh")
            .unwrap()
    );
}

#[test]
fn deleting_a_bot_deletes_its_grants_and_parked_turns() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    fixture
        .store
        .grant_tool(&bot.id, &key("code_run", "*"))
        .unwrap();
    fixture
        .store
        .park_turn("bt_1", &bot.id, "c1", "m1", &card("bt_1"))
        .unwrap();
    fixture.store.delete_bot(&bot.id).unwrap();
    let raw = fixture.raw();
    for table in ["tool_grants", "parked_turns"] {
        let rows: i64 = raw
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

#[test]
fn parking_posts_one_card_and_answering_releases_the_turn_without_waking_the_bot() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    let direct = fixture.direct(&bot);
    let card = card("bt_1");
    let entry = fixture
        .store
        .park_turn("bt_1", &bot.id, &direct.id, "m1", &card)
        .unwrap();
    assert_eq!(entry.sender_id, bot.id);
    assert_eq!(entry.envelope["body"]["approval"]["tool"], "code_run");
    fixture
        .store
        .park_turn("bt_1", &bot.id, &direct.id, "m1", &card)
        .unwrap();
    assert_eq!(fixture.all_entries(&direct.id).len(), 1);
    assert_eq!(
        fixture.store.approval_card(&card.id).unwrap(),
        Some(card.clone())
    );
    let parked: Vec<ParkedTurn> = fixture.store.parked_turns().unwrap();
    assert_eq!(
        (parked.len(), parked[0].trigger_message_id.as_str()),
        (1, "m1")
    );

    let wrong = ApprovalResponse {
        approval_id: "ap-other".into(),
        decision: ApprovalDecision::Once,
    };
    assert_eq!(
        fixture
            .store
            .resolve_parked(&bot.id, "bt_1", &wrong)
            .unwrap(),
        None
    );
    let right = ApprovalResponse {
        approval_id: card.id.clone(),
        decision: ApprovalDecision::Once,
    };
    let released = fixture
        .store
        .resolve_parked(&bot.id, "bt_1", &right)
        .unwrap()
        .unwrap();
    assert_eq!(released.conversation_id, direct.id);
    assert!(fixture.store.parked_turns().unwrap().is_empty());
    assert_eq!(
        fixture
            .store
            .resolve_parked(&bot.id, "bt_1", &right)
            .unwrap(),
        None
    );
    let entries = fixture.all_entries(&direct.id);
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].envelope["body"]["approval_response"]["decision"],
        "once"
    );
}

#[test]
fn a_wait_older_than_seven_days_ends_with_a_notice() {
    let fixture = fixture();
    let bot = fixture.bot("Scout");
    let direct = fixture.direct(&bot);
    fixture
        .store
        .park_turn("bt_old", &bot.id, &direct.id, "m1", &card("bt_old"))
        .unwrap();
    fixture
        .store
        .park_turn("bt_new", &bot.id, &direct.id, "m2", &card("bt_new"))
        .unwrap();
    assert!(fixture.store.expire_parked().unwrap().is_empty());
    fixture
        .raw()
        .execute(
            "UPDATE parked_turns SET parked_at = parked_at - ?1 WHERE turn_id = 'bt_old'",
            params![8 * 86_400_000_000_i64],
        )
        .unwrap();
    let expired = fixture.store.expire_parked().unwrap();
    assert_eq!(
        expired
            .iter()
            .map(|p| p.turn_id.as_str())
            .collect::<Vec<_>>(),
        ["bt_old"]
    );
    assert_eq!(fixture.store.parked_turns().unwrap().len(), 1);
    let notice = fixture.all_entries(&direct.id).into_iter().next().unwrap();
    assert!(
        notice.envelope["body"]["text"]
            .as_str()
            .unwrap()
            .contains("seven days")
    );
}
