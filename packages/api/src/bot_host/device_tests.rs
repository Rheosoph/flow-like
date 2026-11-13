use super::{
    approvals::park_in,
    fixture::{
        assert_code, conversation, database, delete_bot, delivery, insert_bot, message, recipient,
    },
    ledger::{
        self, BOT_TURN_HOST_KIND, BOT_TURN_LEASE_LOST, ClaimRequest, Refusal, claim_device_in,
        claim_in, enqueue_in, queued_device_heads,
    },
    worker::{CODE_FAILED, CODE_NO_MODEL, CODE_SETUP, commit_reply_in},
};
use crate::{
    devices::bots::{
        checked_card, checked_reply, device_bot_infos, next_poll_ms, poll_in, release_in,
        silent_in, wait_in,
    },
    entity::bot_turn,
    routes::{
        bots::{
            presence::{ONLINE_WINDOW_SECS, PresenceState, forget_device_in, presence_in},
            turns::user_claimable,
        },
        inbox::now_millis,
    },
};
use axum::http::StatusCode;
use chrono::Duration;
use flow_like::flow::copilot::tool_grants::approval_request;
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{
        STORED_DONE, STORED_FAILED, STORED_QUEUED, STORED_RUNNING, STORED_WAITING, TurnHostKind,
        failure_message_id, reply_message_id, turn_id,
    },
    device_bot::{DeviceReleaseRequest, DeviceReplyRequest, IDLE_POLL_MS, MAX_POLL_MS},
    inbox::ConversationKind,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait};
use serde_json::json;

const OWNER: &str = "user-a";
const OTHER_OWNER: &str = "user-b";
const HOST: &str = "device:boot-1";

fn device_of(bot: &str) -> String {
    format!("device-of-{bot}")
}

async fn bind(db: &DatabaseConnection, bot: &str, device: &str) {
    db.execute_unprepared(&format!(
        r#"UPDATE public."Bot" SET "deviceId" = '{device}' WHERE "id" = '{bot}'"#
    ))
    .await
    .unwrap();
}

async fn queue(db: &DatabaseConnection, bot: &str, owner: &str, conversation: &str, trigger: &str) {
    let mut target = recipient(bot, owner, BotLocation::Device);
    target.conversation_id = conversation.into();
    target.trigger_message_id = trigger.into();
    let queued = enqueue_in(
        db,
        &delivery(ConversationKind::Direct, vec![target]),
        now_millis(),
    )
    .await
    .unwrap();
    assert_eq!(queued, 1);
}

/// Two bots of `user-a` on different devices, each with a queued turn in its own chat, and one
/// bot of `user-b` on its own device.
async fn fleet() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotHostHeartbeat" (
            "botId" TEXT NOT NULL, "hostId" TEXT NOT NULL, "hostKind" TEXT NOT NULL,
            "lastSeenAt" TEXT NOT NULL, PRIMARY KEY ("botId", "hostId"))"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."BotToolGrant" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
            "tool" TEXT NOT NULL, "scope" TEXT NOT NULL, "grantedAt" TEXT NOT NULL,
            "grantedBy" TEXT NOT NULL, "expiresAt" TEXT, UNIQUE ("botId", "tool", "scope"))"#,
    )
    .await
    .unwrap();
    for (bot, owner, chat) in [
        ("bot_a", OWNER, "c_a"),
        ("bot_b", OWNER, "c_b"),
        ("bot_c", OTHER_OWNER, "c_c"),
    ] {
        insert_bot(&db, bot, owner, "DEVICE").await;
        conversation(&db, chat, "DIRECT", "PLAIN", &[owner, bot]).await;
        message(&db, chat, "m1", owner, 1, json!({ "text": "hi" }), &[bot]).await;
        queue(&db, bot, owner, chat, "m1").await;
    }
    db.execute_unprepared(r#"UPDATE public."Conversation" SET "lastSeq" = 1"#)
        .await
        .unwrap();
    db
}

fn request(bot: &str, owner: &str) -> ClaimRequest {
    ClaimRequest {
        turn_id: turn_id(bot, "m1"),
        owner_id: Some(owner.into()),
        host_kind: TurnHostKind::Device,
        host_id: HOST.into(),
        now: now_millis(),
    }
}

async fn claimed(db: &DatabaseConnection, bot: &str) -> bot_turn::Model {
    claim_device_in(db, &device_of(bot), &request(bot, OWNER))
        .await
        .unwrap()
        .unwrap()
        .turn
}

