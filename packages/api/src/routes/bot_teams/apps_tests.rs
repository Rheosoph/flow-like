use super::{
    access::{TEAM_FORBIDDEN, TEAM_LIMIT, TEAM_NOT_FOUND},
    apps::{approve_pin_in, decline_pin_in, list_pinnable_in, pin_app_in, unpin_app_in},
    bots::add_bot_in,
    model::AddBotRequest,
    people::remove_person_in,
    tests::{create, join, world},
    view::org_view,
};
use crate::{
    entity::{
        app, bot_app, bot_team_app, event, execution_usage_tracking, membership, meta, page, role,
        sea_orm_active_enums::{
            AppType, ExecutionMode, ExecutionStatus, Status as AppStatus, Visibility,
        },
    },
    routes::inbox::fixture::{assert_code, at},
};
use axum::http::StatusCode;
use flow_like_types::bot_team::{
    MAX_TEAM_APPS, ORG_APP_DAYS, OrgApp, OrgView, TeamInvitationKind, TeamMemberStatus,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait, Schema,
    sea_query::Table,
};

const OWNER: i64 = 0b1;
const ADMIN: i64 = 0b10;
const READER: i64 = 0b100;

async fn plain_table(db: &DatabaseConnection, source: sea_orm::sea_query::TableCreateStatement) {
    let mut table = Table::create();
    table.table(source.get_table_name().unwrap().clone());
    for column in source.get_columns() {
        table.col(column.clone());
    }
    db.execute(&table).await.unwrap();
}

async fn apps_world() -> (DatabaseConnection, String) {
    let db = world().await;
    let schema = Schema::new(db.get_database_backend());
    plain_table(&db, schema.create_table_from_entity(app::Entity)).await;
    plain_table(&db, schema.create_table_from_entity(role::Entity)).await;
    plain_table(&db, schema.create_table_from_entity(meta::Entity)).await;
    plain_table(&db, schema.create_table_from_entity(event::Entity)).await;
    plain_table(&db, schema.create_table_from_entity(page::Entity)).await;
    plain_table(&db, schema.create_table_from_entity(bot_app::Entity)).await;
    plain_table(
        &db,
        schema.create_table_from_entity(execution_usage_tracking::Entity),
    )
    .await;
    db.execute(&schema.create_table_from_entity(bot_team_app::Entity))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."ExecutionUsageDaily" (
            day TEXT NOT NULL, id TEXT NOT NULL, "userId" TEXT,
            "technicalUserId" TEXT, "appId" TEXT, status TEXT NOT NULL,
            executions INTEGER NOT NULL, microseconds INTEGER NOT NULL,
            PRIMARY KEY (day, id)
        )"#,
    )
    .await
    .unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."ExecutionUsageDay" (
            day TEXT PRIMARY KEY, "rolledUpAt" TEXT, claim TEXT, "claimedAt" TEXT
        )"#,
    )
    .await
    .unwrap();
    let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
    let security = create(&db, "mara", "Security", Some(&kestrel))
        .await
        .unwrap();
    join(&db, "mara", &security, "jonas").await;
    join(&db, "mara", &security, "anna").await;
    (db, security)
}

