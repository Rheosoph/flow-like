use super::{
    apps::attach_in,
    import::{ImportBotRequest, ImportIssuePart, Review, import_in, review},
    model::BotSettings,
    provision::new_bot_row,
    validation::system_defaults,
    workspace::{
        BOT_WORKSPACE_LOCKED, BOT_WORKSPACE_REFUSED, Claim, claim_in, keeps_workspace,
        live_workspace, locked, queue_deletion, refuse_workspaces, upload_bytes, upload_path,
        workspaces_among,
    },
};
use crate::{
    deletion::job::STATUS_QUEUED,
    entity::{
        app, bot, bot_app, deletion_job, membership, meta, role,
        sea_orm_active_enums::{Status, Visibility},
    },
    permission::role_permission::RolePermissions,
    routes::{
        admin::governance::recompute::scored_apps,
        app::{
            fork::begin_online::BundleSummary,
            internal::{
                get_apps::library_apps,
                search_apps::searchable_apps,
                upsert_app::{NewApp, insert_app_rows},
            },
        },
        explore::query::public_app_condition,
        user::bootstrap::member_apps,
    },
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like::{
    bit::Metadata,
    flow::copilot::{BotRunContext, GENERAL_BOT_NAME},
};
use flow_like_types::bot::{BotCapability, BotSystemKind};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QuerySelect, Schema, Select, TransactionTrait,
    sea_query::{ColumnDef, Table, TableCreateStatement},
};
use serde_json::json;
use std::collections::BTreeSet;

/// The entity's table without foreign keys, with the defaults the migrations give the `App`
/// columns a new app leaves to the database.
fn fixture_table(source: &TableCreateStatement) -> TableCreateStatement {
    let mut table = Table::create();
    table.table(source.get_table_name().unwrap().clone());
    for column in source.get_columns() {
        let mut column: ColumnDef = column.clone();
        match column.get_column_name().as_str() {
            "ratingSum" | "ratingCount" | "downloadCount" | "interactionsCount" | "totalSize"
            | "price" => {
                column.default(0);
            }
            "allowForking" => {
                column.default(false);
            }
            "executionMode" => {
                column.default("ANY");
            }
            _ => {}
        }
        table.col(column);
    }
    table
}

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
        schema.create_table_from_entity(bot_app::Entity),
        schema.create_table_from_entity(membership::Entity),
        schema.create_table_from_entity(role::Entity),
        schema.create_table_from_entity(app::Entity),
        schema.create_table_from_entity(meta::Entity),
        schema.create_table_from_entity(deletion_job::Entity),
    ] {
        db.execute(&fixture_table(&source)).await.unwrap();
    }
    for statement in [
        r#"CREATE UNIQUE INDEX public."App_botId_key" ON "App" ("botId")"#,
        r#"CREATE UNIQUE INDEX public."BotApp_pkey" ON "BotApp" ("botId", "appId")"#,
        r#"CREATE UNIQUE INDEX public."Bot_ownerId_importKey_key" ON "Bot" ("ownerId", "importKey")"#,
        r#"CREATE UNIQUE INDEX public."DeletionJob_rootKind_rootId_key" ON "DeletionJob" ("rootKind", "rootId")"#,
        r#"CREATE TABLE public."User" ("id" TEXT PRIMARY KEY, "tier" TEXT NOT NULL)"#,
    ] {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

fn at(seconds: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_800_000_000 + seconds, 0)
        .unwrap()
        .fixed_offset()
}

async fn insert_bot(
    db: &DatabaseConnection,
    owner_id: &str,
    system: Option<BotSystemKind>,
    configure: impl FnOnce(&mut BotSettings),
) -> bot::Model {
    let mut settings = system_defaults(system.unwrap_or(BotSystemKind::General));
    if system.is_none() {
        settings.name = Some("Scout".into());
        settings.capabilities.clear();
    }
    configure(&mut settings);
    new_bot_row(owner_id, system, None, &settings, at(0))
        .unwrap()
        .insert(db)
        .await
        .unwrap()
}

async fn scout(db: &DatabaseConnection, owner_id: &str) -> bot::Model {
    insert_bot(db, owner_id, None, |_| {}).await
}

