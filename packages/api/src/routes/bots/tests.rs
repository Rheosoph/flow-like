use super::{
    apps::{BOT_APP_LIMIT_REACHED, BotApp, MAX_APPS_PER_BOT, attach_in, attached_apps},
    bots_by_id,
    clone::{Cloned, clone_in, copy_name},
    crud::{admit_new_bot, custom_bot_count, patch_in, server_bot_count},
    model::{Bot, BotSettings, UpdateBotRequest},
    owned_bot,
    provision::{insert_system_bots, live_bots, new_bot_row},
    validation::{
        BOT_CAPABILITIES_FIXED, BOT_LIMIT_REACHED, BOT_SERVER_LIMIT_REACHED, MAX_CUSTOM_BOTS,
        NAME_MAX_CHARS, system_defaults,
    },
};
use crate::{
    db::DbDialect,
    entity::{bot, bot_app, bot_secret, membership, role},
    error::ApiError,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot::{BotCapability, BotLocation, BotModelPolicy, BotSystemKind},
    data_classification::{DataClassification, DataSensitivity},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database,
    DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Schema, sea_query::Table,
};
use serde_json::json;

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
        schema.create_table_from_entity(bot_secret::Entity),
        schema.create_table_from_entity(membership::Entity),
        schema.create_table_from_entity(role::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    for statement in [
        r#"CREATE UNIQUE INDEX public."Bot_ownerId_systemKey_key" ON "Bot" ("ownerId", "systemKey")"#,
        r#"CREATE UNIQUE INDEX public."BotApp_pkey" ON "BotApp" ("botId", "appId")"#,
        r#"CREATE UNIQUE INDEX public."BotSecret_pkey" ON "BotSecret" ("botId", "name")"#,
        r#"CREATE TABLE public."User" ("id" TEXT PRIMARY KEY, "tier" TEXT NOT NULL)"#,
        r#"CREATE TABLE public."App" ("id" TEXT PRIMARY KEY, "botId" TEXT UNIQUE)"#,
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

async fn custom_bot(
    db: &DatabaseConnection,
    owner_id: &str,
    name: &str,
    created_at: DateTime<FixedOffset>,
) -> bot::Model {
    let mut settings = system_defaults(BotSystemKind::Flux);
    settings.name = Some(name.into());
    settings.capabilities.clear();
    new_bot_row(owner_id, None, None, &settings, created_at)
        .unwrap()
        .insert(db)
        .await
        .unwrap()
}

async fn soft_delete(db: &DatabaseConnection, row: &bot::Model) {
    let mut active: bot::ActiveModel = row.clone().into();
    active.deleted_at = Set(Some(at(10_000)));
    active.update(db).await.unwrap();
}

fn keys(bots: &[bot::Model]) -> Vec<Option<&str>> {
    bots.iter().map(|row| row.system_key.as_deref()).collect()
}

#[tokio::test]
async fn built_in_bots_are_created_once_even_by_concurrent_first_visits() {
    let db = database().await;
    let (first, second) = tokio::join!(
        insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL),
        insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL),
    );
    first.unwrap();
    second.unwrap();
    insert_system_bots(&db, DbDialect::Postgres, "owner", &[BotSystemKind::Flux])
        .await
        .unwrap();

    let bots = live_bots(&db, "owner").await.unwrap();
    assert_eq!(keys(&bots), [Some("general"), Some("flux")]);

    let general = Bot::from(bots[0].clone());
    assert_eq!(general.system, Some(BotSystemKind::General));
    assert_eq!(general.name, None);
    assert_eq!(general.description, None);
    assert_eq!(general.appearance, json!({ "kind": "pip" }));
    assert_eq!(
        general.capabilities,
        [BotCapability::Internet, BotCapability::Memory]
    );
    assert_eq!(general.location, BotLocation::Server);
    assert_eq!(bots[0].location, "SERVER");
    assert!(general.id.starts_with("bot_"));

    let flux = Bot::from(bots[1].clone());
    assert_eq!(flux.system, Some(BotSystemKind::Flux));
    assert_eq!(flux.appearance, json!({ "kind": "flux" }));
    assert_eq!(flux.capabilities, [BotCapability::Flowpilot]);
    assert_eq!(*bots[1].capabilities, ["flowpilot"]);

    let wire = serde_json::to_value(&general).unwrap();
    assert_eq!(wire["system"], "general");
    assert_eq!(wire["name"], serde_json::Value::Null);
    assert_eq!(wire["model"], json!({ "primary": null, "fallbacks": [] }));
    assert_eq!(wire["classification"], json!({ "pii": false }));
    assert_eq!(wire["location"], "server");
    assert_eq!(wire["capabilities"], json!(["internet", "memory"]));
    assert_eq!(wire["embedding_model"], serde_json::Value::Null);
}

