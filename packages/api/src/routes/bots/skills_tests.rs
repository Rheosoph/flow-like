use super::skills::{
    Origin, SKILLS_CAPABILITY, active_index, create_in, delete_in, list_in, set_status_in,
    update_in,
};
use crate::{
    entity::{bot, bot_skill},
    routes::inbox::fixture::{database as inbox_database, insert_bot},
};
use axum::http::StatusCode;
use chrono::Utc;
use flow_like_types::bot_skill::{CreateBotSkill, SkillStatus, UpdateBotSkill};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotSkill" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "name" TEXT NOT NULL,
            "description" TEXT NOT NULL, "body" TEXT NOT NULL, "source" TEXT NOT NULL,
            "status" TEXT NOT NULL, "tools" TEXT NOT NULL, "createdAt" TEXT NOT NULL,
            "updatedAt" TEXT NOT NULL)"#,
    )
    .await
    .unwrap();
    db
}

async fn skilled_bot(db: &DatabaseConnection, id: &str, owner: &str) {
    insert_bot(db, id, owner, "SERVER").await;
    let mut row: bot::ActiveModel = bot::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .into();
    row.capabilities = Set(vec![SKILLS_CAPABILITY.to_string()].into());
    row.update(db).await.unwrap();
}

fn skill(name: &str) -> CreateBotSkill {
    CreateBotSkill {
        name: name.into(),
        description: "Writes the weekly report".into(),
        body: "Collect the numbers, then write three bullet points.".into(),
        tools: Vec::new(),
    }
}

#[tokio::test]
async fn another_users_bot_gives_404_on_every_skill_route() {
    let db = database().await;
    skilled_bot(&db, "bot-a", "user-a").await;
    let now = Utc::now().fixed_offset();
    let made = create_in(
        &db,
        "user-a",
        "bot-a",
        skill("weekly"),
        Origin::owner(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(made.status, SkillStatus::Active.stored());

    let not_found =
        |error: crate::error::ApiError| assert_eq!(error.status(), StatusCode::NOT_FOUND);
    not_found(list_in(&db, "user-b", "bot-a").await.unwrap_err());
    not_found(
        create_in(&db, "user-b", "bot-a", skill("other"), Origin::owner(), now)
            .await
            .unwrap_err(),
    );
    not_found(
        update_in(
            &db,
            "user-b",
            "bot-a",
            &made.id,
            UpdateBotSkill {
                body: Some("hijacked".into()),
                ..Default::default()
            },
            now,
        )
        .await
        .unwrap_err(),
    );
    for status in [SkillStatus::Active, SkillStatus::Disabled] {
        not_found(
            set_status_in(&db, "user-b", "bot-a", &made.id, status, now)
                .await
                .unwrap_err(),
        );
    }
    not_found(
        delete_in(&db, "user-b", "bot-a", &made.id)
            .await
            .unwrap_err(),
    );

    let row = bot_skill::Entity::find_by_id(made.id.clone())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.body, made.body);
}

#[tokio::test]
async fn a_skill_is_reachable_only_through_its_own_bot() {
    let db = database().await;
    skilled_bot(&db, "bot-a", "user-a").await;
    skilled_bot(&db, "bot-b", "user-a").await;
    let now = Utc::now().fixed_offset();
    let made = create_in(
        &db,
        "user-a",
        "bot-a",
        skill("weekly"),
        Origin::owner(),
        now,
    )
    .await
    .unwrap();
    let error = delete_in(&db, "user-a", "bot-b", &made.id)
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert!(delete_in(&db, "user-a", "bot-a", &made.id).await.is_ok());
}

#[tokio::test]
async fn skills_need_the_capability_and_only_active_ones_are_indexed() {
    let db = database().await;
    insert_bot(&db, "bare", "user-a", "SERVER").await;
    let now = Utc::now().fixed_offset();
    let refused = create_in(&db, "user-a", "bare", skill("weekly"), Origin::owner(), now)
        .await
        .unwrap_err();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    skilled_bot(&db, "bot-a", "user-a").await;
    create_in(&db, "user-a", "bot-a", skill("owned"), Origin::owner(), now)
        .await
        .unwrap();
    let proposed = create_in(
        &db,
        "user-a",
        "bot-a",
        skill("proposed"),
        Origin::proposed_by_bot(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(proposed.status, SkillStatus::Pending.stored());
    let names = |index: Vec<flow_like_types::bot_skill::SkillIndexEntry>| {
        index
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(active_index(&db, "bot-a").await.unwrap()), ["owned"]);
    set_status_in(
        &db,
        "user-a",
        "bot-a",
        &proposed.id,
        SkillStatus::Active,
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        names(active_index(&db, "bot-a").await.unwrap()),
        ["owned", "proposed"]
    );
}
