use super::{
    ask::{ask_message_id, ask_trigger, run_owner},
    fire::{turn_state, wait_for_turn},
    webhooks::{
        HookQuery, MAX_PAYLOAD_BYTES, WindowLimiter, admit, authorized, check_payload_size,
        create_in, hash_secret, payload_of, rotate_in, trigger_for,
    },
};
use crate::{
    entity::{bot, bot_turn, bot_webhook, inbox_entry},
    routes::inbox::{
        fixture::{assert_code, at, database as inbox_database, insert_bot},
        triggers::trigger_outgoing,
    },
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use flow_like_types::{
    bot_turn::turn_id,
    inbox::{InboxEntryKind, TriggerBody, TriggerSource},
};
use hmac::{Hmac, Mac};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait,
};
use serde_json::json;
use sha2::Sha256;
use std::time::{Duration, Instant};

async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotWebhook" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "name" TEXT NOT NULL,
            "secretHash" TEXT NOT NULL, "instruction" TEXT NOT NULL, "enabled" BOOLEAN NOT NULL,
            "createdAt" TEXT NOT NULL, "lastCalledAt" TEXT)"#,
    )
    .await
    .unwrap();
    db
}

async fn hooked_bot(db: &DatabaseConnection, id: &str, owner: &str) {
    insert_bot(db, id, owner, "SERVER").await;
    let row = bot::Entity::find_by_id(id).one(db).await.unwrap().unwrap();
    let mut row: bot::ActiveModel = row.into();
    row.capabilities = Set(crate::entity::json_types::StringList(vec![
        "webhook".into(),
    ]));
    row.update(db).await.unwrap();
}

async fn hook(db: &DatabaseConnection, bot_id: &str, secret: &str) -> bot_webhook::Model {
    let row = create_in(
        db,
        "user-a",
        bot_id,
        "New order".into(),
        "Summarize the order".into(),
        hash_secret(secret),
    )
    .await
    .unwrap();
    row
}

fn secret_header(secret: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-flow-like-webhook-secret",
        HeaderValue::from_str(secret).unwrap(),
    );
    headers
}

fn signed(key: &str, body: &[u8]) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(body);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-flow-like-signature",
        HeaderValue::from_str(&format!(
            "sha256={}",
            hex::encode(mac.finalize().into_bytes())
        ))
        .unwrap(),
    );
    headers
}

#[test]
fn the_secret_opens_the_hook_in_a_header_or_as_a_bearer_token() {
    let stored = hash_secret("whsec_good");
    assert!(authorized(
        &stored,
        &secret_header("whsec_good"),
        None,
        b"{}"
    ));
    let mut bearer = HeaderMap::new();
    bearer.insert(
        "authorization",
        HeaderValue::from_static("Bearer whsec_good"),
    );
    assert!(authorized(&stored, &bearer, None, b"{}"));
}

#[test]
fn the_forwarded_bearer_secret_survives_cloudfront_origin_signing() {
    let stored = hash_secret("whsec_good");
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("AWS4-HMAC-SHA256 Credential=origin"),
    );
    headers.insert(
        "x-flow-like-authorization",
        HeaderValue::from_static("Bearer whsec_good"),
    );
    assert!(authorized(&stored, &headers, None, b"{}"));
    headers.insert(
        "x-flow-like-authorization",
        HeaderValue::from_static("Bearer whsec_bad"),
    );
    assert!(!authorized(&stored, &headers, None, b"{}"));
}

#[test]
fn a_url_secret_authenticates_without_custom_headers_or_a_signature() {
    let uri = "/bots/hooks/bh_1?secret=whsec%5Fgood&wait=30"
        .parse()
        .unwrap();
    let axum::extract::Query(query) =
        axum::extract::Query::<HookQuery>::try_from_uri(&uri).unwrap();
    assert_eq!(query.wait, Some(30));
    assert!(authorized(
        &hash_secret("whsec_good"),
        &HeaderMap::new(),
        query.secret.as_deref(),
        br#"{"type":"order.created"}"#,
    ));

    let uri = "/bots/hooks/bh_1?wait=30".parse().unwrap();
    let axum::extract::Query(query) =
        axum::extract::Query::<HookQuery>::try_from_uri(&uri).unwrap();
    assert!(query.secret.is_none());
}

