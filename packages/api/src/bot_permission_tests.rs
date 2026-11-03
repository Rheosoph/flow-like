use super::*;
use crate::{
    entity::{bot_turn, membership},
    routes::bots::runtime_token::bot_user_for_turn,
    routes::inbox::fixture::{at, delete_bot, insert_bot},
};
use axum::http::StatusCode;
use flow_like_types::bot_effective::ALL_ROLE_BITS;
use flow_like_types::bot_restrictions::BOT_NEVER_BITS;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectOptions, Database, DatabaseConnection, Schema,
    sea_query::Table,
};

const OWNER: &str = "owner";
const BOT: &str = "bot_a";
const APP: &str = "app-1";
const ROOM: &str = "room-1";

async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(bot::Entity),
        schema.create_table_from_entity(role::Entity),
        schema.create_table_from_entity(membership::Entity),
        schema.create_table_from_entity(bot_app::Entity),
        schema.create_table_from_entity(bot_turn::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    db.execute_unprepared(r#"CREATE TABLE public."App" ("id" TEXT PRIMARY KEY, "botId" TEXT)"#)
        .await
        .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."ConversationMember" (
            "conversationId" TEXT NOT NULL, "principalId" TEXT NOT NULL,
            "principalKind" TEXT NOT NULL, "addedBy" TEXT, "joinedAt" TEXT NOT NULL,
            "leftAt" TEXT, "lastReadAt" TEXT,
        "status" TEXT NOT NULL DEFAULT 'ACTIVE', "limits" TEXT, "invitedBy" TEXT, "invitedAt" TEXT,
        "acceptedAt" TEXT, PRIMARY KEY ("conversationId", "principalId"))"#,
    )
    .await
    .unwrap();
    db
}