#[tokio::test]
async fn a_poll_lists_only_this_devices_bots_and_turns() {
    let db = fleet().await;
    let polled = poll_in(&db, OWNER, &device_of("bot_a"), HOST, now_millis())
        .await
        .unwrap();
    assert_eq!(
        polled
            .bots
            .iter()
            .map(|bot| bot.id.as_str())
            .collect::<Vec<_>>(),
        ["bot_a"]
    );
    assert_eq!(
        polled
            .turns
            .iter()
            .map(|turn| turn.bot_id.as_str())
            .collect::<Vec<_>>(),
        ["bot_a"]
    );
    assert!(polled.next_poll_ms < IDLE_POLL_MS);
}

#[tokio::test]
async fn a_poll_reports_presence_for_its_own_bots_only() {
    let db = fleet().await;
    poll_in(&db, OWNER, &device_of("bot_a"), HOST, now_millis())
        .await
        .unwrap();
    let presence = presence_in(&db, OWNER, now_millis()).await.unwrap();
    assert_eq!(presence["bot_a"].state, PresenceState::Online);
    assert_eq!(presence["bot_b"].state, PresenceState::Unknown);
    assert_eq!(
        presence_in(&db, OTHER_OWNER, now_millis()).await.unwrap()["bot_c"].state,
        PresenceState::Unknown
    );
    let later = now_millis() + Duration::seconds(ONLINE_WINDOW_SECS + 5);
    assert_eq!(
        presence_in(&db, OWNER, later).await.unwrap()["bot_a"].state,
        PresenceState::Offline
    );
}

#[tokio::test]
async fn another_owners_device_sees_nothing_of_this_owners_bots() {
    let db = fleet().await;
    let polled = poll_in(&db, OTHER_OWNER, &device_of("bot_a"), HOST, now_millis())
        .await
        .unwrap();
    assert!(polled.bots.is_empty() && polled.turns.is_empty());
    assert_eq!(polled.next_poll_ms, IDLE_POLL_MS);
    assert!(
        presence_in(&db, OWNER, now_millis()).await.unwrap()["bot_a"].state
            == PresenceState::Unknown
    );
}

#[tokio::test]
async fn the_device_filter_runs_in_sql_before_the_scan_limit() {
    let db = fleet().await;
    for n in 0..5 {
        queue(&db, "bot_b", OWNER, "c_b", &format!("extra{n}")).await;
    }
    let heads = queued_device_heads(&db, OWNER, &device_of("bot_a"), now_millis(), 100)
        .await
        .unwrap();
    assert_eq!(heads.len(), 1);
    assert_eq!(heads[0].0.bot_id, "bot_a");
}

#[tokio::test]
async fn a_device_cannot_claim_the_turn_of_another_devices_bot() {
    let db = fleet().await;
    let foreign = claim_device_in(&db, &device_of("bot_b"), &request("bot_a", OWNER))
        .await
        .unwrap();
    assert_eq!(foreign.unwrap_err(), Refusal::NotFound);
    let turn = ledger::find(&db, &turn_id("bot_a", "m1"), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, STORED_QUEUED);
}

#[tokio::test]
async fn another_owners_device_cannot_claim_this_owners_turn() {
    let db = fleet().await;
    let foreign = claim_device_in(&db, &device_of("bot_a"), &request("bot_a", OTHER_OWNER))
        .await
        .unwrap();
    assert_eq!(foreign.unwrap_err(), Refusal::NotFound);
}

#[tokio::test]
async fn the_bound_device_claims_and_a_rebound_bot_is_out_of_reach() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    assert_eq!(
        (turn.status.as_str(), turn.host_id.as_deref()),
        (STORED_RUNNING, Some(HOST))
    );
    bind(&db, "bot_b", "device-new").await;
    let stale = claim_device_in(&db, &device_of("bot_b"), &request("bot_b", OWNER))
        .await
        .unwrap();
    assert_eq!(stale.unwrap_err(), Refusal::NotFound);
}