#[test]
fn a_wrong_empty_or_hashed_url_secret_opens_nothing() {
    let stored = hash_secret("whsec_good");
    for secret in ["whsec_bad", "", " ", stored.as_str()] {
        assert!(!authorized(&stored, &HeaderMap::new(), Some(secret), b"{}"));
    }
    assert!(!authorized(
        "",
        &HeaderMap::new(),
        Some("whsec_good"),
        b"{}"
    ));
}

#[test]
fn a_wrong_or_missing_secret_opens_nothing() {
    let stored = hash_secret("whsec_good");
    assert!(!authorized(
        &stored,
        &secret_header("whsec_bad"),
        None,
        b"{}"
    ));
    assert!(!authorized(&stored, &secret_header(&stored), None, b"{}"));
    assert!(!authorized(&stored, &HeaderMap::new(), None, b"{}"));
    assert!(!authorized("", &HeaderMap::new(), None, b"{}"));
    assert!(!authorized("", &secret_header(""), None, b"{}"));
}

#[test]
fn a_signature_of_the_exact_body_under_the_signing_key_opens_the_hook() {
    let stored = hash_secret("whsec_good");
    assert!(authorized(
        &stored,
        &signed(&stored, b"{\"a\":1}"),
        None,
        b"{\"a\":1}"
    ));
    assert!(!authorized(
        &stored,
        &signed(&stored, b"{\"a\":1}"),
        None,
        b"{\"a\":2}"
    ));
    assert!(!authorized(
        &stored,
        &signed("whsec_good", b"{}"),
        None,
        b"{}"
    ));
    assert!(!authorized("", &signed("", b"{}"), None, b"{}"));
}

#[test]
fn a_payload_over_the_cap_is_refused_and_one_at_the_cap_is_not() {
    assert!(check_payload_size(MAX_PAYLOAD_BYTES).is_ok());
    let error = check_payload_size(MAX_PAYLOAD_BYTES + 1).unwrap_err();
    assert_code(
        &error,
        StatusCode::PAYLOAD_TOO_LARGE,
        "BOT_WEBHOOK_PAYLOAD_TOO_LARGE",
    );
}

#[test]
fn the_limiter_stops_after_its_limit_and_opens_with_the_next_window() {
    let limiter = WindowLimiter::new(3, Duration::from_secs(60));
    let start = Instant::now();
    assert!((0..3).all(|_| limiter.allow("a", start)));
    assert!(!limiter.allow("a", start));
    assert!(limiter.allow("b", start));
    assert!(limiter.exhausted("a", start));
    assert!(limiter.allow("a", start + Duration::from_secs(61)));
}