async fn grant(db: &DatabaseConnection, app_id: &str, user: &str, permissions: i64) {
    let role_id = format!("role-{app_id}-{user}");
    role::ActiveModel {
        id: Set(role_id.clone()),
        name: Set(role_id.clone()),
        permissions: Set(permissions),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    membership::ActiveModel {
        id: Set(format!("member-{app_id}-{user}")),
        user_id: Set(user.into()),
        app_id: Set(app_id.into()),
        role_id: Set(role_id),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

async fn insert_app(
    db: &DatabaseConnection,
    id: &str,
    owner: &str,
    visibility: Visibility,
    workspace_of: Option<&str>,
    status: AppStatus,
) {
    app::ActiveModel {
        id: Set(id.into()),
        owner_role_id: Set(Some(format!("role-{id}-{owner}"))),
        rating_sum: Set(0),
        rating_count: Set(0),
        download_count: Set(0),
        interactions_count: Set(0),
        total_size: Set(0),
        price: Set(0),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        allow_forking: Set(false),
        status: Set(status),
        visibility: Set(visibility),
        execution_mode: Set(ExecutionMode::Any),
        app_type: Set(Some(AppType::DataPipeline)),
        bot_id: Set(workspace_of.map(str::to_string)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    grant(db, id, owner, OWNER).await;
}

async fn app_of(db: &DatabaseConnection, id: &str, owner: &str) {
    insert_app(db, id, owner, Visibility::Private, None, AppStatus::Active).await;
}

async fn name_app(db: &DatabaseConnection, app_id: &str, lang: &str, name: &str) {
    meta::ActiveModel {
        id: Set(format!("meta-{app_id}-{lang}")),
        lang: Set(lang.into()),
        name: Set(name.into()),
        description: Set(Some(format!("about {name}"))),
        icon: Set(Some("icon".into())),
        app_id: Set(Some(app_id.into())),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

async fn pin(
    db: &DatabaseConnection,
    sub: &str,
    team: &str,
    app_id: &str,
) -> Result<super::effects::Effects, crate::error::ApiError> {
    pin_app_in(db, sub, team, app_id, at(10)).await
}

fn app_in<'a>(view: &'a OrgView, id: &str) -> Option<&'a OrgApp> {
    view.apps.iter().find(|app| app.id == id)
}

async fn pins_of(
    db: &DatabaseConnection,
    viewer: &str,
    team: &str,
) -> Vec<(String, TeamMemberStatus)> {
    org_view(db, viewer)
        .await
        .unwrap()
        .teams
        .into_iter()
        .find(|row| row.id == team)
        .unwrap()
        .apps
        .into_iter()
        .map(|pin| (pin.app_id, pin.status))
        .collect()
}

#[tokio::test]
async fn a_team_admins_pin_is_active_at_once() {
    let (db, security) = apps_world().await;
    app_of(&db, "app1", "mara").await;
    pin(&db, "mara", &security, "app1").await.unwrap();
    pin(&db, "mara", &security, "app1").await.unwrap();
    assert_eq!(
        pins_of(&db, "jonas", &security).await,
        vec![("app1".to_string(), TeamMemberStatus::Active)]
    );
    let view = org_view(&db, "mara").await.unwrap();
    let card = app_in(&view, "app1").unwrap();
    assert!(card.can_open && card.reader_admin && !card.can_request);
    assert_eq!(card.owner_id.as_deref(), Some("mara"));
    assert_eq!(card.app_type.as_deref(), Some("DataPipeline"));
}

#[tokio::test]
async fn a_members_pin_waits_for_an_admin_and_shows_only_to_admins_and_the_pinner() {
    let (db, security) = apps_world().await;
    app_of(&db, "app2", "jonas").await;
    let effects = pin(&db, "jonas", &security, "app2").await.unwrap();
    assert_eq!(effects.notices.len(), 1);
    assert_eq!(effects.notices[0].user_id, "mara");
    assert_eq!(effects.wake, vec!["mara".to_string()]);
    let pending = vec![("app2".to_string(), TeamMemberStatus::Pending)];
    assert_eq!(pins_of(&db, "jonas", &security).await, pending);
    assert_eq!(pins_of(&db, "mara", &security).await, pending);
    assert!(pins_of(&db, "anna", &security).await.is_empty());
    assert!(
        app_in(&org_view(&db, "anna").await.unwrap(), "app2").is_none(),
        "a pending pin stays off the chart for others"
    );
    let invitations = org_view(&db, "mara").await.unwrap().invitations;
    let invitation = invitations
        .iter()
        .find(|row| row.kind == TeamInvitationKind::AppPin)
        .unwrap();
    assert_eq!(invitation.app_id.as_deref(), Some("app2"));
    assert_eq!(invitation.user_id.as_deref(), Some("jonas"));
    assert_eq!(invitation.invited_by.as_deref(), Some("jonas"));
    assert!(
        org_view(&db, "jonas")
            .await
            .unwrap()
            .invitations
            .iter()
            .all(|row| row.kind != TeamInvitationKind::AppPin)
    );

    let error = approve_pin_in(&db, "jonas", &security, "app2", at(11))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    approve_pin_in(&db, "mara", &security, "app2", at(11))
        .await
        .unwrap();
    let pin = bot_team_app::Entity::find_by_id((security.clone(), "app2".to_string()))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (pin.status.as_str(), pin.approved_by.as_deref()),
        ("ACTIVE", Some("mara"))
    );
    assert_eq!(pins_of(&db, "anna", &security).await.len(), 1);
    assert!(
        org_view(&db, "mara")
            .await
            .unwrap()
            .invitations
            .iter()
            .all(|row| row.kind != TeamInvitationKind::AppPin)
    );
}

#[tokio::test]
async fn an_admin_pinning_a_pending_app_approves_it() {
    let (db, security) = apps_world().await;
    app_of(&db, "app2", "jonas").await;
    grant(&db, "app2", "mara", ADMIN).await;
    pin(&db, "jonas", &security, "app2").await.unwrap();
    pin(&db, "mara", &security, "app2").await.unwrap();
    assert_eq!(
        pins_of(&db, "anna", &security).await,
        vec![("app2".to_string(), TeamMemberStatus::Active)]
    );
}

#[tokio::test]
async fn a_declined_pin_is_deleted() {
    let (db, security) = apps_world().await;
    app_of(&db, "app2", "jonas").await;
    pin(&db, "jonas", &security, "app2").await.unwrap();
    let error = decline_pin_in(&db, "jonas", &security, "app2")
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    decline_pin_in(&db, "mara", &security, "app2").await.unwrap();
    assert!(pins_of(&db, "jonas", &security).await.is_empty());
    let error = decline_pin_in(&db, "mara", &security, "app2")
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);
}

#[tokio::test]
async fn pinning_needs_a_team_seat_an_app_admin_role_and_a_real_app() {
    let (db, security) = apps_world().await;
    app_of(&db, "app1", "mara").await;
    grant(&db, "app1", "anna", READER).await;
    let error = pin(&db, "anna", &security, "app1").await.unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    let error = pin(&db, "jonas", &security, "app1").await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "NOT_FOUND");
    let error = pin(&db, "outsider", &security, "app1").await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);

    insert_app(&db, "workspace", "mara", Visibility::Private, Some("mara-bot"), AppStatus::Active)
        .await;
    insert_app(&db, "old", "mara", Visibility::Private, None, AppStatus::Archived).await;
    for refused in ["workspace", "old", "missing"] {
        let error = pin(&db, "mara", &security, refused).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::NOT_FOUND, "{refused}");
    }
    assert!(pins_of(&db, "mara", &security).await.is_empty());
}