#[tokio::test]
async fn a_deleted_bot_has_no_claimable_turn() {
    let db = fleet().await;
    delete_bot(&db, "bot_a").await;
    let gone = claim_device_in(&db, &device_of("bot_a"), &request("bot_a", OWNER))
        .await
        .unwrap();
    assert_eq!(gone.unwrap_err(), Refusal::NotFound);
    assert!(
        device_bot_infos(&db, OWNER, &device_of("bot_a"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_person_token_cannot_claim_a_device_turn_and_a_desktop_host_cannot_either() {
    assert_code(
        &user_claimable(TurnHostKind::Device, "bt_1").unwrap_err(),
        StatusCode::CONFLICT,
        BOT_TURN_HOST_KIND,
    );
    assert!(user_claimable(TurnHostKind::Desktop, "bt_1").is_ok());
    let db = fleet().await;
    let refusal = claim_in(
        &db,
        &ClaimRequest {
            host_kind: TurnHostKind::Desktop,
            ..request("bot_a", OWNER)
        },
    )
    .await
    .unwrap();
    assert_eq!(
        refusal.unwrap_err(),
        Refusal::HostKind(TurnHostKind::Device)
    );
}

#[tokio::test]
async fn the_next_poll_is_short_while_work_waits_and_never_past_the_presence_window() {
    assert!(next_poll_ms(1) < next_poll_ms(0));
    assert!(next_poll_ms(0) <= MAX_POLL_MS);
}

#[tokio::test]
async fn silent_and_release_need_the_bound_device_and_the_held_lease() {
    let db = fleet().await;
    claimed(&db, "bot_a").await;
    let id = turn_id("bot_a", "m1");
    let wrong_device = silent_in(&db, OWNER, &device_of("bot_b"), &id, HOST, now_millis())
        .await
        .unwrap_err();
    assert_eq!(wrong_device.status(), StatusCode::NOT_FOUND);
    let wrong_host = silent_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        "device:other",
        now_millis(),
    )
    .await
    .unwrap_err();
    assert_code(&wrong_host, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
    let ended = silent_in(&db, OWNER, &device_of("bot_a"), &id, HOST, now_millis())
        .await
        .unwrap();
    assert_eq!(ended.turn.error_code.as_deref(), Some("SILENT"));
}

fn release(host: &str, code: &str, retry: bool) -> DeviceReleaseRequest {
    DeviceReleaseRequest {
        host_id: host.into(),
        error_code: code.into(),
        retry,
        usage: None,
    }
}

#[tokio::test]
async fn a_release_queues_again_with_a_backoff_or_fails_with_the_code() {
    let db = fleet().await;
    let id = turn_id("bot_a", "m1");
    claimed(&db, "bot_a").await;
    let (_, retry_at) = release_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        &release(HOST, CODE_SETUP, true),
        now_millis(),
    )
    .await
    .unwrap();
    let queued = ledger::find(&db, &id, None).await.unwrap().unwrap();
    assert_eq!(queued.status, STORED_QUEUED);
    assert_eq!(
        queued.lease_until.map(|at| at.timestamp()),
        retry_at.map(|at| at.timestamp())
    );
    let later = request("bot_a", OWNER);
    claim_device_in(
        &db,
        &device_of("bot_a"),
        &ClaimRequest {
            now: now_millis() + Duration::seconds(120),
            ..later
        },
    )
    .await
    .unwrap()
    .unwrap();
    let (failed, none) = release_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        &release(HOST, CODE_FAILED, false),
        now_millis() + Duration::seconds(121),
    )
    .await
    .unwrap();
    assert!(none.is_none());
    assert_eq!(failed.turn.error_code.as_deref(), Some(CODE_FAILED));
    let row = ledger::find(&db, &id, None).await.unwrap().unwrap();
    assert_eq!(row.status, STORED_FAILED);
}

#[tokio::test]
async fn a_release_refuses_malformed_codes_and_a_lost_lease() {
    let db = fleet().await;
    let id = turn_id("bot_a", "m1");
    claimed(&db, "bot_a").await;
    let bad = release_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        &release(HOST, "not a code", false),
        now_millis(),
    )
    .await
    .unwrap_err();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let lost = release_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        &release("device:other", CODE_FAILED, false),
        now_millis(),
    )
    .await
    .unwrap_err();
    assert_code(&lost, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
}

fn reply(turn: &bot_turn::Model, message_id: String, failure: Option<&str>) -> DeviceReplyRequest {
    DeviceReplyRequest {
        host_id: HOST.into(),
        message_id,
        envelope: json!({ "v": 1, "enc": "none", "body": { "text": "Done", "reply_to": turn.trigger_message_id } }),
        envelopes: None,
        failure_code: failure.map(str::to_string),
        usage: None,
    }
}

#[tokio::test]
async fn a_reply_must_use_the_turns_answer_or_failure_message_id() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let answer = reply_message_id("bot_a", "m1");
    let failure = failure_message_id("bot_a", "m1");
    assert!(checked_reply(&turn, reply(&turn, answer.clone(), None)).is_ok());
    assert!(checked_reply(&turn, reply(&turn, failure.clone(), Some(CODE_FAILED))).is_ok());
    for bad in [
        reply(&turn, "m-forged".into(), None),
        reply(&turn, failure.clone(), None),
        reply(&turn, answer.clone(), Some(CODE_FAILED)),
        reply(&turn, failure.clone(), Some("MADE_UP")),
    ] {
        assert_eq!(
            checked_reply(&turn, bad).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn a_host_that_found_no_model_reports_it_with_its_own_code() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let failure = failure_message_id("bot_a", "m1");
    let (_, code) = checked_reply(&turn, reply(&turn, failure, Some("BOT_TURN_NO_MODEL"))).unwrap();
    assert_eq!(code, Some(CODE_NO_MODEL));
}

#[tokio::test]
async fn a_reply_completes_the_turn_once_and_a_repeat_returns_the_first_copy() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let (request, failure) =
        checked_reply(&turn, reply(&turn, reply_message_id("bot_a", "m1"), None)).unwrap();
    let first = commit_reply_in(&db, &turn, HOST, request, failure)
        .await
        .unwrap();
    assert!(!first.wake.is_empty());
    let done = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (done.status.as_str(), done.reply_message_id.as_deref()),
        (STORED_DONE, Some(reply_message_id("bot_a", "m1").as_str()))
    );
    let (again, failure) =
        checked_reply(&turn, reply(&turn, reply_message_id("bot_a", "m1"), None)).unwrap();
    let repeat = commit_reply_in(&db, &turn, HOST, again, failure)
        .await
        .unwrap();
    assert!(repeat.wake.is_empty());
    assert_eq!(repeat.entry.message_id, first.entry.message_id);
}