#[tokio::test]
async fn an_admitted_call_needs_the_secret_and_the_cap_and_the_capability() {
    let db = database().await;
    hooked_bot(&db, "bot_a", "user-a").await;
    let hook = hook(&db, "bot_a", "whsec_good").await;
    let now = Instant::now();

    let (admitted, bot) = admit(
        &db,
        &hook.id,
        &secret_header("whsec_good"),
        None,
        b"{}",
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        (admitted.id.as_str(), bot.id.as_str()),
        (hook.id.as_str(), "bot_a")
    );

    let wrong = admit(&db, &hook.id, &secret_header("whsec_bad"), None, b"{}", now).await;
    assert_code(
        &wrong.unwrap_err(),
        StatusCode::UNAUTHORIZED,
        "BOT_WEBHOOK_UNAUTHORIZED",
    );
    let missing = admit(
        &db,
        "bh_unknown",
        &secret_header("whsec_good"),
        None,
        b"{}",
        now,
    )
    .await;
    assert_code(
        &missing.unwrap_err(),
        StatusCode::UNAUTHORIZED,
        "BOT_WEBHOOK_UNAUTHORIZED",
    );

    let big = vec![b'x'; MAX_PAYLOAD_BYTES + 1];
    let refused = admit(&db, &hook.id, &secret_header("whsec_good"), None, &big, now).await;
    assert_code(
        &refused.unwrap_err(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "BOT_WEBHOOK_PAYLOAD_TOO_LARGE",
    );

    let mut off: bot_webhook::ActiveModel = hook.clone().into();
    off.enabled = Set(false);
    off.update(&db).await.unwrap();
    let disabled = admit(
        &db,
        &hook.id,
        &secret_header("whsec_good"),
        None,
        b"{}",
        now,
    )
    .await;
    assert_code(
        &disabled.unwrap_err(),
        StatusCode::FORBIDDEN,
        "BOT_WEBHOOK_DISABLED",
    );
}

#[tokio::test]
async fn a_url_secret_admits_a_call_and_rotation_revokes_the_old_url() {
    let db = database().await;
    hooked_bot(&db, "bot_a", "user-a").await;
    let hook = hook(&db, "bot_a", "whsec_old").await;
    let now = Instant::now();
    let headers = HeaderMap::new();

    let (admitted, _) = admit(&db, &hook.id, &headers, Some("whsec_old"), b"{}", now)
        .await
        .unwrap();
    assert_eq!(admitted.id, hook.id);

    for secret in [None, Some("wrong")] {
        let error = admit(&db, &hook.id, &headers, secret, b"{}", now)
            .await
            .unwrap_err();
        assert_code(&error, StatusCode::UNAUTHORIZED, "BOT_WEBHOOK_UNAUTHORIZED");
    }

    let (_, secret) = rotate_in(&db, "user-a", "bot_a", &hook.id).await.unwrap();
    let old = admit(&db, &hook.id, &headers, Some("whsec_old"), b"{}", now)
        .await
        .unwrap_err();
    assert_code(&old, StatusCode::UNAUTHORIZED, "BOT_WEBHOOK_UNAUTHORIZED");
    admit(&db, &hook.id, &headers, Some(&secret), b"{}", now)
        .await
        .unwrap();

    let mut off: bot_webhook::ActiveModel = hook.into();
    off.enabled = Set(false);
    let disabled = off.update(&db).await.unwrap();
    let error = admit(&db, &disabled.id, &headers, Some(&secret), b"{}", now)
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, "BOT_WEBHOOK_DISABLED");
}

#[tokio::test]
async fn a_bot_without_the_capability_cannot_have_or_serve_a_webhook() {
    let db = database().await;
    insert_bot(&db, "bot_plain", "user-a", "SERVER").await;
    let created = create_in(
        &db,
        "user-a",
        "bot_plain",
        "x".into(),
        String::new(),
        hash_secret("s"),
    )
    .await;
    assert_code(
        &created.unwrap_err(),
        StatusCode::FORBIDDEN,
        "BOT_WEBHOOK_CAPABILITY_OFF",
    );

    hooked_bot(&db, "bot_a", "user-a").await;
    let hook = hook(&db, "bot_a", "whsec_good").await;
    let row = bot::Entity::find_by_id("bot_a")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut row: bot::ActiveModel = row.into();
    row.capabilities = Set(Default::default());
    row.update(&db).await.unwrap();
    let served = admit(
        &db,
        &hook.id,
        &secret_header("whsec_good"),
        None,
        b"{}",
        Instant::now(),
    )
    .await;
    assert_code(
        &served.unwrap_err(),
        StatusCode::FORBIDDEN,
        "BOT_WEBHOOK_CAPABILITY_OFF",
    );
}

#[tokio::test]
async fn only_the_owner_creates_and_a_bot_has_at_most_ten() {
    let db = database().await;
    hooked_bot(&db, "bot_a", "user-a").await;
    let stranger = create_in(
        &db,
        "user-b",
        "bot_a",
        "x".into(),
        String::new(),
        hash_secret("s"),
    )
    .await;
    assert_eq!(stranger.unwrap_err().status(), StatusCode::NOT_FOUND);
    for index in 0..10 {
        create_in(
            &db,
            "user-a",
            "bot_a",
            format!("hook {index}"),
            String::new(),
            hash_secret("s"),
        )
        .await
        .unwrap();
    }
    let eleventh = create_in(
        &db,
        "user-a",
        "bot_a",
        "one more".into(),
        String::new(),
        hash_secret("s"),
    )
    .await;
    assert_code(
        &eleventh.unwrap_err(),
        StatusCode::CONFLICT,
        "BOT_WEBHOOK_LIMIT_REACHED",
    );
}