#[tokio::test]
async fn a_team_holds_at_most_the_limit_of_pins() {
    let (db, security) = apps_world().await;
    for index in 0..MAX_TEAM_APPS {
        let id = format!("fill-{index}");
        app_of(&db, &id, "mara").await;
        pin(&db, "mara", &security, &id).await.unwrap();
    }
    app_of(&db, "one-more", "mara").await;
    let error = pin(&db, "mara", &security, "one-more").await.unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, TEAM_LIMIT);
}

#[tokio::test]
async fn the_pinner_a_team_admin_or_an_app_admin_unpins_nobody_else() {
    let (db, security) = apps_world().await;
    app_of(&db, "app2", "jonas").await;
    app_of(&db, "app3", "jonas").await;
    app_of(&db, "app4", "jonas").await;
    for id in ["app2", "app3", "app4"] {
        pin(&db, "jonas", &security, id).await.unwrap();
    }
    let error = unpin_app_in(&db, "anna", &security, "app2").await.unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    let error = unpin_app_in(&db, "outsider", &security, "app2")
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);
    unpin_app_in(&db, "jonas", &security, "app2").await.unwrap();
    unpin_app_in(&db, "mara", &security, "app3").await.unwrap();
    grant(&db, "app4", "outsider", ADMIN).await;
    unpin_app_in(&db, "outsider", &security, "app4").await.unwrap();
    assert!(pins_of(&db, "mara", &security).await.is_empty());
}

#[tokio::test]
async fn pins_of_a_person_who_left_the_org_go_with_them() {
    let (db, security) = apps_world().await;
    app_of(&db, "app1", "mara").await;
    app_of(&db, "app2", "jonas").await;
    pin(&db, "mara", &security, "app1").await.unwrap();
    pin(&db, "jonas", &security, "app2").await.unwrap();
    approve_pin_in(&db, "mara", &security, "app2", at(11))
        .await
        .unwrap();
    remove_person_in(&db, "jonas", &security, "jonas", at(12))
        .await
        .unwrap();
    assert_eq!(
        pins_of(&db, "mara", &security).await,
        vec![("app1".to_string(), TeamMemberStatus::Active)]
    );
}

