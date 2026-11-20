use super::fixture;
use flow_like_types::bot_connector::{
    ConnectorAuth, ConnectorKind, ConnectorPolicy, ConnectorToolInfo, CreateBotConnector,
    UpdateBotConnector,
};

fn mcp(label: &str, url: &str) -> CreateBotConnector {
    CreateBotConnector {
        kind: ConnectorKind::Mcp,
        label: label.into(),
        url: Some(url.into()),
        auth: ConnectorAuth::None,
        grant: None,
        scopes: Vec::new(),
        default_policy: None,
        calendar_url: None,
    }
}

#[test]
fn a_connector_round_trips_with_its_policies_and_tool_snapshot() {
    let f = fixture();
    let bot = f.bot("Scout");
    let made = f
        .store
        .create_connector(&bot.id, mcp("Docs", "http://127.0.0.1:8931/mcp"))
        .unwrap();
    assert_eq!(made.default_policy, ConnectorPolicy::Ask);
    assert!(!made.has_grant);
    let tool = ConnectorToolInfo {
        name: "search".into(),
        description: "Find".into(),
        read_only_hint: Some(true),
        destructive_hint: None,
        schema_hash: "abc".into(),
    };
    f.store
        .set_connector_tools(&bot.id, &made.id, vec![tool.clone()])
        .unwrap();
    let updated = f
        .store
        .update_connector(
            &bot.id,
            &made.id,
            UpdateBotConnector {
                tool_policies: Some([("search".to_string(), ConnectorPolicy::Never)].into()),
                default_policy: Some(ConnectorPolicy::Read),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.default_policy, ConnectorPolicy::Read);
    let listed = f.store.connectors(&bot.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tools_snapshot, vec![tool]);
    assert_eq!(listed[0].tool_policies["search"], ConnectorPolicy::Never);
    assert_eq!(listed[0].url.as_deref(), Some("http://127.0.0.1:8931/mcp"));
    f.store.delete_connector(&bot.id, &made.id).unwrap();
    assert!(f.store.connectors(&bot.id).unwrap().is_empty());
}

#[test]
fn connectors_belong_to_their_bot_and_keep_unique_labels() {
    let f = fixture();
    let bot = f.bot("Scout");
    let other = f.bot("Ledger");
    let made = f
        .store
        .create_connector(&bot.id, mcp("Docs", "https://mcp.example.com/mcp"))
        .unwrap();
    assert!(
        f.store
            .create_connector(&bot.id, mcp("docs", "https://mcp.example.com/other"))
            .is_err()
    );
    assert!(
        f.store
            .create_connector(&other.id, mcp("Docs", "https://x.example/mcp"))
            .is_ok()
    );
    assert!(f.store.delete_connector(&other.id, &made.id).is_err());
    assert!(
        f.store
            .update_connector(&other.id, &made.id, UpdateBotConnector::default())
            .is_err()
    );
    assert!(f.store.connectors("missing").is_err());
}

#[test]
fn urls_and_accounts_are_checked() {
    let f = fixture();
    let bot = f.bot("Scout");
    for url in [
        "ftp://x/mcp",
        "https://user:pw@x/mcp",
        "not a url",
        "https://",
    ] {
        assert!(
            f.store.create_connector(&bot.id, mcp("A", url)).is_err(),
            "{url}"
        );
    }
    let account = CreateBotConnector {
        kind: ConnectorKind::Oauth,
        label: "GitHub".into(),
        url: None,
        auth: ConnectorAuth::Oauth {
            provider_id: "github".into(),
        },
        grant: None,
        scopes: vec!["repo".into()],
        default_policy: Some(ConnectorPolicy::Ask),
        calendar_url: None,
    };
    let made = f.store.create_connector(&bot.id, account).unwrap();
    assert_eq!(made.provider_id.as_deref(), Some("github"));
    assert_eq!(made.scopes, ["repo"]);
    let bare = CreateBotConnector {
        kind: ConnectorKind::Oauth,
        label: "Bare".into(),
        url: None,
        auth: ConnectorAuth::None,
        grant: None,
        scopes: Vec::new(),
        default_policy: None,
        calendar_url: None,
    };
    assert!(f.store.create_connector(&bot.id, bare).is_err());
}

#[test]
fn a_changed_tool_schema_is_not_trusted_until_confirmed() {
    let f = fixture();
    let bot = f.bot("Scout");
    let made = f
        .store
        .create_connector(&bot.id, mcp("Docs", "https://mcp.example.com/mcp"))
        .unwrap();
    let tool = |hash: &str| ConnectorToolInfo {
        name: "search".into(),
        description: String::new(),
        read_only_hint: Some(true),
        destructive_hint: None,
        schema_hash: hash.into(),
    };
    f.store
        .set_connector_tools(&bot.id, &made.id, vec![tool("v1")])
        .unwrap();
    let set = f
        .store
        .update_connector(
            &bot.id,
            &made.id,
            UpdateBotConnector {
                tool_policies: Some([("search".to_string(), ConnectorPolicy::Read)].into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(set.confirmed_hashes["search"], "v1");
    let drifted = f
        .store
        .set_connector_tools(&bot.id, &made.id, vec![tool("v2")])
        .unwrap();
    assert!(drifted.rule().drifted("search"));
    let confirmed = f
        .store
        .update_connector(
            &bot.id,
            &made.id,
            UpdateBotConnector {
                confirm_tools: vec!["search".into()],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!confirmed.rule().drifted("search"));
}

fn caldav(password: Option<&str>) -> CreateBotConnector {
    CreateBotConnector {
        kind: ConnectorKind::Caldav,
        label: "Nextcloud".into(),
        url: Some("http://nas.local:8080/remote.php/dav/".into()),
        auth: ConnectorAuth::Basic {
            username: " felix ".into(),
        },
        grant: password.map(str::to_string),
        scopes: Vec::new(),
        default_policy: None,
        calendar_url: None,
    }
}

fn calendar(url: &str, writable: bool) -> flow_like_types::bot_connector::CalDavCalendarInfo {
    flow_like_types::bot_connector::CalDavCalendarInfo {
        url: url.into(),
        name: url.rsplit('/').nth(1).unwrap_or_default().into(),
        color: None,
        writable,
        zone: Some("Europe/Berlin".into()),
    }
}

#[test]
fn a_caldav_connector_seals_its_password_and_keeps_its_calendars() {
    let f = fixture();
    let bot = f.bot("Secretary");
    assert!(f.store.create_connector(&bot.id, caldav(None)).is_err());
    let mut oauth_basic = caldav(Some("pw"));
    oauth_basic.kind = ConnectorKind::Oauth;
    assert!(f.store.create_connector(&bot.id, oauth_basic).is_err());

    let made = f
        .store
        .create_connector(&bot.id, caldav(Some(" app-pass word ")))
        .unwrap();
    assert!(made.has_grant);
    assert_eq!(made.provider_id.as_deref(), Some("caldav"));
    assert_eq!(
        made.auth,
        ConnectorAuth::Basic {
            username: "felix".into()
        }
    );
    assert!(!serde_json::to_string(&made).unwrap().contains("app-pass"));
    assert_eq!(
        f.store.connector_password(&bot.id, &made.id).unwrap(),
        " app-pass word "
    );

    let home = "http://nas.local:8080/remote.php/dav/calendars/felix/home/";
    let birthdays = "http://nas.local:8080/remote.php/dav/calendars/felix/birthdays/";
    let found = vec![calendar(birthdays, false), calendar(home, true)];
    let listed = f
        .store
        .set_connector_calendars(&bot.id, &made.id, found.clone(), None)
        .unwrap();
    assert_eq!(listed.calendar_url.as_deref(), Some(home));
    assert_eq!(listed.calendar().unwrap().name, "home");
    let picked = f
        .store
        .update_connector(
            &bot.id,
            &made.id,
            UpdateBotConnector {
                calendar_url: Some(birthdays.into()),
                grant: Some("new-pass".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(picked.calendar_url.as_deref(), Some(birthdays));
    assert_eq!(
        f.store.connector_password(&bot.id, &made.id).unwrap(),
        "new-pass"
    );
    let refreshed = f
        .store
        .set_connector_calendars(&bot.id, &made.id, vec![calendar(home, true)], None)
        .unwrap();
    assert_eq!(
        refreshed.calendar_url.as_deref(),
        Some(home),
        "a calendar that is gone falls back to the first writable one"
    );
    assert!(
        f.store
            .update_connector(
                &bot.id,
                &made.id,
                UpdateBotConnector {
                    calendar_url: Some("http://evil.example/".into()),
                    ..Default::default()
                },
            )
            .is_err()
    );
}
