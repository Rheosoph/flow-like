use super::{
    apps::attach_in,
    import::{ImportIssuePart, Review, review},
    workspace::BOT_WORKSPACE_LOCKED,
    workspace_tests::{at, database, import_request, scout},
};
use crate::{
    bot_flow::BotFlowRun,
    bot_permission::bot_grant_in,
    error::ApiError,
    middleware::jwt::{AppUser, BotUser, member_role},
    routes::app::internal::{
        change_bot_access::store_bot_access,
        upsert_app::{NewApp, insert_app_rows},
    },
};
use axum::http::StatusCode;
use flow_like::bit::Metadata;
use flow_like_types::bot_restrictions::APP_REFUSES_BOTS;
use sea_orm::{DatabaseConnection, TransactionTrait};

const OWNER: &str = "owner";
const CRM: &str = "app-crm";
const REFUSAL: &str = "The app 'CRM' does not allow bots; ask its owner.";

async fn named_app(db: &DatabaseConnection, app_id: &str, name: &str, bot_id: Option<&str>) {
    let txn = db.begin().await.unwrap();
    let new_app = NewApp {
        id: app_id.into(),
        owner_id: OWNER.into(),
        metadata: Metadata {
            name: name.into(),
            ..Metadata::default()
        },
        language: "en".into(),
        bits: Vec::new(),
        bot_id: bot_id.map(str::to_owned),
    };
    insert_app_rows(&txn, &new_app, at(0)).await.unwrap();
    txn.commit().await.unwrap();
}

fn as_bot(bot_id: &str) -> BotUser {
    BotUser {
        bot_id: bot_id.into(),
        turn_id: "turn-1".into(),
        conv_id: None,
    }
}

fn assert_refused(error: &ApiError, door: &str) {
    assert_eq!(
        (error.status(), error.public_code()),
        (StatusCode::FORBIDDEN, APP_REFUSES_BOTS),
        "{door}"
    );
    assert_eq!(error.public_message(), Some(REFUSAL), "{door}");
}

async fn event_door(db: &DatabaseConnection, bot: &BotUser) -> Result<(), ApiError> {
    BotFlowRun::admit_event_in(db, &AppUser::Bot(bot.clone()), CRM, "event-1", false)
        .await
        .map(|run| assert!(run.is_some(), "a bot's call runs as the bot"))
}

#[tokio::test]
async fn an_app_that_refuses_bots_closes_every_bot_door_until_its_owner_reopens_it() {
    let db = database().await;
    let attached = scout(&db, OWNER).await;
    let latecomer = scout(&db, OWNER).await;
    named_app(&db, CRM, "CRM", None).await;
    assert!(attach_in(&db, OWNER, &attached.id, CRM).await.unwrap());
    let bot = as_bot(&attached.id);
    bot_grant_in(&db, &bot, CRM).await.unwrap();
    event_door(&db, &bot).await.unwrap();

    assert!(store_bot_access(&db, CRM, true).await.unwrap());
    assert!(
        !store_bot_access(&db, CRM, true).await.unwrap(),
        "unchanged"
    );
    assert_refused(
        &bot_grant_in(&db, &bot, CRM).await.unwrap_err(),
        "permission",
    );
    assert_refused(&event_door(&db, &bot).await.unwrap_err(), "event and MCP");
    assert_refused(
        &attach_in(&db, OWNER, &latecomer.id, CRM).await.unwrap_err(),
        "attach",
    );
    let Review::Blocked(issues) = review(&db, OWNER, &import_request(&[CRM])).await.unwrap() else {
        panic!("an import naming an app that refuses bots is blocked");
    };
    let issue = &issues[0];
    assert_eq!(
        (issue.part, issue.code.as_str(), issue.message.as_str()),
        (ImportIssuePart::App, APP_REFUSES_BOTS, REFUSAL)
    );
    assert!(
        member_role(&db, OWNER, CRM).await.unwrap().is_some(),
        "the owner's own role in the app is untouched"
    );

    assert!(store_bot_access(&db, CRM, false).await.unwrap());
    bot_grant_in(&db, &bot, CRM).await.unwrap();
    event_door(&db, &bot).await.unwrap();
    assert!(attach_in(&db, OWNER, &latecomer.id, CRM).await.unwrap());
}

#[tokio::test]
async fn a_bots_workspace_always_admits_its_bot() {
    let db = database().await;
    let bot = scout(&db, OWNER).await;
    named_app(&db, "workspace", "Scout's workspace", Some(&bot.id)).await;
    let locked = store_bot_access(&db, "workspace", true).await.unwrap_err();
    assert_eq!(
        (locked.status(), locked.public_code()),
        (StatusCode::CONFLICT, BOT_WORKSPACE_LOCKED)
    );
    let missing = store_bot_access(&db, "app-gone", true).await.unwrap_err();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

/// The handler needs the whole API state, so its guard is read from its source.
#[test]
fn only_the_apps_owners_decide_whether_bots_may_use_it() {
    let source = include_str!("../app/internal/change_bot_access.rs");
    let handler = source
        .split_once("pub async fn change_bot_access(")
        .expect("the handler exists")
        .1;
    let guard = handler
        .find("ensure_permission!(user, &app_id, &state, RolePermissions::Owner)")
        .expect("the handler checks the Owner role");
    let write = handler
        .find("store_bot_access(&state.db")
        .expect("the handler stores the setting");
    assert!(guard < write, "the Owner check runs before the write");
    assert!(
        include_str!("../app.rs").contains("patch(internal::change_bot_access::change_bot_access)"),
        "the route is mounted"
    );
}