#[tokio::test]
async fn everyone_in_the_org_sees_metadata_but_only_members_see_numbers() {
    let (db, security) = apps_world().await;
    insert_app(&db, "open", "jonas", Visibility::PublicRequestAccess, None, AppStatus::Active)
        .await;
    insert_app(&db, "shut", "jonas", Visibility::Private, None, AppStatus::Active).await;
    name_app(&db, "open", "de", "Offen").await;
    name_app(&db, "open", "en", "Open").await;
    name_app(&db, "shut", "en", "Shut").await;
    for id in ["open", "shut"] {
        pin(&db, "jonas", &security, id).await.unwrap();
        approve_pin_in(&db, "mara", &security, id, at(11))
            .await
            .unwrap();
    }
    let view = org_view(&db, "anna").await.unwrap();
    let open = app_in(&view, "open").unwrap();
    assert_eq!(open.name, "Open");
    assert_eq!(open.description.as_deref(), Some("about Open"));
    assert_eq!(open.icon.as_deref(), Some("icon"));
    assert!(open.stats.is_none() && !open.can_open && open.can_request && !open.reader_admin);
    let shut = app_in(&view, "shut").unwrap();
    assert!(shut.stats.is_none() && !shut.can_open && !shut.can_request);
    let own = org_view(&db, "jonas").await.unwrap();
    let stats = app_in(&own, "open").unwrap().stats.as_ref().unwrap();
    assert_eq!(stats.runs.len(), ORG_APP_DAYS);
    assert_eq!(stats.failed.len(), ORG_APP_DAYS);
}