#[tokio::test]
async fn a_reply_without_the_lease_is_refused() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let (request, failure) =
        checked_reply(&turn, reply(&turn, reply_message_id("bot_a", "m1"), None)).unwrap();
    let error = commit_reply_in(&db, &turn, "device:other", request, failure)
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_park_posts_the_card_and_waits_for_the_owner() {
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let card = approval_request(
        &turn.id,
        "knowledge_drop",
        &json!({ "index": "notes" }),
        None,
        "Drops 'notes'.",
    );
    checked_card(&turn, &card).unwrap();
    let mut foreign = card.clone();
    foreign.turn_id = turn_id("bot_b", "m1");
    assert_eq!(
        checked_card(&turn, &foreign).unwrap_err().status(),
        StatusCode::BAD_REQUEST
    );
    park_in(&db, &turn, HOST, &card, &turn.conversation_id, now_millis())
        .await
        .unwrap();
    let parked = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parked.status, STORED_WAITING);
    assert_eq!(parked.reply_message_id.as_deref(), Some(card.id.as_str()));
}

#[tokio::test]
async fn forgetting_removes_only_this_devices_reports() {
    let db = fleet().await;
    bind(&db, "bot_b", &device_of("bot_a")).await;
    poll_in(&db, OWNER, &device_of("bot_a"), HOST, now_millis())
        .await
        .unwrap();
    assert_eq!(
        forget_device_in(&db, OWNER, &device_of("bot_b"), HOST)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        forget_device_in(&db, OWNER, &device_of("bot_a"), HOST)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn the_grants_tokens_are_bound_to_the_turn_and_its_owner() {
    crate::backend_jwt::init_for_tests();
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let token = crate::devices::bots_grant::bot_token(&db, OWNER, "bot_a", &turn.id, 600)
        .await
        .unwrap();
    let claims = crate::bot_permission::verify_bot_token(&token.token).unwrap();
    assert_eq!(
        (claims.bot_id.as_str(), claims.turn_id.as_str()),
        ("bot_a", turn.id.as_str())
    );
    assert!(token.expires_at > chrono::Utc::now().timestamp());
    for (owner, bot, turn_id) in [
        (OTHER_OWNER, "bot_a", turn.id.clone()),
        (OWNER, "bot_b", turn.id.clone()),
        (OWNER, "bot_a", turn_id_of("bot_b")),
    ] {
        let refusal = crate::devices::bots_grant::bot_token(&db, owner, bot, &turn_id, 600)
            .await
            .unwrap_err();
        assert_eq!(refusal.status(), StatusCode::NOT_FOUND, "{owner} {bot}");
    }
    let executor =
        crate::devices::bots_grant::executor_token(OWNER, "bot_a", None, &turn.id, Some("ws"), 600)
            .unwrap();
    let claims = crate::execution::verify_execution_jwt(&executor.token).unwrap();
    assert_eq!(
        (
            claims.sub.as_str(),
            claims.bot_id.as_deref(),
            claims.run_id.as_str()
        ),
        (OWNER, Some("bot_a"), turn.id.as_str())
    );
    assert!(executor.expires_at <= chrono::Utc::now().timestamp() + 600);
}

fn turn_id_of(bot: &str) -> String {
    turn_id(bot, "m1")
}

#[tokio::test]
async fn a_completed_checkpoint_gets_only_delivery_credentials_without_model_or_quota_storage() {
    use crate::{
        bot_host::recovery,
        devices::bots_grant,
        entity::{bot, conversation},
    };
    use flow_like_types::bot_turn::BotRunCheckpoint;
    use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
    crate::backend_jwt::init_for_tests();
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    bot::Entity::update_many()
        .col_expr(bot::Column::ProfileId, Expr::value("missing-profile"))
        .filter(bot::Column::Id.eq("bot_a"))
        .exec(&db)
        .await
        .unwrap();
    let row = bot::Entity::find_by_id("bot_a")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let room = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let claimed = ledger::Claimed {
        turn: turn.clone(),
        batched: Vec::new(),
    };
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
            .await
            .unwrap()
            .is_none()
    );
    let now = now_millis().timestamp_millis();
    let checkpoint = BotRunCheckpoint {
        revision: 1,
        started_at_ms: now - 120_000,
        deadline_at_ms: now - 60_000,
        state: json!({
            "version": 1, "phase": "finished", "original_request": "Finish the task",
            "history": [], "prompt": {"role":"user","content":[{"type":"text","text":"Finished execution."}]},
            "reference_fingerprints": [], "work_plan": {"revision":0,"plan":null}, "steering": [],
            "iteration":1, "model_calls_reserved":1, "max_tool_rounds":128, "plan_step_counter":0,
            "empty_continuations":0, "progress_seen": [],
            "stats": flow_like::flow_like_model_provider::response::LLMUsageStats::default(),
            "web": {}, "delegated_web": {}, "local_app_discovery_returned":false,
            "room_refreshes_remaining":0, "recoveries":0, "approval_resume":false,
            "pending_calls": [], "answer":{"output":"Saved final answer","stop_reason":null}
        }),
    };
    let claim = recovery::Claim {
        turn_id: turn.id.clone(),
        owner_id: OWNER.into(),
        host_id: HOST.into(),
        attempt: turn.attempts as u32,
    };
    recovery::save_in(&db, &claim, &checkpoint, now_millis())
        .await
        .unwrap();
    // This fixture has no profile, model or quota tables. Delivery uses only current inbox access.
    let grant = bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
        .await
        .unwrap()
        .unwrap();
    assert!(grant.delivery_only);
    assert!(grant.usage.is_none() && grant.executor_token.is_none());
    assert!(grant.models.model_id.is_empty() && grant.models.completion.is_null());
    assert!(
        grant.bot.capabilities.is_empty()
            && grant.bot.selected_app_ids.is_empty()
            && grant.bot.workspace_app_id.is_none()
            && grant.bot.skills.is_empty()
    );
    assert_eq!(grant.deadline, grant.bot_token.expires_at);
    let claims = crate::bot_permission::verify_bot_token(&grant.bot_token.token).unwrap();
    assert_eq!(
        (claims.bot_id.as_str(), claims.turn_id.as_str()),
        ("bot_a", turn.id.as_str())
    );
    assert_eq!(claims.exp - claims.iat, bots_grant::DELIVERY_TTL_SECS);
    let (refreshed_bot, executor) = crate::devices::bots::refreshed(
        &db,
        &turn,
        &flow_like_types::device_bot::DeviceHeartbeatRequest {
            host_id: HOST.into(),
            bot_token_expires_at: Some(0),
            executor_token_expires_at: Some(0),
        },
    )
    .await
    .unwrap();
    assert!(
        executor.is_none(),
        "A delivery heartbeat cannot request executor privileges"
    );
    let refreshed_claims =
        crate::bot_permission::verify_bot_token(&refreshed_bot.unwrap().token).unwrap();
    assert_eq!(
        refreshed_claims.exp - refreshed_claims.iat,
        bots_grant::DELIVERY_TTL_SECS
    );
    assert_eq!(
        recovery::load_in(&db, &claim, now_millis())
            .await
            .unwrap()
            .unwrap()
            .deadline_at_ms,
        checkpoint.deadline_at_ms
    );
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, "stale-host", &row, &room)
            .await
            .is_err()
    );
    let mut stale = claimed.clone();
    stale.turn.attempts -= 1;
    assert!(
        bots_grant::completed_delivery_in(&db, &stale, HOST, &row, &room)
            .await
            .is_err()
    );
    ledger::cancel_in(&db, &turn.id, ledger::ERROR_OWNER_CANCELLED, now_millis())
        .await
        .unwrap();
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_bot_host_is_stamped_at_most_once_a_minute() {
    use crate::devices::bots::record_bot_host_seen;
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."ManagedDevice" ("id" TEXT PRIMARY KEY, "botHostSeenAt" BIGINT)"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(r#"INSERT INTO public."ManagedDevice" ("id") VALUES ('d1'), ('d2')"#)
        .await
        .unwrap();
    assert!(record_bot_host_seen(&db, "d1", 1_000_000).await.unwrap());
    assert!(!record_bot_host_seen(&db, "d1", 1_030_000).await.unwrap());
    assert!(record_bot_host_seen(&db, "d1", 1_060_000).await.unwrap());
    assert!(
        !record_bot_host_seen(&db, "missing", 1_060_000)
            .await
            .unwrap()
    );
    let stamped = db
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"SELECT "botHostSeenAt" AS t FROM public."ManagedDevice" WHERE id = 'd2'"#,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stamped.try_get::<Option<i64>>("", "t").unwrap(), None);
}

