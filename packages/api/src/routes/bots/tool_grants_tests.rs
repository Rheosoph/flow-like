use super::*;
use crate::{
    middleware::jwt::{BotUser, OpenIDUser},
    routes::inbox::fixture::{database, delete_bot, insert_bot, running_turn},
};
use async_trait::async_trait;
use chrono::Duration;
use flow_like::flow::copilot::tool_grants::{
    GrantKey, HeadlessAnswer, headless_answer, record_card_answer,
};
use flow_like_types::tool_grant::{DENY_PREFIX, ONCE_PREFIX, TOKEN_TTL_HOURS, args_hash};
use sea_orm::DatabaseConnection;
use serde_json::json;

const BOT: &str = "bot_device";
const OWNER: &str = "user-a";
const TURN: &str = "turn_device";
const TOOL: &str = "browser_open";

fn owner() -> AppUser {
    AppUser::OpenID(OpenIDUser {
        sub: OWNER.into(),
        access_token: String::new(),
    })
}

fn runtime(bot: &str, turn: &str) -> AppUser {
    AppUser::Bot(BotUser {
        bot_id: bot.into(),
        turn_id: turn.into(),
        conv_id: None,
    })
}

fn key(scope: &str) -> ToolGrantKey {
    ToolGrantKey {
        tool: TOOL.into(),
        scope: scope.into(),
    }
}

async fn fixture() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotToolGrant" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
            "tool" TEXT NOT NULL, "scope" TEXT NOT NULL, "grantedAt" TEXT NOT NULL,
            "grantedBy" TEXT NOT NULL, "expiresAt" TEXT, UNIQUE ("botId", "tool", "scope"))"#,
    )
    .await
    .unwrap();
    insert_bot(&db, BOT, OWNER, "DEVICE").await;
    insert_bot(&db, "bot_other", OWNER, "DEVICE").await;
    running_turn(
        &db,
        TURN,
        BOT,
        "room",
        "device:one",
        (chrono::Utc::now() + Duration::minutes(5)).fixed_offset(),
    )
    .await;
    db
}

/// Calls the same database operations as the HTTP handlers with a device turn's bot identity.
struct RuntimeGrants {
    db: DatabaseConnection,
    user: AppUser,
}

#[async_trait]
impl GrantStore for RuntimeGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        Ok(list_grants_for(&self.db, &self.user, BOT, true)
            .await
            .map_err(|error| flow_like_types::anyhow!("{error:?}"))?
            .into_iter()
            .map(|grant| GrantKey::new(grant.tool, grant.scope))
            .collect())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        put_grant_for(
            &self.db,
            &self.user,
            BOT,
            &ToolGrantKey {
                tool: tool.into(),
                scope: scope.into(),
            },
        )
        .await
        .map_err(|error| flow_like_types::anyhow!("{error:?}"))?;
        Ok(())
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        remove_grant_for(
            &self.db,
            &self.user,
            BOT,
            &ToolGrantKey {
                tool: tool.into(),
                scope: scope.into(),
            },
        )
        .await
        .map_err(|error| flow_like_types::anyhow!("{error:?}"))
    }
}

#[tokio::test]
async fn device_browser_asks_for_approval_and_consumes_the_owners_answers_once() {
    let db = fixture().await;
    let runtime = RuntimeGrants {
        db: db.clone(),
        user: runtime(BOT, TURN),
    };
    let args = json!({"url":"https://www.autoscout24.de/lst/porsche/panamera"});
    let answer = headless_answer(&runtime, TURN, TOOL, &args, None, "Open AutoScout24").await;
    let HeadlessAnswer::Park(card) = answer else {
        panic!("The device should ask for approval: {answer:?}");
    };
    let owner_grants = GrantRows::new(&db, BOT, OWNER);

    for decision in [ApprovalDecision::Once, ApprovalDecision::Deny] {
        record_card_answer(&owner_grants, &card, decision)
            .await
            .unwrap();
        let answer = headless_answer(&runtime, TURN, TOOL, &args, None, "Open AutoScout24").await;
        match decision {
            ApprovalDecision::Once => assert_eq!(answer, HeadlessAnswer::Run),
            ApprovalDecision::Deny => assert!(matches!(answer, HeadlessAnswer::Refuse(_))),
            ApprovalDecision::Always => unreachable!(),
        }
        let again = headless_answer(&runtime, TURN, TOOL, &args, None, "Open AutoScout24").await;
        assert!(matches!(again, HeadlessAnswer::Park(_)), "{again:?}");
    }
}