#[tokio::test]
async fn built_in_bots_lead_and_custom_bots_follow_oldest_first() {
    let db = database().await;
    let newer = custom_bot(&db, "owner", "Newer", at(20)).await;
    let older = custom_bot(&db, "owner", "Older", at(10)).await;
    let deleted = custom_bot(&db, "owner", "Gone", at(5)).await;
    soft_delete(&db, &deleted).await;
    custom_bot(&db, "someone-else", "Theirs", at(1)).await;
    insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
        .await
        .unwrap();

    let bots = live_bots(&db, "owner").await.unwrap();
    let ids: Vec<&str> = bots.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(keys(&bots), [Some("general"), Some("flux"), None, None]);
    assert_eq!(&ids[2..], [older.id.as_str(), newer.id.as_str()]);
    assert_eq!(custom_bot_count(&db, "owner").await.unwrap(), 2);
    assert_eq!(custom_bot_count(&db, "someone-else").await.unwrap(), 1);
}

#[tokio::test]
async fn a_bot_reads_as_missing_to_everyone_but_its_owner_until_deleted() {
    let db = database().await;
    let row = custom_bot(&db, "owner", "Scout", at(0)).await;

    assert_eq!(owned_bot(&db, "owner", &row.id).await.unwrap().id, row.id);
    let intruder = owned_bot(&db, "intruder", &row.id).await.unwrap_err();
    let unknown = owned_bot(&db, "owner", "bot_unknown").await.unwrap_err();
    for (error, id) in [(&intruder, row.id.as_str()), (&unknown, "bot_unknown")] {
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
        assert_eq!(error.public_code(), unknown.public_code());
        assert_eq!(
            error.public_message(),
            Some(format!("Bot '{id}' not found").as_str())
        );
    }

    soft_delete(&db, &row).await;
    let gone = owned_bot(&db, "owner", &row.id).await.unwrap_err();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    let rendered = bots_by_id(&db, vec![row.id.clone(), "user-1".into()])
        .await
        .unwrap();
    assert_eq!(rendered.len(), 1);
    assert!(rendered[0].deleted_at.is_some());
    assert!(bots_by_id(&db, Vec::new()).await.unwrap().is_empty());
}