#[tokio::test]
async fn a_claim_that_cannot_become_a_grant_ends_like_a_server_turn_would() {
    use super::worker::{CODE_QUOTA, reply_request, retries};
    use crate::{devices::bots::claim_failure, error::ApiError};
    let not_hosted = claim_failure(&ApiError::coded(
        StatusCode::BAD_REQUEST,
        crate::devices::bots_grant::BOT_DEVICE_MODEL_NOT_HOSTED,
        "x",
    ));
    assert_eq!(
        (not_hosted.code, not_hosted.post, not_hosted.retryable),
        (CODE_SETUP, true, false)
    );
    assert!(not_hosted.text.contains("hosted model"));
    let quota = claim_failure(&ApiError::payment_required("allowance used up"));
    assert_eq!(
        (quota.code, quota.post, quota.retryable),
        (CODE_QUOTA, true, false)
    );
    assert!(quota.text.contains("allowance used up"));
    let transient = claim_failure(&ApiError::internal("db"));
    // Claim generations include approval handoffs. The ledger limits failure recovery separately.
    assert!(retries(&transient, 1, false) && retries(&transient, 13, false));

    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let request = reply_request(
        &turn,
        failure_message_id("bot_a", "m1"),
        &not_hosted.text,
        Some(not_hosted.code),
    );
    commit_reply_in(&db, &turn, HOST, request, Some(not_hosted.code))
        .await
        .unwrap();
    let ended = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (ended.status.as_str(), ended.error_code.as_deref()),
        (STORED_FAILED, Some(CODE_SETUP))
    );
    let relisted = queued_device_heads(&db, OWNER, &device_of("bot_a"), now_millis(), 100)
        .await
        .unwrap();
    assert!(relisted.is_empty());
}