/// What `create_in` writes once the bot's lock and the owner's account admit the workspace.
async fn make_workspace(db: &DatabaseConnection, owner_id: &str, bot_id: &str) -> String {
    let txn = db.begin().await.unwrap();
    let Claim::New(new_app) = claim_in(&txn, owner_id, bot_id).await.unwrap() else {
        panic!("bot {bot_id} has a workspace already");
    };
    insert_app_rows(&txn, &new_app, at(0)).await.unwrap();
    txn.commit().await.unwrap();
    new_app.id
}

async fn make_app(db: &DatabaseConnection, owner_id: &str, app_id: &str, visibility: Visibility) {
    let txn = db.begin().await.unwrap();
    let new_app = NewApp {
        id: app_id.into(),
        owner_id: owner_id.into(),
        metadata: Metadata::default(),
        language: "en".into(),
        bits: Vec::new(),
        bot_id: None,
    };
    let stored = insert_app_rows(&txn, &new_app, at(0)).await.unwrap();
    let mut row: app::ActiveModel = stored.into();
    row.visibility = Set(visibility);
    row.update(&txn).await.unwrap();
    txn.commit().await.unwrap();
}

async fn claim_error(
    db: &DatabaseConnection,
    owner_id: &str,
    bot_id: &str,
) -> crate::error::ApiError {
    match claim_in(db, owner_id, bot_id).await {
        Err(error) => error,
        Ok(claim) => panic!("claim of {bot_id} for {owner_id} succeeded: {claim:?}"),
    }
}

#[tokio::test]
async fn a_bot_gets_one_workspace_its_owner_alone_belongs_to() {
    let db = database().await;
    let bot = scout(&db, "owner").await;
    let Claim::New(planned) = claim_in(&db, "owner", &bot.id).await.unwrap() else {
        panic!("a new bot has no workspace");
    };
    assert_eq!(planned.bot_id.as_deref(), Some(bot.id.as_str()));
    assert_eq!(planned.owner_id, "owner");
    assert_eq!(planned.metadata.name, "Scout's workspace");
    assert_eq!(
        planned.metadata.description,
        "The files, databases and memory of bot Scout"
    );
    assert_eq!(planned.language, "en");

    let app_id = make_workspace(&db, "owner", &bot.id).await;
    for _ in 0..2 {
        match claim_in(&db, "owner", &bot.id).await.unwrap() {
            Claim::Existing(existing) => assert_eq!(existing, app_id),
            Claim::New(_) => panic!("a second claim makes a second workspace"),
        }
    }
    assert_eq!(
        live_workspace(&db, "owner", &bot.id).await.unwrap(),
        Some(app_id.clone())
    );

    let stored = app::Entity::find_by_id(&app_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.bot_id.as_deref(), Some(bot.id.as_str()));
    assert_eq!(stored.visibility, Visibility::Private);
    assert_eq!(stored.status, Status::Active);
    let members = membership::Entity::find()
        .filter(membership::Column::AppId.eq(&app_id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].user_id, "owner");
    assert_eq!(Some(&members[0].role_id), stored.owner_role_id.as_ref());
    let owner_role = role::Entity::find_by_id(&members[0].role_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner_role.name, "Owner");
    assert_eq!(owner_role.permissions, RolePermissions::Owner.bits());
    let names: BTreeSet<String> = role::Entity::find()
        .filter(role::Column::AppId.eq(&app_id))
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|role| role.name)
        .collect();
    assert_eq!(names, ["Admin", "Owner", "User"].map(String::from).into());
    let meta = meta::Entity::find()
        .filter(meta::Column::AppId.eq(&app_id))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (meta.name.as_str(), meta.lang.as_str()),
        ("Scout's workspace", "en")
    );

    let txn = db.begin().await.unwrap();
    let mut second = planned.clone();
    second.id = "app-second".into();
    assert!(
        insert_app_rows(&txn, &second, at(1)).await.is_err(),
        "the unique App.botId refuses a second workspace"
    );
}