async fn count_runs(db: &DatabaseConnection, app_id: &str, status: ExecutionStatus, n: usize) {
    for index in 0..n {
        let now = chrono::Utc::now().fixed_offset();
        execution_usage_tracking::ActiveModel {
            id: Set(format!("run-{app_id}-{status:?}-{index}")),
            board_id: Set("board".into()),
            node_id: Set("node".into()),
            version: Set("1.0.0".into()),
            microseconds: Set(10),
            app_id: Set(Some(app_id.into())),
            created_at: Set(now),
            updated_at: Set(now),
            status: Set(status.clone()),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn a_members_numbers_cover_every_day_wiring_and_pages() {
    let (db, security) = apps_world().await;
    app_of(&db, "app1", "mara").await;
    pin(&db, "mara", &security, "app1").await.unwrap();
    let now = chrono::Utc::now().fixed_offset();
    let today = now.date_naive();
    let closed = crate::execution_usage_rollup::last_closed_day(now);
    let first = today - chrono::Duration::days(ORG_APP_DAYS as i64 - 1);
    let mut day = first;
    while day <= closed {
        db.execute_unprepared(&format!(
            r#"INSERT INTO public."ExecutionUsageDay" (day, "rolledUpAt") VALUES ('{day}', '2020-01-01T00:00:00+00:00')"#
        ))
        .await
        .unwrap();
        day += chrono::Duration::days(1);
    }
    let rolled_day = first + chrono::Duration::days(2);
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."ExecutionUsageDaily" (day, id, "appId", status, executions, microseconds) VALUES
            ('{rolled_day}', 'a', 'app1', 'INFO', 5, 0),
            ('{rolled_day}', 'b', 'app1', 'ERROR', 2, 0),
            ('{rolled_day}', 'c', 'other', 'INFO', 9, 0)"#
    ))
    .await
    .unwrap();
    count_runs(&db, "app1", ExecutionStatus::Info, 2).await;
    count_runs(&db, "app1", ExecutionStatus::Fatal, 1).await;
    for (id, kind, config) in [
        ("e1", "api", None),
        (
            "e2",
            "cron",
            Some(serde_json::json!({ "base64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                br#"{"cron_expression":"0 9 * * *"}"#,
            ) })),
        ),
    ] {
        event::ActiveModel {
            id: Set(id.into()),
            app_id: Set("app1".into()),
            name: Set(id.into()),
            event_type: Set(kind.into()),
            active: Set(true),
            priority: Set(0),
            is_default: Set(false),
            event_version: Set("0.0.0".into()),
            config: Set(config),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            execution_mode: Set("Local".into()),
            exposure: Set("PUBLIC".into()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    event::ActiveModel {
        id: Set("e3".into()),
        app_id: Set("app1".into()),
        name: Set("off".into()),
        event_type: Set("email".into()),
        active: Set(false),
        priority: Set(0),
        is_default: Set(false),
        event_version: Set("0.0.0".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        execution_mode: Set("Local".into()),
        exposure: Set("PUBLIC".into()),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    for id in ["p1", "p2"] {
        page::ActiveModel {
            id: Set(id.into()),
            name: Set(id.into()),
            app_id: Set("app1".into()),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }

    let view = org_view(&db, "mara").await.unwrap();
    let stats = app_in(&view, "app1").unwrap().stats.clone().unwrap();
    assert_eq!(stats.runs.len(), ORG_APP_DAYS);
    assert_eq!(stats.runs[2], 7);
    assert_eq!(stats.failed[2], 2);
    assert_eq!(stats.runs[ORG_APP_DAYS - 1], 3);
    assert_eq!(stats.failed[ORG_APP_DAYS - 1], 1);
    assert_eq!(stats.runs.iter().sum::<u32>(), 10);
    assert!(stats.last_run_at.is_some());
    assert_eq!(stats.triggers, vec!["api".to_string(), "cron".to_string()]);
    assert_eq!(stats.schedule.as_deref(), Some("0 9 * * *"));
    assert_eq!(stats.pages, 2);
}

#[tokio::test]
async fn a_bots_apps_count_only_while_it_is_active_and_never_its_workspace() {
    let (db, security) = apps_world().await;
    app_of(&db, "app1", "mara").await;
    app_of(&db, "app2", "jonas").await;
    insert_app(&db, "workspace", "mara", Visibility::Private, Some("mara-bot"), AppStatus::Active)
        .await;
    for (bot, app_id) in [
        ("mara-bot", "app1"),
        ("mara-bot", "workspace"),
        ("anna-bot", "app2"),
        ("jonas-bot", "app2"),
    ] {
        bot_app::ActiveModel {
            bot_id: Set(bot.into()),
            app_id: Set(app_id.into()),
            added_at: Set(at(0)),
            permission_mask: Set(None),
            scopes: Set(serde_json::json!({})),
        }
        .insert(&db)
        .await
        .unwrap();
    }
    super::tests::own_bot(&db, "mara", &security, "mara-bot").await;
    let invite = AddBotRequest {
        bot_id: "anna-bot".into(),
        limits: None,
    };
    add_bot_in(&db, "mara", &security, &invite, at(5)).await.unwrap();
    let view = org_view(&db, "mara").await.unwrap();
    let ids = |bot: &str| {
        view.bots
            .iter()
            .find(|row| row.id == bot)
            .map(|row| row.app_ids.clone())
    };
    assert_eq!(ids("mara-bot"), Some(vec!["app1".to_string()]));
    assert_eq!(ids("anna-bot"), Some(Vec::new()));
    assert!(app_in(&view, "app1").is_some());
    assert!(app_in(&view, "app2").is_none());
    assert!(app_in(&view, "workspace").is_none());
}

#[tokio::test]
async fn the_pin_dialog_lists_my_apps_admin_first_with_pins_here_and_elsewhere() {
    let (db, security) = apps_world().await;
    let other = create(&db, "mara", "Other", None).await.unwrap();
    join(&db, "mara", &other, "jonas").await;
    app_of(&db, "zeta", "jonas").await;
    app_of(&db, "alpha", "mara").await;
    grant(&db, "alpha", "jonas", READER).await;
    app_of(&db, "mid", "jonas").await;
    name_app(&db, "zeta", "en", "Zeta").await;
    name_app(&db, "alpha", "en", "Alpha").await;
    name_app(&db, "mid", "en", "Mid").await;
    pin(&db, "jonas", &other, "mid").await.unwrap();
    approve_pin_in(&db, "mara", &other, "mid", at(11)).await.unwrap();
    pin(&db, "jonas", &security, "zeta").await.unwrap();
    let apps = list_pinnable_in(&db, "jonas", &security).await.unwrap();
    let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
    assert_eq!(names, ["Mid", "Zeta", "Alpha"]);
    assert_eq!(apps[0].pinned_elsewhere, vec![other]);
    assert_eq!(apps[1].pinned, Some(TeamMemberStatus::Pending));
    assert!(!apps[2].reader_admin);
    assert_eq!(apps[0].owner_id.as_deref(), Some("jonas"));
    let error = list_pinnable_in(&db, "outsider", &security).await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);
}