#[tokio::test]
async fn participant_waits_need_the_bound_device_and_a_live_lease() {
    let db = fleet().await;
    claimed(&db, "bot_a").await;
    let id = turn_id("bot_a", "m1");
    let wrong_device = wait_in(
        &db,
        OWNER,
        &device_of("bot_b"),
        &id,
        HOST,
        OWNER,
        now_millis(),
    )
    .await
    .unwrap_err();
    assert_eq!(wrong_device.status(), StatusCode::NOT_FOUND);
    let wrong_host = wait_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        "device:other",
        OWNER,
        now_millis(),
    )
    .await
    .unwrap_err();
    assert_code(&wrong_host, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
    db.execute_unprepared(
        r#"UPDATE public."Conversation" SET "kind" = 'GROUP' WHERE "id" = 'c_a'"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(r#"UPDATE public."Bot" SET "modelPolicy" = '{"decision":"decision-bit"}' WHERE "id" = 'bot_a'"#)
        .await.unwrap();
    let parked = wait_in(
        &db,
        OWNER,
        &device_of("bot_a"),
        &id,
        HOST,
        OWNER,
        now_millis(),
    )
    .await
    .unwrap();
    assert_eq!(parked.status, "parked");
    let ended = bot_turn::Entity::find_by_id(&id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ended.status, STORED_DONE);
    assert!(ended.lease_until.is_none());
}

#[tokio::test]
async fn a_delivery_token_needs_a_live_bot_that_is_still_in_the_conversation() {
    use crate::{
        bot_host::recovery,
        devices::bots_grant,
        entity::{bot, conversation},
    };
    use flow_like_types::bot_turn::BotRunCheckpoint;
    crate::backend_jwt::init_for_tests();
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let row = bot::Entity::find_by_id("bot_a")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let room = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let claimed = ledger::Claimed {
        turn: turn.clone(),
        batched: Vec::new(),
    };
    let now = now_millis().timestamp_millis();
    let checkpoint = BotRunCheckpoint {
        revision: 1,
        started_at_ms: now - 120_000,
        deadline_at_ms: now - 60_000,
        state: json!({
            "version": 1, "phase": "finished", "original_request": "Finish the task",
            "history": [], "prompt": {"role":"user","content":[{"type":"text","text":"Finished execution."}]},
            "reference_fingerprints": [], "work_plan": {"revision":0,"plan":null}, "steering": [],
            "iteration":1, "model_calls_reserved":1, "max_tool_rounds":128, "plan_step_counter":0,
            "empty_continuations":0, "progress_seen": [],
            "stats": flow_like::flow_like_model_provider::response::LLMUsageStats::default(),
            "web": {}, "delegated_web": {}, "local_app_discovery_returned":false,
            "room_refreshes_remaining":0, "recoveries":0, "approval_resume":false,
            "pending_calls": [], "answer":{"output":"Saved final answer","stop_reason":null}
        }),
    };
    let claim = recovery::Claim {
        turn_id: turn.id.clone(),
        owner_id: OWNER.into(),
        host_id: HOST.into(),
        attempt: turn.attempts as u32,
    };
    recovery::save_in(&db, &claim, &checkpoint, now_millis())
        .await
        .unwrap();
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
            .await
            .unwrap()
            .is_some()
    );
    db.execute_unprepared(
        r#"UPDATE public."ConversationMember" SET "status" = 'LEFT', "leftAt" = '2023-01-01T00:00:00+00:00' WHERE "conversationId" = 'c_a' AND "principalId" = 'bot_a'"#,
    )
    .await
    .unwrap();
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
            .await
            .is_err(),
        "a bot that left the conversation gets no delivery token"
    );
    delete_bot(&db, "bot_a").await;
    assert!(
        bots_grant::completed_delivery_in(&db, &claimed, HOST, &row, &room)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_heartbeat_refreshes_only_the_tokens_that_are_due() {
    use flow_like_types::device_bot::{DeviceHeartbeatRequest, REFRESHED_TOKEN_SECS};
    crate::backend_jwt::init_for_tests();
    let db = fleet().await;
    let turn = claimed(&db, "bot_a").await;
    let beat = |bot_token_expires_at: Option<i64>| DeviceHeartbeatRequest {
        host_id: HOST.into(),
        bot_token_expires_at,
        executor_token_expires_at: None,
    };
    let later = chrono::Utc::now().timestamp() + 3_600;
    let (bot_token, executor) = crate::devices::bots::refreshed(&db, &turn, &beat(Some(later)))
        .await
        .unwrap();
    assert!(bot_token.is_none() && executor.is_none());
    let (bot_token, executor) = crate::devices::bots::refreshed(&db, &turn, &beat(Some(0)))
        .await
        .unwrap();
    assert!(executor.is_none());
    let claims = crate::bot_permission::verify_bot_token(&bot_token.unwrap().token).unwrap();
    assert_eq!(claims.exp - claims.iat, REFRESHED_TOKEN_SECS);
}

#[tokio::test]
async fn only_queued_turns_of_bots_whose_device_is_gone_are_orphaned() {
    use crate::devices::bots_removed::orphaned_in;
    let db = fleet().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."ManagedDevice" (
            "id" TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "status" TEXT NOT NULL)"#,
    )
    .await
    .unwrap();
    // bot_a runs on an active device of its owner, bot_b's device is revoked, and the device
    // bot_c names belongs to someone else.
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."ManagedDevice" ("id", "ownerId", "status") VALUES
            ('{}', '{OWNER}', 'active'), ('{}', '{OWNER}', 'revoked'), ('{}', '{OWNER}', 'active')"#,
        device_of("bot_a"),
        device_of("bot_b"),
        device_of("bot_c"),
    ))
    .await
    .unwrap();
    let listed = |mut turns: Vec<(String, String)>| {
        turns.sort();
        turns
    };
    assert_eq!(
        listed(orphaned_in(&db, 10).await.unwrap()),
        listed(vec![
            (turn_id("bot_b", "m1"), OWNER.to_owned()),
            (turn_id("bot_c", "m1"), OTHER_OWNER.to_owned()),
        ])
    );
    assert_eq!(orphaned_in(&db, 1).await.unwrap().len(), 1);

    claim_in(
        &db,
        &ClaimRequest {
            host_id: "hub:device-removed".into(),
            ..request("bot_b", OWNER)
        },
    )
    .await
    .unwrap()
    .unwrap();
    delete_bot(&db, "bot_c").await;
    assert!(
        orphaned_in(&db, 10).await.unwrap().is_empty(),
        "a claimed turn and a deleted bot's turn are no longer listed"
    );
}