#[tokio::test]
async fn another_persons_or_a_deleted_bot_has_no_workspace_for_the_caller() {
    let db = database().await;
    let bot = scout(&db, "owner").await;
    let app_id = make_workspace(&db, "owner", &bot.id).await;

    let foreign = claim_error(&db, "intruder", &bot.id).await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        foreign.public_message(),
        Some(format!("Bot '{}' not found", bot.id).as_str())
    );
    assert_eq!(
        live_workspace(&db, "intruder", &bot.id).await.unwrap(),
        None
    );

    let mut deleted: bot::ActiveModel = bot.clone().into();
    deleted.deleted_at = Set(Some(at(10)));
    deleted.update(&db).await.unwrap();
    assert_eq!(
        claim_error(&db, "owner", &bot.id).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(live_workspace(&db, "owner", &bot.id).await.unwrap(), None);
    assert!(
        app::Entity::find_by_id(&app_id)
            .one(&db)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn flux_has_no_workspace_but_its_duplicates_and_assistant_do() {
    let db = database().await;
    let flux = insert_bot(&db, "owner", Some(BotSystemKind::Flux), |_| {}).await;
    let refused = claim_error(&db, "owner", &flux.id).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(refused.public_code(), BOT_WORKSPACE_REFUSED);
    assert_eq!(live_workspace(&db, "owner", &flux.id).await.unwrap(), None);

    let duplicate = insert_bot(&db, "owner", None, |settings| {
        settings.name = Some("Flux copy".into());
        settings.capabilities = vec![BotCapability::Flowpilot];
    })
    .await;
    let Claim::New(copy) = claim_in(&db, "owner", &duplicate.id).await.unwrap() else {
        panic!("a duplicate of Flux gets a workspace");
    };
    assert_eq!(copy.metadata.name, "Flux copy's workspace");

    let assistant = insert_bot(&db, "owner", Some(BotSystemKind::General), |_| {}).await;
    let Claim::New(assistant_app) = claim_in(&db, "owner", &assistant.id).await.unwrap() else {
        panic!("Assistant gets a workspace");
    };
    assert_eq!(
        assistant_app.metadata.name,
        format!("{GENERAL_BOT_NAME}'s workspace")
    );
}

#[tokio::test]
async fn deleting_a_bot_queues_its_workspace_in_the_same_transaction() {
    let db = database().await;
    let bot = scout(&db, "owner").await;
    let without = scout(&db, "owner").await;
    let app_id = make_workspace(&db, "owner", &bot.id).await;

    let txn = db.begin().await.unwrap();
    assert!(
        queue_deletion(&txn, "owner", &without.id)
            .await
            .unwrap()
            .is_none()
    );
    queue_deletion(&txn, "owner", &bot.id)
        .await
        .unwrap()
        .unwrap();
    txn.rollback().await.unwrap();
    assert_eq!(deletion_job::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(
        live_workspace(&db, "owner", &bot.id).await.unwrap(),
        Some(app_id.clone()),
        "a rolled back delete leaves the workspace"
    );

    let txn = db.begin().await.unwrap();
    let job = queue_deletion(&txn, "owner", &bot.id)
        .await
        .unwrap()
        .unwrap();
    txn.commit().await.unwrap();
    assert_eq!(
        (
            job.root_kind.as_str(),
            job.root_id.as_str(),
            job.status.as_str()
        ),
        ("App", app_id.as_str(), STATUS_QUEUED)
    );
    assert_eq!(job.requested_by.as_deref(), Some("owner"));
    let tombstoned = app::Entity::find_by_id(&app_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (tombstoned.status, tombstoned.visibility),
        (Status::Inactive, Visibility::Offline)
    );

    assert_eq!(live_workspace(&db, "owner", &bot.id).await.unwrap(), None);
    let deleting = claim_error(&db, "owner", &bot.id).await;
    assert_eq!(deleting.status(), StatusCode::CONFLICT);
    assert_eq!(deleting.public_code(), "CONFLICT");
    assert!(
        deleting.public_message().unwrap().contains("being deleted"),
        "{deleting:?}"
    );

    app::Entity::delete_by_id(&app_id).exec(&db).await.unwrap();
    assert!(matches!(
        claim_in(&db, "owner", &bot.id).await.unwrap(),
        Claim::New(_)
    ));
}

async fn ids(db: &DatabaseConnection, select: Select<app::Entity>) -> BTreeSet<String> {
    select
        .select_only()
        .column(app::Column::Id)
        .into_tuple::<String>()
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

/// `owner` has a private app, a public app and a workspace that even reads as public and active,
/// so only the bot marker can hide it.
async fn listed_fixture(db: &DatabaseConnection) -> String {
    make_app(db, "owner", "app-private", Visibility::Private).await;
    make_app(db, "owner", "app-public", Visibility::Public).await;
    let bot = scout(db, "owner").await;
    let workspace = make_workspace(db, "owner", &bot.id).await;
    let mut row: app::ActiveModel = app::Entity::find_by_id(&workspace)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .into();
    row.visibility = Set(Visibility::Public);
    row.update(db).await.unwrap();
    workspace
}

#[tokio::test]
async fn workspaces_are_hidden_from_every_app_list() {
    let db = database().await;
    let workspace = listed_fixture(&db).await;
    let mine = BTreeSet::from(["app-private".to_string(), "app-public".to_string()]);
    let public = BTreeSet::from(["app-public".to_string()]);

    assert_eq!(ids(&db, library_apps("owner")).await, mine, "library");
    assert_eq!(ids(&db, member_apps("owner")).await, mine, "bootstrap page");
    assert_eq!(
        member_apps("owner").count(&db).await.unwrap(),
        2,
        "bootstrap total"
    );
    assert_eq!(ids(&db, searchable_apps()).await, public, "search");
    assert_eq!(
        ids(&db, app::Entity::find().filter(public_app_condition())).await,
        public,
        "explore"
    );
    let scored: BTreeSet<String> = scored_apps()
        .into_tuple::<String>()
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(scored, mine, "governance recompute");
    assert!(
        app::Entity::find_by_id(&workspace)
            .one(&db)
            .await
            .unwrap()
            .is_some(),
        "the workspace itself stays readable by id"
    );
}

#[tokio::test]
async fn a_workspace_is_refused_wherever_it_would_be_shared() {
    let db = database().await;
    let workspace = listed_fixture(&db).await;

    assert_eq!(
        workspaces_among(&db, ["app-private", workspace.as_str(), "app-unknown"])
            .await
            .unwrap(),
        BTreeSet::from([workspace.clone()])
    );
    assert!(
        workspaces_among(&db, Vec::<String>::new())
            .await
            .unwrap()
            .is_empty()
    );
    refuse_workspaces(&db, ["app-private", "app-public"])
        .await
        .unwrap();
    let refused = refuse_workspaces(&db, [&"app-private".to_string(), &workspace])
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(refused.public_code(), BOT_WORKSPACE_LOCKED);
    assert!(refused.public_message().unwrap().contains(&workspace));
    assert_eq!(
        locked(&workspace).public_message(),
        refused.public_message()
    );
}

#[tokio::test]
async fn no_bot_is_attached_to_a_workspace() {
    let db = database().await;
    let bot = scout(&db, "owner").await;
    let own = make_workspace(&db, "owner", &bot.id).await;
    let other = scout(&db, "owner").await;
    let others = make_workspace(&db, "owner", &other.id).await;
    make_app(&db, "owner", "app-crm", Visibility::Private).await;

    for workspace in [&own, &others] {
        let refused = attach_in(&db, "owner", &bot.id, workspace)
            .await
            .unwrap_err();
        assert_eq!(refused.status(), StatusCode::CONFLICT, "{workspace}");
        assert_eq!(refused.public_code(), BOT_WORKSPACE_LOCKED, "{workspace}");
    }
    assert!(attach_in(&db, "owner", &bot.id, "app-crm").await.unwrap());
    let attached: Vec<String> = bot_app::Entity::find()
        .filter(bot_app::Column::BotId.eq(&bot.id))
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.app_id)
        .collect();
    assert_eq!(attached, ["app-crm"]);
}

fn import_request(app_ids: &[&str]) -> ImportBotRequest {
    serde_json::from_value(json!({
        "local_id": "bot_on_the_desktop",
        "name": "Mover",
        "appearance": { "kind": "pip" },
        "location": "desktop",
        "app_ids": app_ids,
    }))
    .unwrap()
}

#[tokio::test]
async fn an_import_naming_a_workspace_is_refused() {
    let db = database().await;
    let bot = scout(&db, "owner").await;
    let workspace = make_workspace(&db, "owner", &bot.id).await;
    make_app(&db, "owner", "app-crm", Visibility::Private).await;
    let theirs = scout(&db, "someone-else").await;
    let foreign = make_workspace(&db, "someone-else", &theirs.id).await;

    let request = import_request(&["app-crm", workspace.as_str(), foreign.as_str()]);
    let Review::Blocked(issues) = review(&db, "owner", &request).await.unwrap() else {
        panic!("an import naming a workspace is blocked");
    };
    let found: Vec<(ImportIssuePart, &str, Option<&str>)> = issues
        .iter()
        .map(|issue| (issue.part, issue.code.as_str(), issue.ref_id.as_deref()))
        .collect();
    assert_eq!(
        found,
        [
            (
                ImportIssuePart::App,
                BOT_WORKSPACE_LOCKED,
                Some(workspace.as_str())
            ),
            (
                ImportIssuePart::App,
                "BOT_APP_UNAVAILABLE",
                Some(foreign.as_str())
            ),
        ]
    );

    let refused = import_in(&db, "owner", &import_request(&[workspace.as_str()]), at(5))
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(refused.public_code(), BOT_WORKSPACE_LOCKED);
    import_in(&db, "owner", &import_request(&["app-crm"]), at(6))
        .await
        .unwrap();
}

#[test]
fn uploads_stay_within_the_fork_caps_and_the_workspace_root() {
    let summary = |bytes, files| BundleSummary {
        total_size_bytes: bytes,
        total_object_count: files,
    };
    assert_eq!(upload_bytes(&summary(1_000, 10), 1_000, 10).unwrap(), 1_000);
    for (bytes, files) in [(1_001, 10), (1_000, 11)] {
        let refused = upload_bytes(&summary(bytes, files), 1_000, 10).unwrap_err();
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{bytes}/{files}");
    }
    assert_eq!(
        upload_bytes(&summary(u64::MAX, 1), u64::MAX, 10)
            .unwrap_err()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(upload_path("app-1"), "apps/app-1");
}

#[test]
fn only_bots_with_a_row_and_not_flux_work_in_a_workspace() {
    let custom = BotRunContext {
        bot_id: "bot_scout".into(),
        ..BotRunContext::default()
    };
    let flux = BotRunContext {
        bot_id: "bot_flux_row".into(),
        system: Some(BotSystemKind::Flux),
        ..BotRunContext::default()
    };
    let assistant_row = BotRunContext {
        bot_id: "bot_assistant_row".into(),
        system: Some(BotSystemKind::General),
        ..BotRunContext::default()
    };
    assert!(keeps_workspace(&custom));
    assert!(keeps_workspace(&assistant_row));
    assert!(!keeps_workspace(&flux));
    assert!(!keeps_workspace(&BotRunContext::general()));
}

/// The body of the first function after `signature` in `source`.
fn body<'a>(source: &'a str, signature: &str) -> &'a str {
    let body = source
        .split_once(signature)
        .unwrap_or_else(|| panic!("{signature} exists"))
        .1;
    body.split_once("\n}\n").map_or(body, |(body, _)| body)
}

fn position(body: &str, needle: &str, site: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("{site}: '{needle}' is missing"))
}

/// The handlers below only run with the whole API state, so their wiring is read from their
/// source: each refuses a workspace before the first write that would share it.
#[test]
fn every_sharing_path_refuses_a_workspace_before_it_writes() {
    let sites: [(&str, &str, &str, &str, &str); 10] = [
        (
            "change_visibility",
            include_str!("../app/internal/change_visibility.rs"),
            "pub async fn change_visibility(",
            "workspace::locked(&app_id)",
            "transition(&app.visibility",
        ),
        (
            "request_publication",
            include_str!("../app/publication/upsert_publication.rs"),
            "pub async fn request_publication(",
            "refuse_workspaces(&state.db, [&app_id])",
            "new_request(",
        ),
        (
            "add_member",
            include_str!("../app/groups/members.rs"),
            "pub async fn add_member(",
            "[&app_id, &payload.member_app_id]",
            "app_group_member::ActiveModel {",
        ),
        (
            "invite_user",
            include_str!("../app/team/invite_user.rs"),
            "pub async fn invite_user(",
            "workspace::locked(&app_id)",
            "invitation::ActiveModel {",
        ),
        (
            "create_invite_link",
            include_str!("../app/team/create_invite_link.rs"),
            "pub async fn create_invite_link(",
            "workspace::locked(&app_id)",
            "new_link.insert(",
        ),
        (
            "change_forking",
            include_str!("../app/internal/change_forking.rs"),
            "pub async fn change_forking(",
            "workspace::locked(&app_id)",
            "active.update(",
        ),
        (
            "add_connection",
            include_str!("../app/connection/add_connection.rs"),
            "pub async fn add_connection(",
            "[&app_id, &payload.source_app_id]",
            "app_connection::ActiveModel {",
        ),
        (
            "request_connection",
            include_str!("../app/connection/request_connection.rs"),
            "pub async fn request_connection(",
            "[&app_id, &payload.target_app_id]",
            "app_connection::ActiveModel {",
        ),
        (
            "upsert_app_link",
            include_str!("../course/app_links.rs"),
            "pub async fn upsert_app_link(",
            "refuse_workspaces(&state.db, [&body.app_id])",
            "course_app_link::Entity::find_by_id(",
        ),
        (
            "capacity",
            include_str!("../../capacity.rs"),
            "pub async fn set_visibility(",
            "refuse_bot_workspace(txn, app_id).await?;",
            "flow_like_db::coordination::app_capacity(",
        ),
    ];
    for (site, source, signature, guard, write) in sites {
        let handler = body(source, signature);
        assert!(
            position(handler, guard, site) < position(handler, write, site),
            "{site} refuses a workspace before it writes"
        );
    }
    let transfer = body(
        include_str!("../../capacity.rs"),
        "pub async fn transfer_project(",
    );
    assert!(
        position(transfer, "refuse_bot_workspace(txn, app_id)", "transfer")
            < position(transfer, "app_capacity(", "transfer")
    );
}

#[test]
fn deleting_a_bot_queues_its_workspace_inside_its_transaction_and_runs_a_pass_after() {
    let delete = body(include_str!("crud.rs"), "pub async fn delete_bot(");
    let opened = position(delete, ".transaction(", "delete_bot");
    let queued = position(delete, "workspace::queue_deletion(txn", "delete_bot");
    let committed = position(delete, "    .await?;\n", "delete_bot");
    let ran = position(delete, "workspace::delete_queued(&state", "delete_bot");
    assert!(opened < queued && queued < committed && committed < ran);
}

#[test]
fn uploads_get_content_credentials_of_the_bots_own_workspace() {
    let upload = body(
        include_str!("workspace.rs"),
        "pub async fn upload_to_bot_workspace(",
    );
    let steps = [
        "ensure(&state, &sub, &bot_id)",
        "ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles)",
        "capacity::check_storage_write(&state, &app_id, &sub, bytes)",
        "RuntimeCredentials::scoped(&sub, &app_id, &state, CredentialsAccess::EditAppContent)",
        "upload_path: upload_path(&app_id)",
    ];
    let positions: Vec<usize> = steps
        .iter()
        .map(|step| position(upload, step, "upload"))
        .collect();
    assert!(positions.is_sorted(), "{steps:?} run in this order");
}

#[test]
fn the_workspace_endpoints_are_documented_with_their_codes() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    let operation = |path: &str| {
        spec.pointer(&format!(
            "/paths/{}/post",
            path.replace('~', "~0").replace('/', "~1")
        ))
        .unwrap_or_else(|| panic!("POST {path} is documented"))
        .clone()
    };
    let ensure = operation("/bots/{bot_id}/workspace");
    for status in ["200", "201", "404"] {
        assert!(
            ensure.pointer(&format!("/responses/{status}")).is_some(),
            "{status}"
        );
    }
    let conflict = ensure["responses"]["409"]["description"].as_str().unwrap();
    assert!(conflict.contains("BOT_WORKSPACE_REFUSED"), "{conflict}");
    let upload = operation("/bots/{bot_id}/workspace/upload");
    assert_eq!(
        upload
            .pointer("/requestBody/content/application~1json/schema/$ref")
            .and_then(serde_json::Value::as_str),
        Some("#/components/schemas/WorkspaceUploadRequest")
    );
    let deleted = spec
        .pointer("/paths/~1bots~1{bot_id}/delete/responses/204/description")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    for part in ["permission grants", "heartbeats", "summaries", "workspace"] {
        assert!(deleted.contains(part), "{part}: {deleted}");
    }
}

