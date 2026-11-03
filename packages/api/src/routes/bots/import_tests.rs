use super::{
    apps::{BOT_APP_LIMIT_REACHED, MAX_APPS_PER_BOT},
    crud::custom_bot_count,
    import::{
        BOT_APP_UNAVAILABLE, BOT_DEVICE_UNAVAILABLE, BOT_INVALID, BOT_PROFILE_UNAVAILABLE,
        ImportBotRequest, ImportIssue, ImportIssuePart, Imported, LOCAL_ID_MAX_CHARS, Review,
        import_in, review,
    },
    model::BotSettings,
    provision::new_bot_row,
    validation::{
        BOT_CAPABILITY_LOCATION, BOT_LIMIT_REACHED, BOT_SERVER_LIMIT_REACHED, MAX_CUSTOM_BOTS,
        system_defaults,
    },
};
use crate::{
    entity::{bot, bot_app, membership},
    error::ApiError,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::bot::{BotCapability, BotLocation, BotSystemKind};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database,
    DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, Schema, sea_query::Table,
};
use serde_json::{Value, json};

/// A local id as the desktop mints them; to the server it is only a key.
const LOCAL_ID: &str = "bot_tz4a98xxat96iws9zmbrgj3a";

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
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    for statement in [
        r#"CREATE UNIQUE INDEX public."BotApp_pkey" ON "BotApp" ("botId", "appId")"#,
        r#"CREATE UNIQUE INDEX public."Bot_ownerId_importKey_key" ON "Bot" ("ownerId", "importKey")"#,
        r#"CREATE TABLE public."Profile" (
            "id" TEXT NOT NULL, "userId" TEXT NOT NULL, "deletedAt" TEXT,
            PRIMARY KEY ("id", "userId"))"#,
        r#"CREATE TABLE public."ManagedDevice" (
            "id" TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "status" TEXT NOT NULL)"#,
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