#[tokio::test]
async fn too_many_calls_or_too_many_refusals_are_answered_with_429() {
    let db = database().await;
    hooked_bot(&db, "bot_a", "user-a").await;
    let hook = hook(&db, "bot_a", "whsec_good").await;
    let now = Instant::now();
    for _ in 0..30 {
        admit(
            &db,
            &hook.id,
            &secret_header("whsec_good"),
            None,
            b"{}",
            now,
        )
        .await
        .unwrap();
    }
    let throttled = admit(
        &db,
        &hook.id,
        &secret_header("whsec_good"),
        None,
        b"{}",
        now,
    )
    .await;
    assert_eq!(
        throttled.unwrap_err().status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    let other = self::hook(&db, "bot_a", "whsec_other").await;
    for _ in 0..10 {
        let refused = admit(&db, &other.id, &secret_header("nope"), None, b"{}", now).await;
        assert_eq!(refused.unwrap_err().status(), StatusCode::UNAUTHORIZED);
    }
    let blocked = admit(
        &db,
        &other.id,
        &secret_header("whsec_other"),
        None,
        b"{}",
        now,
    )
    .await;
    assert_eq!(blocked.unwrap_err().status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rotating_invalidates_the_old_secret_and_the_old_signing_key() {
    let db = database().await;
    hooked_bot(&db, "bot_a", "user-a").await;
    let hook = hook(&db, "bot_a", "whsec_old").await;
    let old_key = hook.secret_hash.clone();

    let stranger = rotate_in(&db, "user-b", "bot_a", &hook.id).await;
    assert_eq!(stranger.unwrap_err().status(), StatusCode::NOT_FOUND);

    let (rotated, secret) = rotate_in(&db, "user-a", "bot_a", &hook.id).await.unwrap();
    let now = Instant::now();
    let old = admit(&db, &hook.id, &secret_header("whsec_old"), None, b"{}", now).await;
    assert_eq!(old.unwrap_err().status(), StatusCode::UNAUTHORIZED);
    let old_signature = admit(&db, &hook.id, &signed(&old_key, b"{}"), None, b"{}", now).await;
    assert_eq!(
        old_signature.unwrap_err().status(),
        StatusCode::UNAUTHORIZED
    );
    admit(&db, &hook.id, &secret_header(&secret), None, b"{}", now)
        .await
        .unwrap();
    admit(
        &db,
        &hook.id,
        &signed(&rotated.secret_hash, b"{}"),
        None,
        b"{}",
        now,
    )
    .await
    .unwrap();
}

#[test]
fn a_call_becomes_a_webhook_trigger_with_the_instruction_and_the_payload() {
    let hook = bot_webhook::Model {
        id: "bh_1".into(),
        bot_id: "bot_a".into(),
        name: "New order".into(),
        secret_hash: String::new(),
        instruction: "Summarize the order".into(),
        enabled: true,
        created_at: at(0),
        last_called_at: None,
    };
    let payload = payload_of(Some("application/json"), br#"{"order": 7}"#);
    let trigger = trigger_for(&hook, Some(payload.clone()), None);
    assert_eq!(trigger.source, TriggerSource::Webhook);
    assert_eq!(
        (trigger.ref_id.as_str(), trigger.title.as_str()),
        ("bh_1", "New order")
    );
    assert_eq!(trigger.text, "Summarize the order");
    assert_eq!(trigger.payload, Some(payload));
    TriggerBody::parse(&trigger.envelope()["body"]).unwrap();
    let outgoing = trigger_outgoing("wh-1", &trigger).unwrap();
    assert_eq!(outgoing.kind, InboxEntryKind::Trigger);
}

#[test]
fn payloads_keep_json_as_json_and_wrap_text_and_binary() {
    assert_eq!(payload_of(None, br#"{"a":1}"#)["body"], json!({"a": 1}));
    assert_eq!(
        payload_of(Some("text/plain"), b"hello")["body"],
        json!("hello")
    );
    assert_eq!(
        payload_of(None, &[0xff, 0xfe])["body"],
        json!({"base64": "//4="})
    );
}

#[test]
fn asking_twice_with_one_request_id_is_one_message_and_two_requests_are_two() {
    let first = ask_message_id("user-a", "bot_a", Some("req-1"));
    assert_eq!(first, ask_message_id("user-a", "bot_a", Some("req-1")));
    assert_ne!(first, ask_message_id("user-a", "bot_a", Some("req-2")));
    assert_ne!(first, ask_message_id("user-a", "bot_b", Some("req-1")));
    assert_ne!(
        ask_message_id("user-a", "bot_a", None),
        ask_message_id("user-a", "bot_a", None)
    );
    assert!(first.len() <= 64);
}

#[test]
fn an_ask_is_an_event_trigger_and_an_empty_one_is_refused() {
    let trigger = ask_trigger("ask-1", "  Check the stock  ").unwrap();
    assert_eq!(trigger.source, TriggerSource::Event);
    assert_eq!(trigger.text, "Check the stock");
    assert!(ask_trigger("ask-1", "   ").is_err());
    assert!(ask_trigger("ask-1", &"x".repeat(8_001)).is_err());
}

#[test]
fn only_a_run_acting_as_a_person_asks_for_that_person() {
    use crate::middleware::jwt::ExecutorUser;
    let run = |sub: &str, shadow: bool, hosted_frontend: bool| ExecutorUser {
        bot: None,
        sub: sub.into(),
        app_id: "app".into(),
        run_id: "run".into(),
        technical_user_id: None,
        app_chain: None,
        correlation: None,
        shadow,
        hosted_frontend,
        event_id: None,
        custom_bits_owner: None,
        unattended: false,
    };
    assert_eq!(run_owner(&run("user-a", false, false)).unwrap(), "user-a");
    assert!(run_owner(&run("sink:abc", false, false)).is_err());
    assert!(run_owner(&run("user-a", true, false)).is_err());
    assert!(run_owner(&run("visitor", false, true)).is_err());
}

async fn queued_turn(db: &DatabaseConnection, bot_id: &str, trigger: &str) -> String {
    let id = turn_id(bot_id, trigger);
    bot_turn::ActiveModel {
        id: Set(id.clone()),
        bot_id: Set(bot_id.into()),
        owner_id: Set("user-a".into()),
        conversation_id: Set("conv".into()),
        trigger_message_id: Set(trigger.into()),
        host_kind: Set("SERVER".into()),
        status: Set("QUEUED".into()),
        attempts: Set(0),
        host_id: Set(None),
        lease_until: Set(None),
        operation_id: Set(None),
        reply_message_id: Set(None),
        error_code: Set(None),
        hops: Set(0),
        requester_id: Set(None),
        audience: Set("OWNER".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
    }
    .insert(db)
    .await
    .unwrap();
    id
}

async fn finish(db: &DatabaseConnection, id: &str, text: &str) {
    inbox_entry::ActiveModel {
        id: Set("entry-1".into()),
        inbox_id: Set("user-a".into()),
        conversation_id: Set("conv".into()),
        message_id: Set("re-1".into()),
        sender_id: Set("bot_a".into()),
        kind: Set("message".into()),
        envelope: Set(json!({"v": 1, "enc": "none", "body": {"text": text}})),
        created_at: Set(at(5)),
        seq: Set(Some(2)),
    }
    .insert(db)
    .await
    .unwrap();
    let turn = bot_turn::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let mut turn: bot_turn::ActiveModel = turn.into();
    turn.status = Set("DONE".into());
    turn.reply_message_id = Set(Some("re-1".into()));
    turn.update(db).await.unwrap();
}

#[tokio::test]
async fn waiting_returns_the_reply_once_the_turn_is_done_and_the_queue_state_on_timeout() {
    let db = database().await;
    let id = queued_turn(&db, "bot_a", "wh-1").await;

    let early = wait_for_turn(&db, "user-a", &id, Duration::from_millis(10))
        .await
        .unwrap();
    assert_eq!(
        (early.status.as_str(), early.reply.is_none()),
        ("queued", true)
    );

    finish(&db, &id, "Order 7 is a bike.").await;
    let done = wait_for_turn(&db, "user-a", &id, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(done.status, "done");
    assert_eq!(done.reply.unwrap().text, "Order 7 is a bike.");

    let stranger = turn_state(&db, "user-b", &id).await.unwrap();
    assert_eq!(
        (stranger.status.as_str(), stranger.reply.is_none()),
        ("queued", true)
    );
}