#[test]
fn a_bot_chat_turn_runs_in_its_workspace() {
    let chat_bot = body(include_str!("runtime.rs"), "pub(crate) async fn chat_bot(");
    let loaded = position(chat_bot, "load_chat_bot(", "chat_bot");
    let placed = position(
        chat_bot,
        "bot.context.workspace_app_id =\n            super::workspace::chat_workspace(state, owner_id, &bot.context)",
        "chat_bot",
    );
    assert!(loaded < placed);
}

/// PostgreSQL checks of what SQLite can't show: first uses racing under the bot's lock with commit
/// conflicts injected the way DSQL raises them, and the owner's capacity. Each test makes its own
/// database next to the one the URL names: `FLOW_LIKE_TEST_DATABASE_URL=postgres://… cargo test -p
/// flow-like-api --features db-chaos --lib -- routes::bots::workspace_tests`
#[cfg(feature = "db-chaos")]
mod postgres {
    use super::{at, fixture_table, make_app, make_workspace, scout};
    use crate::{
        capacity,
        db::{DbDialect, RetryPolicy, testing::Chaos},
        entity::{
            app, bot, deletion_job, membership, meta, role, sea_orm_active_enums::Visibility,
        },
        routes::bots::workspace::{BOT_WORKSPACE_LOCKED, Claim, create_in},
    };
    use axum::http::StatusCode;
    use futures::future::join_all;
    use sea_orm::{
        ColumnTrait, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, EntityTrait,
        PaginatorTrait, QueryFilter, Schema, Statement, TransactionTrait,
        sqlx::postgres::{PgConnectOptions, PgPoolOptions},
    };
    use std::{str::FromStr, sync::Arc};