/// `owner` is a member of `app-1` and `app-2`, has the profile `profile-1`, the active device
/// `device-1` and the revoked device `device-old`; `someone-else` has `profile-2`, `device-2`
/// and is the only member of `app-3`.
async fn seed_account(db: &DatabaseConnection) {
    for (id, user_id, app_id) in [
        ("member-1", "owner", "app-1"),
        ("member-2", "owner", "app-2"),
        ("member-3", "someone-else", "app-3"),
    ] {
        membership::ActiveModel {
            id: Set(id.into()),
            user_id: Set(user_id.into()),
            app_id: Set(app_id.into()),
            role_id: Set("role".into()),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            joined_via: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }
    db.execute_unprepared(
        r#"INSERT INTO public."Profile" ("id", "userId", "deletedAt") VALUES
            ('profile-1', 'owner', NULL), ('profile-2', 'someone-else', NULL),
            ('profile-gone', 'owner', '2027-01-01T00:00:00+00:00')"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"INSERT INTO public."ManagedDevice" ("id", "ownerId", "status") VALUES
            ('device-1', 'owner', 'active'), ('device-old', 'owner', 'revoked'),
            ('device-2', 'someone-else', 'active')"#,
    )
    .await
    .unwrap();
}

/// A local bot as the desktop sends it, with `fields` on top.
fn request(local_id: &str, fields: Value) -> ImportBotRequest {
    let mut body = json!({
        "local_id": local_id,
        "name": "Scout",
        "appearance": { "kind": "pip" },
        "location": "desktop",
    });
    if let (Value::Object(body), Value::Object(fields)) = (&mut body, fields) {
        body.extend(fields);
    }
    serde_json::from_value(body).unwrap()
}

fn issues(review: Review) -> Vec<(ImportIssuePart, String, Option<String>)> {
    review
        .into_issues()
        .into_iter()
        .map(|found| (found.part, found.code, found.ref_id))
        .collect()
}

fn issue(
    part: ImportIssuePart,
    code: &str,
    ref_id: Option<&str>,
) -> (ImportIssuePart, String, Option<String>) {
    (part, code.to_string(), ref_id.map(str::to_string))
}

fn code(error: &ApiError) -> (StatusCode, &str) {
    (error.status(), error.public_code())
}

async fn import(db: &DatabaseConnection, request: &ImportBotRequest) -> Result<Imported, ApiError> {
    import_in(db, "owner", request, at(1_000)).await
}

async fn app_ids(db: &DatabaseConnection, bot_id: &str) -> Vec<String> {
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

async fn stored_bot(db: &DatabaseConnection, id: &str) -> bot::Model {
    bot::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .expect("the bot row stays")
}

async fn custom_bot(db: &DatabaseConnection, owner_id: &str) -> bot::Model {
    let mut settings = system_defaults(BotSystemKind::Flux);
    settings.name = Some("Existing".into());
    settings.capabilities.clear();
    new_bot_row(owner_id, None, None, &settings, at(0))
        .unwrap()
        .insert(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_clean_bot_passes_the_check_and_moves_under_a_minted_id_with_its_apps() {
    let db = database().await;
    seed_account(&db).await;
    let local = request(
        LOCAL_ID,
        json!({
            "preset_id": "researcher",
            "capabilities": ["computer_use", "selected_apps", "profile_apps"],
            "profile_id": " profile-1 ",
            "app_ids": ["app-2", " app-1 ", "app-2", ""],
            "created_at": "2027-01-15T08:00:00Z",
        }),
    );
    assert!(issues(review(&db, "owner", &local).await.unwrap()).is_empty());

    let Imported::Created(stored) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };
    assert!(stored.id.starts_with("bot_"), "{}", stored.id);
    assert_ne!(stored.id, LOCAL_ID, "the server mints the id");
    assert_eq!(
        (
            stored.owner_id.as_str(),
            stored.system_key.as_deref(),
            stored.import_key.as_deref()
        ),
        ("owner", None, Some(LOCAL_ID))
    );
    assert_eq!(stored.preset_id.as_deref(), Some("researcher"));
    assert_eq!((stored.created_at, stored.updated_at), (at(0), at(1_000)));
    let settings = BotSettings::from_row(&stored);
    assert_eq!(settings.location, BotLocation::Desktop);
    assert_eq!(settings.profile_id.as_deref(), Some("profile-1"));
    assert_eq!(
        settings.capabilities,
        [
            BotCapability::ProfileApps,
            BotCapability::SelectedApps,
            BotCapability::ComputerUse
        ]
    );
    assert_eq!(app_ids(&db, &stored.id).await, ["app-1", "app-2"]);
    assert!(app_ids(&db, LOCAL_ID).await.is_empty());
}

#[tokio::test]
async fn importing_again_returns_the_bot_unchanged() {
    let db = database().await;
    seed_account(&db).await;
    let local = request(LOCAL_ID, json!({ "app_ids": ["app-1"] }));
    let Imported::Created(first) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };

    let renamed = request(LOCAL_ID, json!({ "name": "Renamed", "app_ids": ["app-2"] }));
    assert_eq!(
        review(&db, "owner", &renamed).await.unwrap(),
        Review::Imported(first.clone())
    );
    assert_eq!(
        import(&db, &renamed).await.unwrap(),
        Imported::Existing(first.clone())
    );
    assert_eq!(app_ids(&db, &first.id).await, ["app-1"]);
    assert_eq!(bot::Entity::find().count(&db).await.unwrap(), 1);
}

/// A local id is only a key of its owner: another person's bot with the same key, or a bot
/// whose id equals it, never stands in the way.
#[tokio::test]
async fn import_keys_belong_to_their_owner_and_are_never_ids() {
    let db = database().await;
    let Imported::Created(theirs) = import_in(
        &db,
        "someone-else",
        &request(LOCAL_ID, json!({})),
        at(1_000),
    )
    .await
    .unwrap() else {
        panic!("a new bot is created");
    };
    let assistant = new_bot_row(
        "owner",
        Some(BotSystemKind::General),
        None,
        &system_defaults(BotSystemKind::General),
        at(0),
    )
    .unwrap()
    .insert(&db)
    .await
    .unwrap();

    for local_id in [LOCAL_ID, theirs.id.as_str(), assistant.id.as_str()] {
        let local = request(local_id, json!({}));
        assert!(
            matches!(
                review(&db, "owner", &local).await.unwrap(),
                Review::Ready(_)
            ),
            "{local_id}"
        );
        let Imported::Created(mine) = import(&db, &local).await.unwrap() else {
            panic!("a new bot is created for {local_id}");
        };
        assert_eq!(mine.import_key.as_deref(), Some(local_id));
        assert!(
            ![theirs.id.as_str(), assistant.id.as_str()].contains(&mine.id.as_str()),
            "{local_id}"
        );
    }
    let still_theirs = bot::Entity::find_by_id(theirs.id.as_str())
        .one(&db)
        .await
        .unwrap();
    assert_eq!(still_theirs, Some(theirs));
}

#[tokio::test]
async fn a_deleted_bot_gives_its_import_key_to_the_next_move() {
    let db = database().await;
    let local = request(LOCAL_ID, json!({}));
    let Imported::Created(first) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };
    let mut deleted: bot::ActiveModel = first.clone().into();
    deleted.deleted_at = Set(Some(at(5)));
    deleted.update(&db).await.unwrap();

    assert!(matches!(
        review(&db, "owner", &local).await.unwrap(),
        Review::Ready(_)
    ));
    assert_eq!(
        stored_bot(&db, &first.id).await.import_key.as_deref(),
        Some(LOCAL_ID),
        "checking writes nothing"
    );

    let Imported::Created(second) = import(&db, &local).await.unwrap() else {
        panic!("a new bot replaces the deleted one");
    };
    assert_ne!(second.id, first.id);
    assert_eq!(second.import_key.as_deref(), Some(LOCAL_ID));
    let old = stored_bot(&db, &first.id).await;
    assert_eq!((old.import_key, old.deleted_at), (None, Some(at(5))));
    assert_eq!(
        import(&db, &local).await.unwrap(),
        Imported::Existing(second)
    );
}

#[tokio::test]
async fn a_local_id_has_one_to_128_characters() {
    let db = database().await;
    for malformed in [String::new(), "k".repeat(LOCAL_ID_MAX_CHARS + 1)] {
        let local = request(&malformed, json!({}));
        let error = review(&db, "owner", &local).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{malformed:?}");
        let error = import(&db, &local).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{malformed:?}");
    }
    assert_eq!(bot::Entity::find().count(&db).await.unwrap(), 0);

    for opaque in [
        "k".to_string(),
        " ".to_string(),
        "ü".repeat(LOCAL_ID_MAX_CHARS),
        "BOT_TZ4A98XXAT96IWS9ZMBRGJ3A".to_string(),
    ] {
        assert!(
            matches!(
                review(&db, "owner", &request(&opaque, json!({})))
                    .await
                    .unwrap(),
                Review::Ready(_)
            ),
            "{opaque:?}"
        );
    }
}

#[tokio::test]
async fn the_owner_s_own_bots_count_toward_the_limit() {
    let db = database().await;
    for _ in 0..MAX_CUSTOM_BOTS {
        custom_bot(&db, "owner").await;
    }
    custom_bot(&db, "someone-else").await;
    new_bot_row(
        "owner",
        Some(BotSystemKind::Flux),
        None,
        &system_defaults(BotSystemKind::Flux),
        at(0),
    )
    .unwrap()
    .insert(&db)
    .await
    .unwrap();

    let local = request(LOCAL_ID, json!({}));
    assert_eq!(
        issues(review(&db, "owner", &local).await.unwrap()),
        [issue(ImportIssuePart::Bot, BOT_LIMIT_REACHED, None)]
    );
    let error = import(&db, &local).await.unwrap_err();
    assert_eq!(code(&error), (StatusCode::CONFLICT, BOT_LIMIT_REACHED));
}

/// Creating a bot can't switch `flowpilot` on, but a duplicate of Flux made on the device keeps it
/// when it moves in.
#[tokio::test]
async fn a_duplicate_of_flux_moves_in_with_flowpilot() {
    let db = database().await;
    let local = request(
        LOCAL_ID,
        json!({ "name": "Flux copy", "capabilities": ["flowpilot"] }),
    );
    assert!(issues(review(&db, "owner", &local).await.unwrap()).is_empty());
    let Imported::Created(stored) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };
    assert_eq!(
        BotSettings::from_row(&stored).capabilities,
        [BotCapability::Flowpilot]
    );
}