async fn owner_role(db: &DatabaseConnection, user: &str, app_id: &str, bits: i64) {
    let now = at(0);
    let role_id = format!("role-{user}-{app_id}");
    role::ActiveModel {
        id: Set(role_id.clone()),
        name: Set("Role".into()),
        permissions: Set(bits),
        app_id: Set(Some(app_id.into())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    membership::ActiveModel {
        id: Set(format!("m-{user}-{app_id}")),
        user_id: Set(user.into()),
        app_id: Set(app_id.into()),
        role_id: Set(role_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

async fn attach(db: &DatabaseConnection, app_id: &str, mask: Option<i64>, scopes: &str) {
    let mask = mask.map_or("NULL".to_string(), |m| m.to_string());
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."BotApp" ("botId","appId","addedAt","permissionMask","scopes")
        VALUES ('{BOT}','{app_id}','2023-11-14T00:00:00+00:00',{mask},'{scopes}')"#
    ))
    .await
    .unwrap();
}

async fn join_room(db: &DatabaseConnection, left: bool) {
    let left = if left {
        "'2023-11-15T00:00:00+00:00'"
    } else {
        "NULL"
    };
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."ConversationMember"
        ("conversationId","principalId","principalKind","joinedAt","leftAt")
        VALUES ('{ROOM}','{BOT}','BOT','2023-11-14T00:00:00+00:00',{left})"#
    ))
    .await
    .unwrap();
}

fn bot_user(conv_id: Option<&str>) -> BotUser {
    BotUser {
        bot_id: BOT.into(),
        turn_id: "turn-1".into(),
        conv_id: conv_id.map(str::to_owned),
    }
}

async fn bits(db: &DatabaseConnection, user: &BotUser) -> Result<i64, ApiError> {
    bot_grant_in(db, user, APP)
        .await
        .map(|g| g.permissions.bits())
}

fn edit_bits() -> i64 {
    (RolePermissions::ReadFiles | RolePermissions::WriteFiles | RolePermissions::ExecuteEvents)
        .bits()
}

async fn world(role_bits: i64, mask: Option<i64>) -> DatabaseConnection {
    let db = database().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    owner_role(&db, OWNER, APP, role_bits).await;
    attach(&db, APP, mask, "{}").await;
    db
}

#[test]
fn contract_bits_match_role_permissions() {
    assert_eq!(ALL_ROLE_BITS, RolePermissions::all().bits());
    let never = RolePermissions::Owner
        | RolePermissions::Admin
        | RolePermissions::ReadRoles
        | RolePermissions::WriteMeta
        | RolePermissions::ReadAnalytics
        | RolePermissions::WriteConfig
        | RolePermissions::WriteRoutes;
    assert_eq!(BOT_NEVER_BITS, never.bits());
}

#[tokio::test]
async fn an_unrestricted_bot_holds_the_owners_role() {
    let db = world(edit_bits(), None).await;
    assert_eq!(bits(&db, &bot_user(None)).await.unwrap(), edit_bits());
}

#[tokio::test]
async fn an_admin_owner_never_hands_over_the_superbits() {
    let db = world(RolePermissions::Admin.bits(), None).await;
    let granted = bot_grant_in(&db, &bot_user(None), APP).await.unwrap();
    assert_eq!(granted.permissions.bits() & BOT_NEVER_BITS, 0);
    assert!(granted.permissions.contains(RolePermissions::WriteFiles));
    assert!(!granted.permissions.contains(RolePermissions::Admin));
    assert!(!granted.permissions.contains(RolePermissions::Owner));
}

#[tokio::test]
async fn the_row_mask_narrows_and_never_widens() {
    let read = RolePermissions::ReadFiles.bits();
    let db = world(edit_bits(), Some(read)).await;
    assert_eq!(bits(&db, &bot_user(None)).await.unwrap(), read);

    let db = world(read, Some(edit_bits())).await;
    assert_eq!(bits(&db, &bot_user(None)).await.unwrap(), read);
}

#[tokio::test]
async fn the_response_role_carries_the_narrowed_bits_not_the_owners() {
    let read = RolePermissions::ReadFiles.bits();
    let db = world(edit_bits(), Some(read)).await;
    let granted = bot_grant_in(&db, &bot_user(None), APP).await.unwrap();
    assert_eq!(granted.role.permissions, read);
    assert_eq!(granted.owner_id, OWNER);
    assert_eq!(granted.bot_id, BOT);
}

#[tokio::test]
async fn a_bot_whose_owner_lost_membership_gets_nothing() {
    let db = world(edit_bits(), None).await;
    membership::Entity::delete_many().exec(&db).await.unwrap();
    let refusal = bits(&db, &bot_user(None)).await.unwrap_err();
    assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_deleted_bot_gets_nothing() {
    let db = world(edit_bits(), None).await;
    delete_bot(&db, BOT).await;
    let refusal = bits(&db, &bot_user(None)).await.unwrap_err();
    assert_eq!(refusal.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unattached_app_is_closed_but_the_workspace_is_open() {
    let db = world(edit_bits(), None).await;
    owner_role(&db, OWNER, "app-2", edit_bits()).await;
    let other = bot_grant_in(&db, &bot_user(None), "app-2")
        .await
        .unwrap_err();
    assert_eq!(other.status(), StatusCode::FORBIDDEN);

    db.execute_unprepared(
        r#"INSERT INTO public."App" ("id", "botId") VALUES ('workspace', 'bot_a')"#,
    )
    .await
    .unwrap();
    owner_role(&db, OWNER, "workspace", edit_bits()).await;
    let workspace = bot_grant_in(&db, &bot_user(None), "workspace").await;
    assert_eq!(workspace.unwrap().permissions.bits(), edit_bits());
}

#[tokio::test]
async fn a_restriction_applies_on_the_next_call() {
    let db = world(edit_bits(), None).await;
    assert_eq!(bits(&db, &bot_user(None)).await.unwrap(), edit_bits());
    db.execute_unprepared(&format!(
        r#"UPDATE public."BotApp" SET "permissionMask" = {} WHERE "botId" = '{BOT}'"#,
        RolePermissions::ReadFiles.bits()
    ))
    .await
    .unwrap();
    assert_eq!(
        bits(&db, &bot_user(None)).await.unwrap(),
        RolePermissions::ReadFiles.bits()
    );
}

#[tokio::test]
async fn a_room_turn_needs_an_active_membership() {
    let db = world(edit_bits(), None).await;
    let room = bot_user(Some(ROOM));
    assert_eq!(
        bits(&db, &room).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
    join_room(&db, true).await;
    assert_eq!(
        bits(&db, &room).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
    db.execute_unprepared(r#"UPDATE public."ConversationMember" SET "leftAt" = NULL"#)
        .await
        .unwrap();
    assert_eq!(bits(&db, &room).await.unwrap(), edit_bits());
}

#[tokio::test]
async fn unreadable_scopes_fail_closed() {
    let db = database().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    owner_role(&db, OWNER, APP, edit_bits()).await;
    attach(&db, APP, None, "[1,2]").await;
    assert_eq!(
        bits(&db, &bot_user(None)).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
}

async fn turn(db: &DatabaseConnection, id: &str, owner: &str, audience: &str) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."BotTurn"
        ("id","botId","ownerId","conversationId","triggerMessageId","hostKind","status","attempts",
         "hops","audience","createdAt","updatedAt")
        VALUES ('{id}','{BOT}','{owner}','{ROOM}','m-{id}','SERVER','RUNNING',0,0,'{audience}',
        '2023-11-14T00:00:00+00:00','2023-11-14T00:00:00+00:00')"#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn the_runtime_token_binds_the_owners_own_turn_and_pins_a_room() {
    let db = world(edit_bits(), None).await;
    turn(&db, "t-owner", OWNER, "OWNER").await;
    turn(&db, "t-room", OWNER, "ROOM").await;
    turn(&db, "t-foreign", "someone-else", "OWNER").await;

    let direct = bot_user_for_turn(&db, OWNER, BOT, "t-owner").await.unwrap();
    assert_eq!(
        (direct.turn_id.as_str(), direct.conv_id.clone()),
        ("t-owner", None)
    );
    let room = bot_user_for_turn(&db, OWNER, BOT, "t-room").await.unwrap();
    assert_eq!(room.conv_id.as_deref(), Some(ROOM));

    for (caller, turn_id) in [
        (OWNER, "t-foreign"),
        (OWNER, "missing"),
        ("someone-else", "t-owner"),
    ] {
        let refusal = bot_user_for_turn(&db, caller, BOT, turn_id)
            .await
            .unwrap_err();
        assert_eq!(
            refusal.status(),
            StatusCode::NOT_FOUND,
            "{caller} {turn_id}"
        );
    }
}

#[test]
fn tokens_round_trip_without_a_sub_and_reject_other_token_types() {
    crate::backend_jwt::init_for_tests();
    let user = bot_user(Some(ROOM));
    let signed = sign_bot_token(&user, None).unwrap();
    let claims = verify_bot_token(&signed.token).unwrap();
    assert_eq!(
        (
            claims.bot_id.as_str(),
            claims.turn_id.as_str(),
            claims.conv_id.as_deref()
        ),
        (BOT, "turn-1", Some(ROOM))
    );
    let payload: serde_json::Value = serde_json::to_value(&claims).unwrap();
    assert!(payload.get("sub").is_none());
    assert!(payload.get("owner").is_none());

    let connection =
        crate::app_connection_jwt::sign(crate::app_connection_jwt::AppConnectionJwtParams {
            sub: Some("user".into()),
            origin_app_id: "a".into(),
            target_app_id: "b".into(),
            app_chain: vec![],
            technical_user_id: None,
            run_id: None,
            correlation: None,
            ttl_seconds: None,
        })
        .unwrap();
    assert!(verify_bot_token(&connection).is_err());
    assert!(crate::app_connection_jwt::verify(&signed.token).is_err());
}

#[tokio::test]
async fn the_room_limits_set_on_accept_narrow_the_turn() {
    let db = world(edit_bits(), None).await;
    join_room(&db, false).await;
    let read = RolePermissions::ReadFiles.bits();
    db.execute_unprepared(&format!(
        r#"UPDATE public."ConversationMember" SET "limits" = '{{"mask":{read}}}'"#
    ))
    .await
    .unwrap();
    assert_eq!(bits(&db, &bot_user(Some(ROOM))).await.unwrap(), read);
    assert_eq!(
        bits(&db, &bot_user(None)).await.unwrap(),
        edit_bits(),
        "outside the room the limits do not apply"
    );
}

#[tokio::test]
async fn a_pending_or_declined_invitation_grants_nothing() {
    let db = world(edit_bits(), None).await;
    join_room(&db, false).await;
    for status in ["PENDING", "DECLINED"] {
        db.execute_unprepared(&format!(
            r#"UPDATE public."ConversationMember" SET "status" = '{status}'"#
        ))
        .await
        .unwrap();
        assert_eq!(
            bits(&db, &bot_user(Some(ROOM))).await.unwrap_err().status(),
            StatusCode::FORBIDDEN,
            "{status}"
        );
    }
}

#[tokio::test]
async fn unreadable_room_limits_fail_closed() {
    let db = world(edit_bits(), None).await;
    join_room(&db, false).await;
    db.execute_unprepared(r#"UPDATE public."ConversationMember" SET "limits" = '{"mask":"x"}'"#)
        .await
        .unwrap();
    assert!(bits(&db, &bot_user(Some(ROOM))).await.is_err());
}

fn executor_token_of_bot(conv_id: Option<&str>) -> crate::middleware::jwt::ExecutorUser {
    crate::backend_jwt::init_for_tests();
    let token =
        crate::execution::sign_unattended_execution_jwt(crate::execution::ExecutionJwtParams {
            user_id: OWNER.into(),
            technical_user_id: None,
            run_id: "turn-1".into(),
            app_id: APP.into(),
            board_id: String::new(),
            event_id: None,
            app_chain: None,
            correlation: None,
            callback_url: "https://api.example".into(),
            token_type: crate::execution::TokenType::Executor,
            ttl_seconds: Some(300),
            shadow: None,
        })
        .unwrap();
    let token = crate::execution::bind_bot_identity(&token, BOT, conv_id).unwrap();
    crate::middleware::jwt::ExecutorUser::from(
        crate::execution::verify_execution_jwt(&token).unwrap(),
    )
}

#[tokio::test]
async fn an_executor_token_with_a_bot_claim_resolves_as_the_bot_and_pays_as_the_owner() {
    let db = world(edit_bits(), Some(RolePermissions::ReadFiles.bits())).await;
    let executor = executor_token_of_bot(None);
    assert_eq!(executor.sub, OWNER);
    let bot = executor.bot.expect("bot claim");
    assert_eq!(bot, bot_user(None));
    assert_eq!(
        bits(&db, &bot).await.unwrap(),
        RolePermissions::ReadFiles.bits()
    );
}

#[tokio::test]
async fn an_executor_token_whose_bot_no_longer_holds_the_app_is_refused() {
    let db = world(edit_bits(), None).await;
    let bot = executor_token_of_bot(None).bot.unwrap();
    bits(&db, &bot).await.unwrap();
    db.execute_unprepared(&format!(
        r#"DELETE FROM public."BotApp" WHERE "botId" = '{BOT}'"#
    ))
    .await
    .unwrap();
    assert_eq!(
        bits(&db, &bot).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_room_executor_token_needs_the_bot_to_still_be_in_the_room() {
    let db = world(edit_bits(), None).await;
    join_room(&db, true).await;
    let bot = executor_token_of_bot(Some(ROOM)).bot.unwrap();
    assert_eq!(bot.conv_id.as_deref(), Some(ROOM));
    assert_eq!(
        bits(&db, &bot).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
}

mod flow_runs {
    use super::*;
    use crate::bot_flow::{BotFlowRun, bind_executor, credential_target};
    use crate::credentials::CredentialsAccess;
    use crate::middleware::jwt::{AppUser, ConnectedAppUser};

    const EVENTS_ONLY: &str = r#"{"events":["allowed"]}"#;

    fn full_bits() -> i64 {
        (RolePermissions::ReadFiles
            | RolePermissions::WriteFiles
            | RolePermissions::ReadDatabase
            | RolePermissions::WriteDatabase
            | RolePermissions::ExecuteEvents
            | RolePermissions::ExecuteBoards)
            .bits()
    }

    async fn flow_world(mask: Option<i64>, scopes: &str) -> DatabaseConnection {
        let db = database().await;
        insert_bot(&db, BOT, OWNER, "SERVER").await;
        owner_role(&db, OWNER, APP, full_bits()).await;
        attach(&db, APP, mask, scopes).await;
        db
    }

    async fn admit(
        db: &DatabaseConnection,
        user: &AppUser,
        event: &str,
    ) -> Result<Option<BotFlowRun>, ApiError> {
        BotFlowRun::admit_event_in(db, user, APP, event, false).await
    }

    fn token_user() -> AppUser {
        AppUser::Bot(bot_user(None))
    }

    #[tokio::test]
    async fn a_bot_invokes_an_allowed_event_and_pays_as_its_owner() {
        let db = flow_world(None, EVENTS_ONLY).await;
        let run = admit(&db, &token_user(), "allowed").await.unwrap().unwrap();
        assert_eq!(run.owner_id(), OWNER);
        assert_eq!(run.bot_id(), BOT);
        let (credential_sub, mode) =
            credential_target(Some(&run), OWNER, CredentialsAccess::ServerExecute);
        assert_eq!(credential_sub, BOT);
        assert!(matches!(mode, CredentialsAccess::ServerExecute));
    }

    #[tokio::test]
    async fn a_disallowed_event_is_refused() {
        let db = flow_world(None, EVENTS_ONLY).await;
        let refusal = admit(&db, &token_user(), "other").await.err().unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_empty_allow_list_refuses_every_event() {
        let db = flow_world(None, r#"{"events":[]}"#).await;
        assert!(admit(&db, &token_user(), "allowed").await.is_err());
    }

    #[tokio::test]
    async fn the_board_allow_list_gates_board_runs() {
        let db = flow_world(None, r#"{"boards":["b1"]}"#).await;
        let run = BotFlowRun::resolve_in(
            &db,
            &token_user(),
            APP,
            RolePermissions::ExecuteBoards,
        )
        .await
        .unwrap()
        .unwrap();
        run.ensure_board("b1").unwrap();
        assert!(run.ensure_board("b2").is_err());
    }

    #[tokio::test]
    async fn a_mask_without_execute_refuses_the_run() {
        let db = flow_world(Some(RolePermissions::ReadFiles.bits()), "{}").await;
        let refusal = BotFlowRun::resolve_in(
            &db,
            &token_user(),
            APP,
            RolePermissions::ExecuteEvents,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_restricted_bots_flow_gets_read_only_content_credentials() {
        let mask = (RolePermissions::ReadFiles | RolePermissions::ExecuteEvents).bits();
        let db = flow_world(Some(mask), "{}").await;
        let run = admit(&db, &token_user(), "any").await.unwrap().unwrap();
        assert!(matches!(
            run.credentials_access(),
            CredentialsAccess::ShadowExecute
        ));
        let (sub, mode) = credential_target(Some(&run), OWNER, CredentialsAccess::ServerExecute);
        assert_eq!(sub, BOT);
        assert!(matches!(mode, CredentialsAccess::ShadowExecute));
    }

    #[tokio::test]
    async fn a_person_is_not_a_bot_run() {
        let db = flow_world(None, "{}").await;
        let person = AppUser::OpenID(crate::middleware::jwt::OpenIDUser {
            sub: OWNER.into(),
            access_token: String::new(),
        });
        assert!(admit(&db, &person, "any").await.unwrap().is_none());
    }

    fn connection_of_bot(target: &str) -> AppUser {
        AppUser::ConnectedApp(ConnectedAppUser {
            sub: Some(OWNER.into()),
            origin_app_id: "origin".into(),
            target_app_id: target.into(),
            app_chain: vec!["origin".into()],
            technical_user_id: Some(BOT.into()),
            run_id: Some("run-1".into()),
            correlation: None,
        })
    }

    #[tokio::test]
    async fn the_cross_app_call_is_intersected_with_the_bots_grant_in_the_target() {
        let db = flow_world(None, EVENTS_ONLY).await;
        let call = connection_of_bot(APP);
        assert!(admit(&db, &call, "allowed").await.unwrap().is_some());
        assert!(admit(&db, &call, "other").await.is_err());
    }

    #[tokio::test]
    async fn the_cross_app_call_is_refused_where_the_bot_is_not_attached() {
        let db = database().await;
        insert_bot(&db, BOT, OWNER, "SERVER").await;
        owner_role(&db, OWNER, APP, full_bits()).await;
        let refusal = admit(&db, &connection_of_bot(APP), "any").await.err().unwrap();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_bot_cannot_trigger_a_page_action() {
        let db = flow_world(None, "{}").await;
        for user in [token_user(), connection_of_bot(APP)] {
            let refusal = BotFlowRun::admit_event_in(&db, &user, APP, "any", true)
                .await
                .err()
                .unwrap();
            assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        }
    }

    fn signed_executor_token() -> String {
        crate::backend_jwt::init_for_tests();
        crate::execution::sign_execution_jwt(crate::execution::ExecutionJwtParams {
            user_id: OWNER.into(),
            technical_user_id: Some(BOT.into()),
            run_id: "run-1".into(),
            app_id: APP.into(),
            board_id: "board".into(),
            event_id: Some("allowed".into()),
            app_chain: None,
            correlation: None,
            callback_url: "https://api.example".into(),
            token_type: crate::execution::TokenType::Executor,
            ttl_seconds: Some(300),
            shadow: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn the_run_carries_the_bot_on_its_executor_token_and_bills_the_owner() {
        let db = flow_world(None, EVENTS_ONLY).await;
        join_room(&db, false).await;
        let room = Some(ROOM);
        let run = admit(&db, &AppUser::Bot(bot_user(room)), "allowed")
            .await
            .unwrap();
        let token = bind_executor(run.as_ref(), signed_executor_token()).unwrap();
        let claims = crate::execution::verify_execution_jwt(&token).unwrap();
        assert_eq!(claims.sub, OWNER);
        assert_eq!(claims.bot_id.as_deref(), Some(BOT));
        assert_eq!(claims.conv_id.as_deref(), room);
        assert_eq!(claims.technical_user_id.as_deref(), Some(BOT));
        let executor = crate::middleware::jwt::ExecutorUser::from(claims);
        assert_eq!(
            executor.bot,
            Some(BotUser {
                turn_id: "run-1".into(),
                ..bot_user(room)
            })
        );
    }

    #[tokio::test]
    async fn a_bot_behind_a_connection_stays_on_the_connection_path() {
        let db = flow_world(None, "{}").await;
        let run = admit(&db, &connection_of_bot(APP), "any").await.unwrap();
        let token = bind_executor(run.as_ref(), signed_executor_token()).unwrap();
        let claims = crate::execution::verify_execution_jwt(&token).unwrap();
        assert!(claims.bot_id.is_none());
        assert_eq!(claims.technical_user_id.as_deref(), Some(BOT));
    }

    #[test]
    fn a_person_run_token_is_not_bound() {
        let original = signed_executor_token();
        assert_eq!(bind_executor(None, original.clone()).unwrap(), original);
    }
}
