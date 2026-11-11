use super::{
    bot::{authenticated, drop_reason, ids_in, save_files},
    bot_address::{address_of, bot_of_address, claim_in, release_in},
};
use flow_like_catalog_core::{InboundEmail, InboundEmailAuthentication, InboundEmailVerdict};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStoreExt, memory::InMemory},
};
use flow_like_types::bot_message::WorkspaceHandle;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use serde_json::json;
use std::sync::Arc;

fn email(from: &str, extra: serde_json::Value) -> InboundEmail {
    let mut value = json!({
        "id": "i", "delivery_id": "d", "envelope_from": from, "recipient": "bot@flow-like.me",
        "raw_path": {"path": "p", "store_ref": "s"},
        "from": [{"email": from}],
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

fn verdict(status: &str) -> Option<InboundEmailVerdict> {
    Some(InboundEmailVerdict {
        status: status.into(),
    })
}

#[test]
fn automated_failed_and_self_sent_mail_never_wakes_the_bot() {
    let own = "bot@flow-like.me";
    assert_eq!(drop_reason(&email("ann@x.com", json!({})), own), None);
    assert_eq!(
        drop_reason(&email("ann@x.com", json!({"automated": true})), own),
        Some("automated")
    );
    assert_eq!(
        drop_reason(&email(own, json!({})), own),
        Some("no sender or sent by the bot itself")
    );
    assert!(drop_reason(&email("", json!({"from": []})), own).is_some());
    let mut spoofed = email("ann@x.com", json!({}));
    spoofed.authentication = Some(InboundEmailAuthentication {
        dmarc: verdict("FAIL"),
        ..Default::default()
    });
    assert_eq!(drop_reason(&spoofed, own), Some("failed authentication"));
}

#[tokio::test]
async fn stored_mail_files_land_where_their_handles_resolve() {
    let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
    let long_text = "x".repeat(5_000);
    let mail = email(
        "a@example.com",
        json!({
            "attachments": [{
                "filename": "key.asc", "content_type": "application/pgp-keys", "size": 3,
                "path": {"path": "mail/d/key.asc", "store_ref": "s"},
            }],
            "text_path": {"path": "mail/d/body.txt", "store_ref": "s"},
        }),
    );
    let objects = vec![
        ("mail/d/key.asc".to_owned(), b"key".to_vec()),
        ("mail/d/body.txt".to_owned(), long_text.clone().into_bytes()),
    ];
    let stored = save_files(&store, "ws", &mail, &objects, "f")
        .await
        .unwrap();
    let attachment: WorkspaceHandle =
        serde_json::from_value(stored.attachments[0]["handle"].clone()).unwrap();
    let text = stored.text_file.unwrap();
    for (handle, path, expected) in [
        (&attachment, "bot/inbox/f/key.asc", b"key".as_slice()),
        (&text, "bot/inbox/f/body.txt", long_text.as_bytes()),
    ] {
        assert_eq!(handle.app_id, "ws");
        assert_eq!(handle.path, path);
        let key = store
            .construct_upload(&handle.app_id, &handle.path)
            .await
            .unwrap();
        assert_eq!(key.as_ref(), format!("apps/ws/upload/{path}"));
        let bytes = store
            .as_generic()
            .get(&key)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), expected);
    }
}

#[test]
fn a_sender_is_verified_only_by_dmarc_or_spf_and_dkim_together() {
    assert!(!authenticated(None));
    let auth = |spf, dkim, dmarc| {
        authenticated(Some(&InboundEmailAuthentication {
            spf: verdict(spf),
            dkim: verdict(dkim),
            dmarc: verdict(dmarc),
            ..Default::default()
        }))
    };
    assert!(auth("FAIL", "FAIL", "PASS"));
    assert!(auth("PASS", "PASS", "GRAY"));
    assert!(!auth("PASS", "FAIL", "GRAY"));
    assert!(!auth("GRAY", "GRAY", "GRAY"));
}

#[test]
fn threading_headers_yield_normalized_ids() {
    assert_eq!(
        ids_in("<A@x.com> <b@Y.com>, <c@z.org>"),
        ["a@x.com", "b@y.com", "c@z.org"]
    );
    assert!(ids_in("   ").is_empty());
}

async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE "InboundMailAddress" ("address" TEXT PRIMARY KEY, "appId" TEXT NOT NULL,
        "eventId" TEXT NOT NULL, "kind" TEXT NOT NULL, "active" BOOLEAN NOT NULL DEFAULT 1,
        "createdAt" BIGINT NOT NULL, "updatedAt" BIGINT NOT NULL)"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(r#"CREATE TABLE "Bot" ("id" TEXT PRIMARY KEY, "deletedAt" TEXT)"#)
        .await
        .unwrap();
    db.execute_unprepared(r#"INSERT INTO "Bot" ("id") VALUES ('bot_a'), ('bot_b')"#)
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn a_bot_claims_an_address_and_mail_routes_to_it() {
    let db = database().await;
    claim_in(&db, "bot_a", "ann@flow-like.me").await.unwrap();
    assert_eq!(
        address_of(&db, "bot_a").await.unwrap().as_deref(),
        Some("ann@flow-like.me")
    );
    assert_eq!(
        bot_of_address(&db, "ann@flow-like.me")
            .await
            .unwrap()
            .as_deref(),
        Some("bot_a")
    );
}

#[tokio::test]
async fn an_address_belongs_to_one_bot_and_an_event_alias_blocks_it_too() {
    let db = database().await;
    claim_in(&db, "bot_a", "ann@flow-like.me").await.unwrap();
    assert!(claim_in(&db, "bot_b", "ann@flow-like.me").await.is_err());
    assert_eq!(address_of(&db, "bot_b").await.unwrap(), None);
    db.execute_unprepared(
        r#"INSERT INTO "InboundMailAddress" VALUES ('orders@flow-like.me','app1','ev1','alias',1,0,0)"#,
    )
    .await
    .unwrap();
    assert!(claim_in(&db, "bot_b", "orders@flow-like.me").await.is_err());
}

#[tokio::test]
async fn changing_the_address_releases_the_old_one_and_only_that_bot_may_retake_it() {
    let db = database().await;
    claim_in(&db, "bot_a", "old@flow-like.me").await.unwrap();
    claim_in(&db, "bot_a", "new@flow-like.me").await.unwrap();
    assert_eq!(
        address_of(&db, "bot_a").await.unwrap().as_deref(),
        Some("new@flow-like.me")
    );
    assert_eq!(bot_of_address(&db, "old@flow-like.me").await.unwrap(), None);
    assert!(claim_in(&db, "bot_b", "old@flow-like.me").await.is_err());
    claim_in(&db, "bot_a", "old@flow-like.me").await.unwrap();
}

#[tokio::test]
async fn a_released_address_stops_delivering_and_stays_reserved() {
    let db = database().await;
    claim_in(&db, "bot_a", "ann@flow-like.me").await.unwrap();
    assert_eq!(release_in(&db, "bot_a").await.unwrap(), 1);
    assert_eq!(address_of(&db, "bot_a").await.unwrap(), None);
    assert_eq!(bot_of_address(&db, "ann@flow-like.me").await.unwrap(), None);
    assert!(claim_in(&db, "bot_b", "ann@flow-like.me").await.is_err());
}

#[tokio::test]
async fn mail_for_a_deleted_bot_has_no_destination() {
    let db = database().await;
    claim_in(&db, "bot_a", "ann@flow-like.me").await.unwrap();
    db.execute_unprepared(r#"UPDATE "Bot" SET "deletedAt"='now' WHERE "id"='bot_a'"#)
        .await
        .unwrap();
    assert_eq!(bot_of_address(&db, "ann@flow-like.me").await.unwrap(), None);
}

#[test]
fn only_the_owners_own_profile_facts_name_owner_addresses() {
    use super::owner_addresses::addresses_in;
    use flow_like_bot_memory::{
        facts::{Fact, FactKind},
        model::FactSource,
    };
    let fact = |text: &str, subject: &str, kind: FactKind, source: FactSource| Fact {
        id: "f".into(),
        text: text.into(),
        kind,
        subject: subject.into(),
        geojson: None,
        time: None,
        expires_at: None,
        valid_from: None,
        valid_to: None,
        recorded_at: chrono::Utc::now(),
        expired_at: None,
        evidence: Vec::new(),
        segment_id: None,
        source,
        confidence: 1.0,
        superseded_by: None,
        content_hash: String::new(),
    };
    let facts = vec![
        fact(
            "The user's email address is Me@Home.org.",
            "user",
            FactKind::Profile,
            FactSource::UserStated,
        ),
        fact(
            "Ann's email is ann@x.com",
            "user.sister",
            FactKind::Profile,
            FactSource::UserStated,
        ),
        fact(
            "The user's email is eve@x.com",
            "user",
            FactKind::Knowledge,
            FactSource::ToolUntrusted,
        ),
        fact(
            "The user's mail is bot@x.com",
            "user",
            FactKind::Profile,
            FactSource::ToolUntrusted,
        ),
        fact(
            "The user lives at 1 Main St, user@home",
            "user",
            FactKind::Profile,
            FactSource::UserStated,
        ),
    ];
    assert_eq!(addresses_in(&facts), vec!["me@home.org"]);
}

#[test]
fn owner_mail_needs_a_passing_dmarc_verdict_and_only_named_senders() {
    use super::owner_addresses::{dmarc_passed, from_addresses};
    let signed = |dmarc: Option<InboundEmailVerdict>| InboundEmailAuthentication {
        dmarc,
        spf: verdict("PASS"),
        dkim: verdict("PASS"),
        ..Default::default()
    };
    assert!(dmarc_passed(Some(&signed(verdict("pass")))));
    assert!(!dmarc_passed(Some(&signed(verdict("GRAY")))));
    assert!(!dmarc_passed(Some(&signed(None))));
    assert!(!dmarc_passed(None));
    assert_eq!(
        from_addresses(&email("Me@Home.org", json!({}))),
        Some(vec!["me@home.org".to_owned()])
    );
    assert_eq!(from_addresses(&email("", json!({"from": []}))), None);
    let mut nameless = email("me@home.org", json!({}));
    nameless
        .from
        .push(serde_json::from_value(json!({"name": "x"})).unwrap());
    assert_eq!(from_addresses(&nameless), None);
}