#[tokio::test]
async fn a_device_turn_reads_and_touches_but_cannot_create_or_revoke_standing_grants() {
    let db = fixture().await;
    let token = runtime(BOT, TURN);
    let grant = key("browser:autoscout24.de");
    put_grant_for(&db, &owner(), BOT, &grant).await.unwrap();
    assert_eq!(
        list_grants_for(&db, &token, BOT, false)
            .await
            .unwrap()
            .len(),
        1
    );
    touch_grants_for(
        &db,
        &token,
        BOT,
        vec![BotToolGrantKey {
            tool: grant.tool.clone(),
            scope: grant.scope.clone(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        put_grant_for(&db, &token, BOT, &key("*"))
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        remove_grant_for(&db, &token, BOT, &grant)
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(remove_grant_for(&db, &owner(), BOT, &grant).await.unwrap());
}

#[tokio::test]
async fn tool_approvals_reject_foreign_direct_missing_and_expired_turn_tokens() {
    let db = fixture().await;
    let cases = [
        (runtime(BOT, TURN), "bot_other", StatusCode::FORBIDDEN),
        (runtime(BOT, "direct:app"), BOT, StatusCode::FORBIDDEN),
        (runtime(BOT, "missing"), BOT, StatusCode::UNAUTHORIZED),
        (
            runtime("bot_other", TURN),
            "bot_other",
            StatusCode::UNAUTHORIZED,
        ),
    ];
    for (user, bot, expected) in cases {
        assert_eq!(
            list_grants_for(&db, &user, bot, true)
                .await
                .unwrap_err()
                .status(),
            expected
        );
        assert_eq!(
            remove_grant_for(&db, &user, bot, &key("once:answer"))
                .await
                .unwrap_err()
                .status(),
            expected
        );
        assert_eq!(
            touch_grants_for(&db, &user, bot, Vec::new())
                .await
                .unwrap_err()
                .status(),
            expected
        );
    }
    db.execute_unprepared(r#"UPDATE public."BotTurn" SET "status" = 'DONE'"#)
        .await
        .unwrap();
    assert_eq!(
        list_grants_for(&db, &runtime(BOT, TURN), BOT, true)
            .await
            .unwrap_err()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    db.execute_unprepared(r#"UPDATE public."BotTurn" SET "status" = 'RUNNING'"#)
        .await
        .unwrap();
    delete_bot(&db, BOT).await;
    assert_eq!(
        list_grants_for(&db, &runtime(BOT, TURN), BOT, true)
            .await
            .unwrap_err()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn expired_answer_tokens_are_neither_listed_nor_consumed_by_a_device_turn() {
    let db = fixture().await;
    let token = runtime(BOT, TURN);
    let old = now_millis() - Duration::hours(TOKEN_TTL_HOURS + 1);
    for prefix in [ONCE_PREFIX, DENY_PREFIX] {
        let scope = format!(
            "{prefix}{}",
            args_hash(TOOL, &json!({"url":"https://example.com"}))
        );
        approvals::put_in(&db, BOT, OWNER, TOOL, &scope, old)
            .await
            .unwrap();
        assert!(
            list_grants_for(&db, &token, BOT, true)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !remove_grant_for(&db, &token, BOT, &key(&scope))
                .await
                .unwrap()
        );
    }
    assert_eq!(
        remove_grant_for(&db, &token, BOT, &key("once:invalid answer"))
            .await
            .unwrap_err()
            .status(),
        StatusCode::BAD_REQUEST
    );
}