/// `owner` is an Editor in `app-1`; only `someone-else` is a member of `app-2`.
async fn seed_memberships(db: &DatabaseConnection) {
    let now = at(0);
    for (id, name, app_id) in [("role-1", "Editor", "app-1"), ("role-2", "Owner", "app-2")] {
        role::ActiveModel {
            id: Set(id.into()),
            name: Set(name.into()),
            permissions: Set(1),
            app_id: Set(Some(app_id.into())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }
    for (id, user_id, app_id, role_id) in [
        ("member-1", "owner", "app-1", "role-1"),
        ("member-2", "someone-else", "app-2", "role-2"),
    ] {
        membership::ActiveModel {
            id: Set(id.into()),
            user_id: Set(user_id.into()),
            app_id: Set(app_id.into()),
            role_id: Set(role_id.into()),
            created_at: Set(now),
            updated_at: Set(now),
            joined_via: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn attached_apps_carry_the_owner_role_until_the_owner_leaves() {
    let db = database().await;
    let row = custom_bot(&db, "owner", "Scout", at(0)).await;
    seed_memberships(&db).await;
    for (app_id, added) in [("app-2", at(20)), ("app-1", at(10))] {
        bot_app::ActiveModel {
            bot_id: Set(row.id.clone()),
            app_id: Set(app_id.into()),
            added_at: Set(added),
        }
        .insert(&db)
        .await
        .unwrap();
    }

    let apps = attached_apps(&db, "owner", &row.id).await.unwrap();
    assert_eq!(
        apps,
        [
            BotApp {
                app_id: "app-1".into(),
                added_at: at(10),
                owner_role: Some("Editor".into()),
            },
            BotApp {
                app_id: "app-2".into(),
                added_at: at(20),
                owner_role: None,
            },
        ]
    );
    assert!(
        attached_apps(&db, "owner", "bot_other")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_bot_uses_at_most_two_hundred_apps() {
    let db = database().await;
    let row = custom_bot(&db, "owner", "Scout", at(0)).await;
    seed_memberships(&db).await;
    membership::ActiveModel {
        id: Set("member-3".into()),
        user_id: Set("owner".into()),
        app_id: Set("app-3".into()),
        role_id: Set("role-1".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        joined_via: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    bot_app::Entity::insert_many((1..MAX_APPS_PER_BOT).map(|n| bot_app::ActiveModel {
        bot_id: Set(row.id.clone()),
        app_id: Set(format!("app-x-{n}")),
        added_at: Set(at(0)),
    }))
    .exec_without_returning(&db)
    .await
    .unwrap();

    assert!(attach_in(&db, "owner", &row.id, "app-1").await.unwrap());
    assert!(
        !attach_in(&db, "owner", &row.id, "app-1").await.unwrap(),
        "attaching an attached app at the limit changes nothing"
    );
    let full = attach_in(&db, "owner", &row.id, "app-3").await.unwrap_err();
    assert_eq!(
        (full.status(), full.public_code()),
        (StatusCode::CONFLICT, BOT_APP_LIMIT_REACHED)
    );
    let foreign = attach_in(&db, "owner", &row.id, "app-2").await.unwrap_err();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
}

/// Concurrent patches are serialized by the bot's `MutationLock` row, which needs `now()` and a
/// table this SQLite harness lacks. This pins what the lock relies on: a patch merges onto the
/// row stored when it runs, so a rename after a revocation keeps the revocation.
#[tokio::test]
async fn a_patch_merges_onto_the_stored_bot() {
    let db = database().await;
    let row = custom_bot(&db, "owner", "Scout", at(0)).await;
    let capabilities = |ids: &[&str]| UpdateBotRequest {
        capabilities: Some(ids.iter().map(|id| id.to_string()).collect()),
        ..Default::default()
    };
    let rename = UpdateBotRequest {
        name: Some(Some("Ada".into())),
        ..Default::default()
    };

    let granted = patch_in(&db, "owner", &row.id, capabilities(&["internet"]))
        .await
        .unwrap();
    assert_eq!(granted.changed, ["capabilities"]);
    patch_in(&db, "owner", &row.id, capabilities(&[]))
        .await
        .unwrap();
    let renamed = patch_in(&db, "owner", &row.id, rename.clone())
        .await
        .unwrap();
    assert_eq!(renamed.changed, ["name"]);
    let settings = BotSettings::from_row(&renamed.bot);
    assert_eq!(settings.name.as_deref(), Some("Ada"));
    assert!(
        settings.capabilities.is_empty(),
        "the rename keeps the revocation"
    );

    let again = patch_in(&db, "owner", &row.id, rename).await.unwrap();
    assert!(again.changed.is_empty());
    assert_eq!(again.bot.updated_at, renamed.bot.updated_at);
    let intruder = patch_in(&db, "intruder", &row.id, UpdateBotRequest::default())
        .await
        .unwrap_err();
    assert_eq!(intruder.status(), StatusCode::NOT_FOUND);
}

#[test]
fn stored_rows_read_back_as_the_settings_that_wrote_them() {
    let mut settings = system_defaults(BotSystemKind::General);
    settings.name = Some("Ada".into());
    settings.capabilities = vec![BotCapability::Webhook, BotCapability::ComputerUse];
    settings.location = BotLocation::Desktop;
    settings.model.primary = Some("model-a".into());
    let mut row = <bot::ActiveModel as Default>::default();
    settings.write(&mut row).unwrap();

    let now = at(0);
    let model = bot::Model {
        id: "bot_1".into(),
        owner_id: "owner".into(),
        system_key: None,
        preset_id: None,
        name: row.name.unwrap(),
        description: row.description.unwrap(),
        instructions: row.instructions.unwrap(),
        appearance: row.appearance.unwrap(),
        model_policy: row.model_policy.unwrap(),
        classification: row.classification.unwrap(),
        pii: row.pii.unwrap(),
        capabilities: row.capabilities.unwrap(),
        profile_id: row.profile_id.unwrap(),
        location: row.location.unwrap(),
        device_id: row.device_id.unwrap(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
        import_key: None,
        embedding_model: None,
    };
    assert_eq!(model.location, "DESKTOP");
    assert_eq!(*model.capabilities, ["computer_use", "webhook"]);
    settings.capabilities.sort();
    assert_eq!(BotSettings::from_row(&model), settings);
    assert_eq!(
        BotSettings::from_row(&bot::Model {
            capabilities: vec!["internet".to_string(), "retired_capability".to_string()].into(),
            ..model
        })
        .capabilities,
        [BotCapability::Internet]
    );
}

fn code(error: &ApiError) -> (StatusCode, &str) {
    (error.status(), error.public_code())
}

async fn set_plan(db: &DatabaseConnection, user_id: &str, plan: &str) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."User" ("id", "tier") VALUES ('{user_id}', '{plan}')
            ON CONFLICT ("id") DO UPDATE SET "tier" = excluded."tier""#
    ))
    .await
    .unwrap();
}

/// A bot the owner created, running at `location`.
async fn located_bot(
    db: &DatabaseConnection,
    owner_id: &str,
    location: BotLocation,
    created_at: DateTime<FixedOffset>,
) -> bot::Model {
    let mut settings = system_defaults(BotSystemKind::General);
    settings.name = Some(format!("{} bot", location.as_str()));
    settings.location = location;
    settings.device_id = (location == BotLocation::Device).then(|| "device-1".to_string());
    new_bot_row(owner_id, None, None, &settings, created_at)
        .unwrap()
        .insert(db)
        .await
        .unwrap()
}

async fn bot_count(db: &DatabaseConnection, owner_id: &str) -> u64 {
    bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .count(db)
        .await
        .unwrap()
}

async fn app_ids_of(db: &DatabaseConnection, bot_id: &str) -> Vec<String> {
    let mut ids: Vec<String> = bot_app::Entity::find()
        .filter(bot_app::Column::BotId.eq(bot_id))
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.app_id)
        .collect();
    ids.sort();
    ids
}

fn to(location: BotLocation) -> UpdateBotRequest {
    UpdateBotRequest {
        location: Some(location),
        ..Default::default()
    }
}

/// Counted: the owner's own live bots on the server. Assistant and Flux, deleted bots, bots on
/// a desktop or device and other people's bots never are.
#[tokio::test]
async fn the_plan_limits_bots_on_the_server_only() {
    let db = database().await;
    insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
        .await
        .unwrap();
    for n in 0..3 {
        located_bot(&db, "owner", BotLocation::Server, at(n)).await;
    }
    let gone = located_bot(&db, "owner", BotLocation::Server, at(5)).await;
    soft_delete(&db, &gone).await;
    located_bot(&db, "owner", BotLocation::Desktop, at(6)).await;
    located_bot(&db, "owner", BotLocation::Device, at(7)).await;
    located_bot(&db, "someone-else", BotLocation::Server, at(8)).await;
    assert_eq!(server_bot_count(&db, "owner").await.unwrap(), 3);
    assert_eq!(custom_bot_count(&db, "owner").await.unwrap(), 5);

    let full = admit_new_bot(&db, "owner", BotLocation::Server)
        .await
        .unwrap_err();
    assert_eq!(
        code(&full),
        (StatusCode::CONFLICT, BOT_SERVER_LIMIT_REACHED)
    );
    let message = full.public_message().unwrap();
    assert!(
        message.contains("FREE plan allows 3 bots on the server"),
        "{message}"
    );
    for location in [BotLocation::Desktop, BotLocation::Device] {
        admit_new_bot(&db, "owner", location).await.unwrap();
    }
    admit_new_bot(&db, "someone-else", BotLocation::Server)
        .await
        .unwrap();

    set_plan(&db, "owner", "PREMIUM").await;
    admit_new_bot(&db, "owner", BotLocation::Server)
        .await
        .unwrap();
    set_plan(&db, "owner", "FREE").await;
    assert!(
        admit_new_bot(&db, "owner", BotLocation::Server)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn every_bot_counts_toward_the_safety_cap_whatever_the_plan() {
    let db = database().await;
    set_plan(&db, "owner", "ENTERPRISE").await;
    for n in 0..MAX_CUSTOM_BOTS - 1 {
        located_bot(&db, "owner", BotLocation::Server, at(n as i64)).await;
    }
    admit_new_bot(&db, "owner", BotLocation::Server)
        .await
        .unwrap();
    located_bot(&db, "owner", BotLocation::Desktop, at(100)).await;
    for location in BotLocation::ALL {
        let error = admit_new_bot(&db, "owner", location).await.unwrap_err();
        assert_eq!(
            code(&error),
            (StatusCode::CONFLICT, BOT_LIMIT_REACHED),
            "{location:?}"
        );
    }
}

/// `update_bot` takes the owner's lock before the bot's when the body sets `server`; this pins
/// the check that lock makes exact.
#[tokio::test]
async fn moving_a_bot_to_the_server_needs_a_free_server_slot() {
    let db = database().await;
    insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
        .await
        .unwrap();
    let mut server = Vec::new();
    for n in 0..3 {
        server.push(located_bot(&db, "owner", BotLocation::Server, at(n)).await);
    }
    let desktop = located_bot(&db, "owner", BotLocation::Desktop, at(10)).await;

    let full = patch_in(&db, "owner", &desktop.id, to(BotLocation::Server))
        .await
        .unwrap_err();
    assert_eq!(
        code(&full),
        (StatusCode::CONFLICT, BOT_SERVER_LIMIT_REACHED)
    );
    let stored = owned_bot(&db, "owner", &desktop.id).await.unwrap();
    assert_eq!(stored.location, "DESKTOP");

    let renamed = patch_in(
        &db,
        "owner",
        &server[0].id,
        UpdateBotRequest {
            name: Some(Some("Ada".into())),
            location: Some(BotLocation::Server),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        renamed.changed,
        ["name"],
        "a bot already on the server stays"
    );

    let flux = live_bots(&db, "owner").await.unwrap().remove(1);
    for location in [BotLocation::Desktop, BotLocation::Server] {
        let moved = patch_in(&db, "owner", &flux.id, to(location))
            .await
            .unwrap();
        assert_eq!(moved.changed, ["location"], "built-in bots never count");
    }

    patch_in(&db, "owner", &server[0].id, to(BotLocation::Desktop))
        .await
        .unwrap();
    let moved = patch_in(&db, "owner", &desktop.id, to(BotLocation::Server))
        .await
        .unwrap();
    assert_eq!(moved.changed, ["location"]);
    assert_eq!(server_bot_count(&db, "owner").await.unwrap(), 3);
}

#[tokio::test]
async fn a_duplicate_copies_the_settings_and_the_apps_the_owner_still_uses() {
    let db = database().await;
    seed_memberships(&db).await;
    let settings = BotSettings {
        name: Some("Scout".into()),
        description: Some("Finds things".into()),
        instructions: Some("Cite your sources".into()),
        appearance: json!({ "kind": "pip", "frame": "round" }),
        model: BotModelPolicy {
            primary: Some("model-a".into()),
            fallbacks: vec!["model-b".into()],
        },
        embedding_model: Some("embed-a".into()),
        classification: DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: true,
        },
        capabilities: vec![
            BotCapability::Internet,
            BotCapability::ProfileApps,
            BotCapability::SelectedApps,
            BotCapability::ComputerUse,
            BotCapability::Memory,
        ],
        profile_id: Some("profile-1".into()),
        location: BotLocation::Device,
        device_id: Some("device-1".into()),
    };
    let mut row = new_bot_row("owner", None, Some("researcher".into()), &settings, at(0)).unwrap();
    row.import_key = Set(Some("bot_local".into()));
    let source = row.insert(&db).await.unwrap();
    for (app_id, added) in [("app-2", at(1)), ("app-1", at(2))] {
        bot_app::ActiveModel {
            bot_id: Set(source.id.clone()),
            app_id: Set(app_id.into()),
            added_at: Set(added),
        }
        .insert(&db)
        .await
        .unwrap();
    }
    bot_secret::ActiveModel {
        bot_id: Set(source.id.clone()),
        name: Set("API_TOKEN".into()),
        encryption: Set("server".into()),
        ciphertext: Set("sealed".into()),
        description: Set(None),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        last_used_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();

    let Cloned { bot: copy, app_ids } = clone_in(&db, "owner", &source.id, at(100)).await.unwrap();
    assert!(
        copy.id.starts_with("bot_") && copy.id != source.id,
        "{}",
        copy.id
    );
    assert_eq!(
        (
            copy.owner_id.as_str(),
            copy.system_key.as_deref(),
            copy.import_key.as_deref(),
            copy.preset_id.as_deref()
        ),
        ("owner", None, None, Some("researcher"))
    );
    assert_eq!((copy.created_at, copy.updated_at), (at(100), at(100)));
    assert_eq!(
        BotSettings::from_row(&copy),
        BotSettings {
            name: Some("Scout copy".into()),
            ..settings
        }
    );
    assert_eq!(
        app_ids,
        ["app-1"],
        "only apps the owner is still a member of"
    );
    assert_eq!(app_ids_of(&db, &copy.id).await, ["app-1"]);
    assert_eq!(app_ids_of(&db, &source.id).await, ["app-1", "app-2"]);
    let secrets = bot_secret::Entity::find()
        .filter(bot_secret::Column::BotId.eq(&copy.id))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(secrets, 0, "secrets stay with the original");
}

#[tokio::test]
async fn built_in_bots_duplicate_into_bots_of_your_own_under_their_names() {
    let db = database().await;
    set_plan(&db, "owner", "PRO").await;
    insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
        .await
        .unwrap();
    let bots = live_bots(&db, "owner").await.unwrap();
    let (general, flux) = (&bots[0], &bots[1]);

    let assistant = clone_in(&db, "owner", &general.id, at(1))
        .await
        .unwrap()
        .bot;
    assert_eq!(
        (assistant.name.as_deref(), assistant.system_key.as_deref()),
        (Some("Assistant copy"), None)
    );
    assert_eq!(
        BotSettings::from_row(&assistant).capabilities,
        [BotCapability::Internet, BotCapability::Memory]
    );
    let flux_copy = clone_in(&db, "owner", &flux.id, at(2)).await.unwrap().bot;
    assert_eq!(flux_copy.name.as_deref(), Some("Flux copy"));
    assert_eq!(flux_copy.appearance, json!({ "kind": "flux" }));
    let again = clone_in(&db, "owner", &flux_copy.id, at(3))
        .await
        .unwrap()
        .bot;
    assert_eq!(again.name.as_deref(), Some("Flux copy copy"));
    for copy in [&flux_copy, &again] {
        assert_eq!(
            *copy.capabilities,
            ["flowpilot"],
            "duplicates of Flux hold it"
        );
    }

    patch_in(
        &db,
        "owner",
        &flux.id,
        UpdateBotRequest {
            name: Some(Some("Navigator".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let renamed = clone_in(&db, "owner", &flux.id, at(4)).await.unwrap().bot;
    assert_eq!(renamed.name.as_deref(), Some("Navigator copy"));
    assert_eq!(
        keys(&live_bots(&db, "owner").await.unwrap()),
        [Some("general"), Some("flux"), None, None, None, None]
    );
}

/// Flux rows stored before `flowpilot` existed hold `[]`; they read as holding it, so their
/// duplicates and their runs get it too.
#[tokio::test]
async fn flowpilot_stays_with_flux_and_its_duplicates() {
    let db = database().await;
    let mut early = system_defaults(BotSystemKind::Flux);
    early.capabilities.clear();
    let flux = new_bot_row("owner", Some(BotSystemKind::Flux), None, &early, at(0))
        .unwrap()
        .insert(&db)
        .await
        .unwrap();
    assert!(flux.capabilities.is_empty());
    assert_eq!(
        Bot::from(flux.clone()).capabilities,
        [BotCapability::Flowpilot]
    );
    let fixed = (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITIES_FIXED);
    let capabilities = |ids: &[&str]| UpdateBotRequest {
        capabilities: Some(ids.iter().map(|id| id.to_string()).collect()),
        ..Default::default()
    };
    let same = patch_in(&db, "owner", &flux.id, capabilities(&["flowpilot"]))
        .await
        .unwrap();
    assert!(same.changed.is_empty());
    for ids in [Vec::new(), vec!["flowpilot", "internet"]] {
        let error = patch_in(&db, "owner", &flux.id, capabilities(&ids))
            .await
            .unwrap_err();
        assert_eq!(code(&error), fixed, "{ids:?}");
    }

    let duplicate = clone_in(&db, "owner", &flux.id, at(1)).await.unwrap().bot;
    assert_eq!(*duplicate.capabilities, ["flowpilot"]);
    let error = patch_in(&db, "owner", &duplicate.id, capabilities(&["internet"]))
        .await
        .unwrap_err();
    assert_eq!(code(&error), fixed);
    let moved = patch_in(&db, "owner", &duplicate.id, to(BotLocation::Desktop))
        .await
        .unwrap();
    assert_eq!(
        BotSettings::from_row(&moved.bot).capabilities,
        [BotCapability::Flowpilot]
    );

    let scout = custom_bot(&db, "owner", "Scout", at(2)).await;
    let error = patch_in(&db, "owner", &scout.id, capabilities(&["flowpilot"]))
        .await
        .unwrap_err();
    assert_eq!(code(&error), fixed);
    assert!(
        owned_bot(&db, "owner", &scout.id)
            .await
            .unwrap()
            .capabilities
            .is_empty()
    );
}

#[test]
fn a_copy_s_name_keeps_its_marker_within_the_name_limit() {
    assert_eq!(copy_name("Scout"), "Scout copy");
    let longest = "n".repeat(NAME_MAX_CHARS);
    let copied = copy_name(&longest);
    assert_eq!(copied.chars().count(), NAME_MAX_CHARS);
    assert!(copied.ends_with("n copy"), "{copied}");
    let spaced = format!("{} {}", "n".repeat(NAME_MAX_CHARS - 6), "tail");
    assert_eq!(
        copy_name(&spaced),
        format!("{} copy", "n".repeat(NAME_MAX_CHARS - 6))
    );
    assert_eq!(
        copy_name(&"ü".repeat(NAME_MAX_CHARS)).chars().count(),
        NAME_MAX_CHARS
    );
}

#[tokio::test]
async fn only_the_owner_duplicates_and_the_copy_counts_toward_the_limits() {
    let db = database().await;
    let theirs = located_bot(&db, "someone-else", BotLocation::Desktop, at(0)).await;
    let gone = located_bot(&db, "owner", BotLocation::Desktop, at(1)).await;
    soft_delete(&db, &gone).await;
    for bot_id in [theirs.id.as_str(), gone.id.as_str(), "bot_unknown"] {
        let error = clone_in(&db, "owner", bot_id, at(2)).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{bot_id}");
    }

    let mut server = Vec::new();
    for n in 0..3 {
        server.push(located_bot(&db, "owner", BotLocation::Server, at(10 + n)).await);
    }
    let desktop = located_bot(&db, "owner", BotLocation::Desktop, at(20)).await;
    let before = bot_count(&db, "owner").await;
    let full = clone_in(&db, "owner", &server[0].id, at(30))
        .await
        .unwrap_err();
    assert_eq!(
        code(&full),
        (StatusCode::CONFLICT, BOT_SERVER_LIMIT_REACHED)
    );
    assert_eq!(bot_count(&db, "owner").await, before);
    let copy = clone_in(&db, "owner", &desktop.id, at(31))
        .await
        .unwrap()
        .bot;
    assert_eq!(copy.location, "DESKTOP");

    while custom_bot_count(&db, "owner").await.unwrap() < MAX_CUSTOM_BOTS {
        located_bot(&db, "owner", BotLocation::Desktop, at(40)).await;
    }
    let capped = clone_in(&db, "owner", &desktop.id, at(50))
        .await
        .unwrap_err();
    assert_eq!(code(&capped), (StatusCode::CONFLICT, BOT_LIMIT_REACHED));
}

#[test]
fn the_duplicate_route_and_the_server_bot_limit_are_documented() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    let pointer = |path: &str, method: &str, status: &str| {
        format!(
            "/paths/{}/{method}/responses/{status}",
            path.replace('/', "~1")
        )
    };
    for status in ["201", "401", "403", "404", "409", "422"] {
        assert!(
            spec.pointer(&pointer("/bots/{bot_id}/clone", "post", status))
                .is_some(),
            "{status}"
        );
    }
    for (path, method) in [
        ("/bots", "post"),
        ("/bots/{bot_id}", "patch"),
        ("/bots/import", "post"),
        ("/bots/{bot_id}/clone", "post"),
    ] {
        let conflict = spec
            .pointer(&format!("{}/description", pointer(path, method, "409")))
            .and_then(|description| description.as_str())
            .unwrap_or_default();
        assert!(
            conflict.contains(BOT_SERVER_LIMIT_REACHED),
            "{method} {path}: {conflict}"
        );
    }
}