    struct Scratch {
        admin: DatabaseConnection,
        db: DatabaseConnection,
        name: String,
    }

    impl Scratch {
        async fn remove(self) {
            self.db.close().await.unwrap();
            self.admin
                .execute_unprepared(&format!(r#"DROP DATABASE "{}" WITH (FORCE)"#, self.name))
                .await
                .unwrap();
        }
    }

    const TABLES: [&str; 5] = [
        r#"CREATE TABLE "MutationLock" ("id" BIGINT PRIMARY KEY, "owner" TEXT, "expiresAt" TIMESTAMPTZ(3), "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "User" ("id" TEXT PRIMARY KEY, "tier" TEXT NOT NULL)"#,
        r#"CREATE TABLE "ForkJob" ("id" TEXT PRIMARY KEY, "destAppId" TEXT, "policy" JSONB, "status" TEXT)"#,
        r#"CREATE TABLE "FileAccountingObject" ("id" TEXT PRIMARY KEY, "size" BIGINT)"#,
        r#"CREATE UNIQUE INDEX "DeletionJob_rootKind_rootId_key" ON "DeletionJob" ("rootKind", "rootId")"#,
    ];

    async fn postgres() -> Option<Scratch> {
        let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
            eprintln!("skipping bot workspace test: FLOW_LIKE_TEST_DATABASE_URL is unset");
            return None;
        };
        let admin = Database::connect(&url).await.unwrap();
        let name = format!(
            "workspace_{}",
            flow_like_types::create_id().to_ascii_lowercase()
        );
        admin
            .execute_unprepared(&format!(r#"CREATE DATABASE "{name}""#))
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(PgConnectOptions::from_str(&url).unwrap().database(&name))
            .await
            .unwrap();
        let db = DatabaseConnection::from(pool);
        let schema = Schema::new(DatabaseBackend::Postgres);
        for source in [
            schema.create_table_from_entity(bot::Entity),
            schema.create_table_from_entity(app::Entity),
            schema.create_table_from_entity(meta::Entity),
            schema.create_table_from_entity(role::Entity),
            schema.create_table_from_entity(membership::Entity),
            schema.create_table_from_entity(deletion_job::Entity),
        ] {
            db.execute(&fixture_table(&source)).await.unwrap();
        }
        let capacity = include_str!(
            "../../../prisma/migrations/20260913120002_account_capacity/migration.sql"
        );
        for statement in TABLES
            .into_iter()
            .chain(capacity.split(';').map(str::trim))
            .filter(|statement| !statement.is_empty())
        {
            db.execute_unprepared(statement)
                .await
                .unwrap_or_else(|error| panic!("{statement}: {error}"));
        }
        db.execute_unprepared(
            r#"INSERT INTO "User" VALUES ('owner', 'FREE'), ('recipient', 'FREE')"#,
        )
        .await
        .unwrap();
        Some(Scratch { admin, db, name })
    }

    async fn scalar(db: &DatabaseConnection, sql: &str) -> serde_json::Value {
        let row = db
            .query_one_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{sql} found no row"));
        row.try_get::<serde_json::Value>("", "value").unwrap()
    }

    #[tokio::test]
    async fn racing_first_uses_make_one_workspace_that_takes_no_project_slot() {
        let Some(scratch) = postgres().await else {
            return;
        };
        let db = scratch.db.clone();
        let bot = scout(&db, "owner").await;
        let chaos = Arc::new(Chaos::new(None, Some(3)));
        let claims = join_all((0..8).map(|_| {
            let (db, chaos, bot_id) = (db.clone(), chaos.clone(), bot.id.clone());
            async move {
                chaos
                    .transaction(
                        &db,
                        DbDialect::Postgres,
                        None,
                        &RetryPolicy::default(),
                        move |txn| {
                            let bot_id = bot_id.clone();
                            Box::pin(async move { create_in(txn, "owner", &bot_id, at(0)).await })
                        },
                    )
                    .await
                    .unwrap()
            }
        }))
        .await;
        let made: Vec<String> = claims
            .iter()
            .filter_map(|claim| match claim {
                Claim::New(new_app) => Some(new_app.id.clone()),
                Claim::Existing(_) => None,
            })
            .collect();
        assert_eq!(made.len(), 1, "{claims:?}");
        assert!(claims.iter().all(|claim| match claim {
            Claim::Existing(app_id) => *app_id == made[0],
            Claim::New(_) => true,
        }));
        assert!(chaos.injected_conflicts() > 0, "retries ran");
        assert_eq!(
            app::Entity::find()
                .filter(app::Column::BotId.eq(&bot.id))
                .count(&db)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            scalar(
                &db,
                &format!(
                    r#"SELECT json_build_array("payerId", counted, active) AS value FROM "ProjectCapacity" WHERE "appId" = '{}'"#,
                    made[0]
                )
            )
            .await,
            serde_json::json!(["owner", false, true])
        );
        assert_eq!(
            scalar(
                &db,
                r#"SELECT json_build_array("projectCount", initialized) AS value FROM "AccountCapacity" WHERE "payerId" = 'owner'"#
            )
            .await,
            serde_json::json!([0, true])
        );
        scratch.remove().await;
    }

    #[tokio::test]
    async fn the_baseline_counts_a_workspaces_bytes_but_never_a_project() {
        let Some(scratch) = postgres().await else {
            return;
        };
        let db = scratch.db.clone();
        make_app(&db, "owner", "app-plain", Visibility::Private).await;
        let bot = scout(&db, "owner").await;
        let workspace = make_workspace(&db, "owner", &bot.id).await;
        db.execute_unprepared(&format!(
            r#"UPDATE "App" SET "totalSize" = CASE WHEN "id" = 'app-plain' THEN 5 ELSE 7 END WHERE "id" IN ('app-plain', '{workspace}')"#
        ))
        .await
        .unwrap();
        let txn = db.begin().await.unwrap();
        let usage = capacity::lock_account(&txn, "owner").await.unwrap();
        txn.commit().await.unwrap();
        assert_eq!((usage.storage_bytes, usage.project_count), (12, 1));
        assert_eq!(
            scalar(
                &db,
                &format!(
                    r#"SELECT json_build_array(counted, "payerId") AS value FROM "ProjectCapacity" WHERE "appId" = '{workspace}'"#
                )
            )
            .await,
            serde_json::json!([false, "owner"])
        );
        scratch.remove().await;
    }

    #[tokio::test]
    async fn a_workspace_never_changes_visibility_or_payer() {
        let Some(scratch) = postgres().await else {
            return;
        };
        let db = scratch.db.clone();
        make_app(&db, "owner", "app-plain", Visibility::Prototype).await;
        let bot = scout(&db, "owner").await;
        let workspace = make_workspace(&db, "owner", &bot.id).await;
        let txn = db.begin().await.unwrap();
        capacity::lock_account(&txn, "owner").await.unwrap();
        for refused in [
            capacity::set_visibility(&txn, &workspace, &Visibility::Prototype, "FREE", 10)
                .await
                .unwrap_err(),
            capacity::transfer_project(&txn, &workspace, "recipient", "FREE", 10, 1_000)
                .await
                .unwrap_err(),
        ] {
            assert_eq!(refused.status(), StatusCode::CONFLICT);
            assert_eq!(refused.public_code(), BOT_WORKSPACE_LOCKED);
        }
        capacity::set_visibility(&txn, "app-plain", &Visibility::Private, "FREE", 10)
            .await
            .unwrap();
        txn.commit().await.unwrap();
        scratch.remove().await;
    }
}