#[tokio::test]
async fn old_hosts_cannot_claim_or_report_docker_bots_online() {
    use crate::devices::bots::{
        BOT_EXECUTION_MODE_UNSUPPORTED, poll_in_modes, require_execution_mode,
    };
    use flow_like_types::{bot::BotExecutionMode, device_bot::DeviceClaimRequest};
    let db = fleet().await;
    db.execute_unprepared(
        r#"UPDATE public."Bot" SET "executionMode" = 'DOCKER' WHERE id = 'bot_a'"#,
    )
    .await
    .unwrap();
    let old: DeviceClaimRequest = serde_json::from_value(json!({"host_id":HOST})).unwrap();
    let poll = poll_in_modes(&db, OWNER, &device_of("bot_a"), HOST, now_millis(), &old)
        .await
        .unwrap();
    assert_eq!(poll.bots[0].execution_mode, BotExecutionMode::Docker);
    assert!(poll.turns.is_empty());
    let error = require_execution_mode(&db, OWNER, "bot_a", &old)
        .await
        .unwrap_err();
    assert_eq!(error.public_code(), BOT_EXECUTION_MODE_UNSUPPORTED);
    let turn = ledger::find(&db, &turn_id("bot_a", "m1"), Some(OWNER))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, STORED_QUEUED);
    assert!(turn.host_id.is_none());
    let docker: DeviceClaimRequest =
        serde_json::from_value(json!({"host_id":HOST,"execution_modes":["docker"]})).unwrap();
    require_execution_mode(&db, OWNER, "bot_a", &docker)
        .await
        .unwrap();
    let poll = poll_in_modes(&db, OWNER, &device_of("bot_a"), HOST, now_millis(), &docker)
        .await
        .unwrap();
    assert_eq!(poll.turns.len(), 1);
    assert!(
        require_execution_mode(&db, OWNER, "bot_b", &docker)
            .await
            .is_err()
    );
    db.execute_unprepared(
        r#"UPDATE public."Bot" SET "executionMode" = 'FUTURE' WHERE id = 'bot_a'"#,
    )
    .await
    .unwrap();
    assert!(
        require_execution_mode(&db, OWNER, "bot_a", &docker)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unsupported_docker_work_cannot_fill_the_native_hosts_scan() {
    let db = fleet().await;
    bind(&db, "bot_b", &device_of("bot_a")).await;
    db.execute_unprepared(
        r#"UPDATE public."Bot" SET "executionMode" = 'DOCKER' WHERE id = 'bot_a'"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(r#"DELETE FROM public."BotTurn" WHERE "botId" = 'bot_b'"#)
        .await
        .unwrap();
    for n in 0..400 {
        queue(&db, "bot_a", OWNER, "c_a", &format!("docker-{n}")).await;
    }
    queue(&db, "bot_b", OWNER, "c_b", "m1").await;

    let poll = poll_in(&db, OWNER, &device_of("bot_a"), HOST, now_millis())
        .await
        .unwrap();
    assert_eq!(poll.bots.len(), 2);
    assert_eq!(poll.turns.len(), 1);
    assert_eq!(poll.turns[0].bot_id, "bot_b");
}

#[tokio::test]
async fn docker_turns_remain_dispatchable_until_the_runtime_is_ready() {
    use crate::devices::bots::poll_in_modes;
    use flow_like_types::device_bot::DevicePollRequest;
    let db = fleet().await;
    db.execute_unprepared(
        r#"UPDATE public."Bot" SET "executionMode" = 'DOCKER' WHERE id = 'bot_a'"#,
    )
    .await
    .unwrap();
    let mut request: DevicePollRequest = serde_json::from_value(json!({
        "host_id":HOST, "execution_modes":["native","docker"], "ready_bot_ids":[]
    }))
    .unwrap();
    let polled = poll_in_modes(
        &db,
        OWNER,
        &device_of("bot_a"),
        HOST,
        now_millis(),
        &request,
    )
    .await
    .unwrap();
    assert_eq!(
        polled.turns.len(),
        1,
        "A queued turn can start its container"
    );
    assert_eq!(
        crate::entity::bot_host_heartbeat::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .len(),
        0
    );
    request.ready_bot_ids = Some(vec!["bot_a".into(), "bot_c".into()]);
    poll_in_modes(
        &db,
        OWNER,
        &device_of("bot_a"),
        HOST,
        now_millis(),
        &request,
    )
    .await
    .unwrap();
    let heartbeats = crate::entity::bot_host_heartbeat::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(
        heartbeats[0].bot_id, "bot_a",
        "Readiness cannot mark another device's bot online"
    );
    request.ready_bot_ids = Some(vec![]);
    poll_in_modes(
        &db,
        OWNER,
        &device_of("bot_a"),
        HOST,
        now_millis(),
        &request,
    )
    .await
    .unwrap();
    assert!(
        crate::entity::bot_host_heartbeat::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .is_empty()
    );
}
