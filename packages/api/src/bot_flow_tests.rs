use super::*;
use flow_like_types::bot_restrictions::BotFileScope;

fn effective(mask: i64, scopes: BotScopes) -> EffectivePermissions {
    EffectivePermissions {
        role_mask: Some(mask),
        scopes,
        ..EffectivePermissions::default()
    }
}

fn write_everywhere() -> i64 {
    BotPermission::ReadFiles.bit()
        | BotPermission::WriteFiles.bit()
        | BotPermission::ReadDatabase.bit()
        | BotPermission::WriteDatabase.bit()
}

fn events(ids: &[&str]) -> BotScopes {
    BotScopes {
        events: Some(ids.iter().map(|id| id.to_string()).collect()),
        ..BotScopes::default()
    }
}

#[test]
fn without_an_events_list_every_event_is_allowed() {
    assert!(event_allowed(&BotScopes::default(), "any"));
}

#[test]
fn an_events_list_allows_only_its_entries() {
    let scopes = events(&["a", "b"]);
    assert!(event_allowed(&scopes, "a"));
    assert!(event_allowed(&scopes, "b"));
    assert!(!event_allowed(&scopes, "c"));
    assert!(!event_allowed(&scopes, "A"));
}

#[test]
fn an_empty_events_list_allows_nothing() {
    assert!(!event_allowed(&events(&[]), "a"));
}

#[test]
fn the_board_list_is_independent_of_the_event_list() {
    let scopes = BotScopes {
        boards: Some(vec!["b1".into()]),
        ..events(&["e1"])
    };
    assert!(board_allowed(&scopes, "b1"));
    assert!(!board_allowed(&scopes, "b2"));
    assert!(event_allowed(&scopes, "e1"));
    assert!(board_allowed(&events(&["e1"]), "anything"));
}

#[test]
fn a_bot_that_may_write_everywhere_gets_full_execute_credentials() {
    assert!(matches!(
        credentials_access_for(&effective(write_everywhere(), BotScopes::default())),
        CredentialsAccess::ServerExecute
    ));
}

#[test]
fn a_flow_cannot_write_where_the_mask_or_scopes_forbid() {
    let without = |bit: BotPermission| write_everywhere() & !bit.bit();
    for restricted in [
        effective(without(BotPermission::WriteFiles), BotScopes::default()),
        effective(without(BotPermission::WriteDatabase), BotScopes::default()),
        effective(BotPermission::ExecuteEvents.bit(), BotScopes::default()),
        effective(
            write_everywhere(),
            BotScopes {
                files: BotFileScope {
                    read_only: true,
                    ..BotFileScope::default()
                },
                ..BotScopes::default()
            },
        ),
        effective(
            write_everywhere(),
            BotScopes {
                files: BotFileScope {
                    prefixes: vec!["reports".into()],
                    ..BotFileScope::default()
                },
                ..BotScopes::default()
            },
        ),
        effective(
            write_everywhere(),
            BotScopes {
                tables: Some(vec!["orders".into()]),
                ..BotScopes::default()
            },
        ),
    ] {
        assert!(matches!(
            credentials_access_for(&restricted),
            CredentialsAccess::ShadowExecute
        ));
    }
}

#[test]
fn a_person_run_keeps_its_own_credentials() {
    let (sub, mode) = credential_target(None, "owner", CredentialsAccess::ServerExecute);
    assert_eq!(sub, "owner");
    assert!(matches!(mode, CredentialsAccess::ServerExecute));
}

#[test]
fn only_bot_ids_name_a_bot_behind_a_connection() {
    assert_eq!(
        bot_from_technical_id("bot_abc").map(|bot| bot.bot_id),
        Some("bot_abc".to_string())
    );
    assert!(bot_from_technical_id("keyid123").is_none());
    assert!(bot_from_technical_id("").is_none());
}

#[test]
fn the_invoke_routes_admit_a_bot_through_this_module_only() {
    let routes = [
        ("invoke_event", include_str!("routes/app/events/invoke_event.rs")),
        (
            "invoke_event_async",
            include_str!("routes/app/events/invoke_event_async.rs"),
        ),
        ("invoke_board", include_str!("routes/app/board/invoke_board.rs")),
        (
            "invoke_board_async",
            include_str!("routes/app/board/invoke_board_async.rs"),
        ),
    ];
    for (name, source) in routes {
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("BotFlowRun::admit_"), "{name} admits");
        assert!(production.contains("credential_target("), "{name} credentials");
        assert!(production.contains("bind_executor("), "{name} binds");
        assert!(production.contains("run_user_id()"), "{name} payer");
        assert!(!production.contains("person_user_id()"), "{name} owner borrow");
    }
}

#[test]
fn the_routes_that_borrow_the_owner_stay_closed_to_bots() {
    for (name, source) in [
        ("regression", include_str!("routes/app/events/regression.rs")),
        ("mcp_operation", include_str!("routes/app/events/mcp_operation.rs")),
        ("page_trigger", include_str!("routes/app/events/page_trigger.rs")),
        ("create_api_key", include_str!("routes/app/api/create_api_key.rs")),
        ("actions", include_str!("routes/app/graph/actions.rs")),
    ] {
        assert!(source.contains("person_user_id()"), "{name}");
        assert!(!source.contains("run_user_id()"), "{name}");
    }
}