#[tokio::test]
async fn a_moved_bot_keeps_its_memory_and_embedding_model() {
    let db = database().await;
    let local = request(
        LOCAL_ID,
        json!({ "capabilities": ["memory"], "embedding_model": "embed-a" }),
    );
    let Imported::Created(stored) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };
    let settings = BotSettings::from_row(&stored);
    assert_eq!(settings.capabilities, [BotCapability::Memory]);
    assert_eq!(settings.embedding_model.as_deref(), Some("embed-a"));
}

/// `custom_bot` rows run on the server; FREE (no account row yet) allows three of them.
#[tokio::test]
async fn a_bot_moves_onto_the_server_only_within_the_plan() {
    let db = database().await;
    for _ in 0..3 {
        custom_bot(&db, "owner").await;
    }
    let server = request(LOCAL_ID, json!({ "location": "server" }));
    assert_eq!(
        issues(review(&db, "owner", &server).await.unwrap()),
        [issue(ImportIssuePart::Bot, BOT_SERVER_LIMIT_REACHED, None)]
    );
    let error = import(&db, &server).await.unwrap_err();
    assert_eq!(
        code(&error),
        (StatusCode::CONFLICT, BOT_SERVER_LIMIT_REACHED)
    );
    assert!(
        error
            .public_message()
            .unwrap()
            .contains("FREE plan allows 3")
    );
    assert!(matches!(
        review(&db, "owner", &request(LOCAL_ID, json!({})))
            .await
            .unwrap(),
        Review::Ready(_)
    ));

    db.execute_unprepared(r#"INSERT INTO public."User" ("id", "tier") VALUES ('owner', 'PRO')"#)
        .await
        .unwrap();
    let Imported::Created(stored) = import(&db, &server).await.unwrap() else {
        panic!("a new bot is created");
    };
    assert_eq!(stored.location, "SERVER");
}

/// A move interrupted after the bot landed is repeated from the start: check, then import.
#[tokio::test]
async fn a_repeated_move_passes_at_the_limit_and_keeps_the_stored_apps() {
    let db = database().await;
    seed_account(&db).await;
    let local = request(LOCAL_ID, json!({ "app_ids": ["app-1"] }));
    let first = import(&db, &local).await.unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let Imported::Created(stored) = first else {
        panic!("a new bot is created");
    };
    for _ in 1..MAX_CUSTOM_BOTS {
        custom_bot(&db, "owner").await;
    }
    assert_eq!(
        custom_bot_count(&db, "owner").await.unwrap(),
        MAX_CUSTOM_BOTS
    );

    let crowd: Vec<String> = (0..=MAX_APPS_PER_BOT)
        .map(|n| format!("app-x-{n}"))
        .collect();
    for apps in [json!(["app-1", "app-2"]), json!(crowd)] {
        let retried = request(LOCAL_ID, json!({ "app_ids": apps }));
        let checked = review(&db, "owner", &retried).await.unwrap();
        assert_eq!(checked, Review::Imported(stored.clone()));
        assert!(checked.into_issues().is_empty(), "the check passes");
        let again = import(&db, &retried).await.unwrap();
        assert_eq!(again.status(), StatusCode::OK);
        assert_eq!(again, Imported::Existing(stored.clone()));
    }
    assert_eq!(
        app_ids(&db, &stored.id).await,
        ["app-1"],
        "apps listed on a repeat are skipped"
    );
    assert_eq!(
        custom_bot_count(&db, "owner").await.unwrap(),
        MAX_CUSTOM_BOTS
    );
}

#[tokio::test]
async fn every_part_the_account_cannot_take_is_reported_and_the_first_one_answers() {
    let db = database().await;
    seed_account(&db).await;
    let local = request(
        LOCAL_ID,
        json!({
            "location": "server",
            "capabilities": ["command_line", "internet", "computer_use", "COMPUTER_USE"],
            "profile_id": "profile-2",
            "device_id": "device-2",
            "app_ids": ["app-3", "app-1", "app-404"],
        }),
    );
    assert_eq!(
        issues(review(&db, "owner", &local).await.unwrap()),
        [
            issue(
                ImportIssuePart::Capability,
                BOT_CAPABILITY_LOCATION,
                Some("computer_use")
            ),
            issue(
                ImportIssuePart::Capability,
                BOT_CAPABILITY_LOCATION,
                Some("command_line")
            ),
            issue(
                ImportIssuePart::Profile,
                BOT_PROFILE_UNAVAILABLE,
                Some("profile-2")
            ),
            issue(
                ImportIssuePart::Device,
                BOT_DEVICE_UNAVAILABLE,
                Some("device-2")
            ),
            issue(ImportIssuePart::App, BOT_APP_UNAVAILABLE, Some("app-3")),
            issue(ImportIssuePart::App, BOT_APP_UNAVAILABLE, Some("app-404")),
        ]
    );
    let error = import(&db, &local).await.unwrap_err();
    assert_eq!(
        code(&error),
        (StatusCode::UNPROCESSABLE_ENTITY, BOT_CAPABILITY_LOCATION)
    );
    assert!(error.public_message().unwrap().contains("'computer_use'"));
    assert_eq!(bot::Entity::find().count(&db).await.unwrap(), 0);

    let broken = request(
        LOCAL_ID,
        json!({ "name": " ", "capabilities": ["telepathy"], "app_ids": ["app-3"] }),
    );
    let found = review(&db, "owner", &broken).await.unwrap().into_issues();
    assert_eq!(
        found
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>(),
        [BOT_INVALID, BOT_APP_UNAVAILABLE]
    );
    assert!(found[0].message.contains("name"), "{}", found[0].message);
    let error = import(&db, &broken).await.unwrap_err();
    assert_eq!(
        code(&error),
        (StatusCode::UNPROCESSABLE_ENTITY, BOT_INVALID)
    );
}

#[tokio::test]
async fn the_profile_and_device_must_be_live_and_the_owner_s() {
    let db = database().await;
    seed_account(&db).await;
    for (profile_id, available) in [
        ("profile-1", true),
        ("profile-2", false),
        ("profile-gone", false),
        ("profile-404", false),
    ] {
        let local = request(LOCAL_ID, json!({ "profile_id": profile_id }));
        let expected = (!available).then(|| {
            issue(
                ImportIssuePart::Profile,
                BOT_PROFILE_UNAVAILABLE,
                Some(profile_id),
            )
        });
        assert_eq!(
            issues(review(&db, "owner", &local).await.unwrap()),
            Vec::from_iter(expected),
            "{profile_id}"
        );
    }
    for (device_id, available) in [
        ("device-1", true),
        ("device-old", false),
        ("device-2", false),
    ] {
        let local = request(
            LOCAL_ID,
            json!({ "location": "device", "device_id": device_id }),
        );
        let expected = (!available).then(|| {
            issue(
                ImportIssuePart::Device,
                BOT_DEVICE_UNAVAILABLE,
                Some(device_id),
            )
        });
        assert_eq!(
            issues(review(&db, "owner", &local).await.unwrap()),
            Vec::from_iter(expected),
            "{device_id}"
        );
    }
    let error = import(&db, &request(LOCAL_ID, json!({ "device_id": "device-2" })))
        .await
        .unwrap_err();
    assert_eq!(
        code(&error),
        (StatusCode::UNPROCESSABLE_ENTITY, BOT_DEVICE_UNAVAILABLE)
    );
}

#[tokio::test]
async fn a_bot_moves_with_at_most_two_hundred_apps() {
    let db = database().await;
    seed_account(&db).await;
    let crowd: Vec<String> = (0..=MAX_APPS_PER_BOT)
        .map(|n| format!("app-x-{n}"))
        .collect();
    let local = request(LOCAL_ID, json!({ "app_ids": crowd }));
    assert_eq!(
        issues(review(&db, "owner", &local).await.unwrap()),
        [issue(ImportIssuePart::App, BOT_APP_LIMIT_REACHED, None)]
    );
    let error = import(&db, &local).await.unwrap_err();
    assert_eq!(
        code(&error),
        (StatusCode::UNPROCESSABLE_ENTITY, BOT_APP_LIMIT_REACHED)
    );
}

#[tokio::test]
async fn a_creation_time_from_the_future_counts_as_now() {
    let db = database().await;
    let local = request(LOCAL_ID, json!({ "created_at": "2099-01-01T00:00:00Z" }));
    let Imported::Created(stored) = import(&db, &local).await.unwrap() else {
        panic!("a new bot is created");
    };
    assert_eq!(
        (stored.created_at, stored.updated_at),
        (at(1_000), at(1_000))
    );
}

#[test]
fn both_routes_are_documented_with_the_flattened_bot_fields() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    for (path, statuses) in [
        ("/bots/import/check", &["200", "400", "401", "403"][..]),
        (
            "/bots/import",
            &["200", "201", "400", "401", "403", "409", "422"][..],
        ),
    ] {
        for status in statuses {
            let pointer = format!("/paths/{}/post/responses/{status}", path.replace('/', "~1"));
            assert!(spec.pointer(&pointer).is_some(), "{path} {status}");
        }
    }
    let schemas = &spec["components"]["schemas"];
    let request = schemas["ImportBotRequest"].to_string();
    assert!(
        request.contains("#/components/schemas/CreateBotRequest"),
        "{request}"
    );
    assert!(
        request.contains("local_id")
            && request.contains("app_ids")
            && request.contains("created_at"),
        "{request}"
    );
    for schema in ["ImportBotCheck", "ImportIssue", "ImportIssuePart"] {
        assert!(schemas.get(schema).is_some(), "{schema}");
    }
}

#[test]
fn issues_serialize_with_snake_case_parts() {
    let issue = ImportIssue {
        part: ImportIssuePart::Capability,
        code: BOT_CAPABILITY_LOCATION.into(),
        message: "A bot on the server can't hold 'command_line'".into(),
        ref_id: Some("command_line".into()),
    };
    assert_eq!(
        serde_json::to_value(issue).unwrap(),
        json!({
            "part": "capability",
            "code": "BOT_CAPABILITY_LOCATION",
            "message": "A bot on the server can't hold 'command_line'",
            "ref_id": "command_line",
        })
    );
}
