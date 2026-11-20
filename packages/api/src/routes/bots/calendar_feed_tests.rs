use super::calendar_feed::{
    UpdateCalendarFeed, delete_in, feed_in, feed_response, hash_token, layers_of, lookup_in,
    rotate_in, token_of, touch_in, upsert_in, view,
};
use crate::{
    bot_calendar_feed::etag_of,
    entity::{bot, bot_calendar_feed},
    routes::inbox::fixture::{assert_code, database as inbox_database, insert_bot},
};
use axum::http::{StatusCode, header};
use chrono::{Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait, Schema,
};

const KEY: [u8; 32] = [9; 32];
const ORIGIN: &str = "https://api.example.com";

async fn database() -> DatabaseConnection {
    let db = inbox_database().await;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(bot_calendar_feed::Entity))
        .await
        .unwrap();
    insert_bot(&db, "bot-a", "user-a", "SERVER").await;
    insert_bot(&db, "bot-b", "user-b", "SERVER").await;
    db
}

fn token_of_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap()
        .trim_end_matches(".ics")
        .to_string()
}

async fn created(db: &DatabaseConnection) -> (bot_calendar_feed::Model, String) {
    let row = upsert_in(db, "user-a", "bot-a", &UpdateCalendarFeed::default(), &KEY)
        .await
        .unwrap();
    let url = view(&row, &KEY, ORIGIN).unwrap().url;
    (row, token_of_url(&url))
}

#[tokio::test]
async fn the_first_put_creates_a_feed_with_default_layers_and_a_private_link() {
    let db = database().await;
    let (row, token) = created(&db).await;
    assert_eq!(
        (
            row.include_events,
            row.include_tasks,
            row.include_memories,
            row.include_notes
        ),
        (true, true, false, false)
    );
    let feed = view(&row, &KEY, ORIGIN).unwrap();
    assert_eq!(
        feed.url,
        format!("{ORIGIN}/api/v1/calendar-feeds/{token}.ics")
    );
    assert_eq!(
        feed.webcal_url,
        format!("webcal://api.example.com/api/v1/calendar-feeds/{token}.ics")
    );
    assert_eq!(token.len(), 43);
    assert_eq!(row.token_hash, hash_token(&token));
    assert!(!row.token_encrypted.contains(&token));
    assert!(token_of(&token).is_some());
}

#[tokio::test]
async fn a_later_put_changes_the_layers_and_keeps_the_link() {
    let db = database().await;
    let (_, token) = created(&db).await;
    let row = upsert_in(
        &db,
        "user-a",
        "bot-a",
        &UpdateCalendarFeed {
            include_memories: Some(true),
            include_tasks: Some(false),
            ..Default::default()
        },
        &KEY,
    )
    .await
    .unwrap();
    assert!(row.include_memories && !row.include_tasks && row.include_events);
    assert_eq!(row.token_hash, hash_token(&token));
}

#[tokio::test]
async fn only_the_owner_reaches_a_feed() {
    let db = database().await;
    created(&db).await;
    let error = feed_in(&db, "user-b", "bot-a").await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "NOT_FOUND");
    let error = upsert_in(&db, "user-b", "bot-a", &UpdateCalendarFeed::default(), &KEY)
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "NOT_FOUND");
    let error = rotate_in(&db, "user-b", "bot-a", &KEY).await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "NOT_FOUND");
    assert!(delete_in(&db, "user-b", "bot-a").await.is_err());
    assert!(feed_in(&db, "user-a", "bot-a").await.is_ok());
}

#[tokio::test]
async fn a_bot_without_a_feed_reads_as_none() {
    let db = database().await;
    let error = feed_in(&db, "user-a", "bot-a").await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "BOT_CALENDAR_FEED_NONE");
}

#[tokio::test]
async fn rotating_kills_the_old_link() {
    let db = database().await;
    let (_, old) = created(&db).await;
    assert!(lookup_in(&db, &old).await.unwrap().is_some());
    let row = rotate_in(&db, "user-a", "bot-a", &KEY).await.unwrap();
    let new = token_of_url(&view(&row, &KEY, ORIGIN).unwrap().url);
    assert_ne!(old, new);
    assert!(lookup_in(&db, &old).await.unwrap().is_none());
    let (feed, bot) = lookup_in(&db, &new).await.unwrap().unwrap();
    assert_eq!((feed.bot_id.as_str(), bot.id.as_str()), ("bot-a", "bot-a"));
}

#[tokio::test]
async fn deleting_turns_the_link_off() {
    let db = database().await;
    let (_, token) = created(&db).await;
    delete_in(&db, "user-a", "bot-a").await.unwrap();
    assert!(lookup_in(&db, &token).await.unwrap().is_none());
    let error = feed_in(&db, "user-a", "bot-a").await.unwrap_err();
    assert_code(&error, StatusCode::NOT_FOUND, "BOT_CALENDAR_FEED_NONE");
}

#[tokio::test]
async fn a_deleted_bot_serves_nothing() {
    let db = database().await;
    let (_, token) = created(&db).await;
    let row = bot::Entity::find_by_id("bot-a")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut row: bot::ActiveModel = row.into();
    row.deleted_at = Set(Some(Utc::now().fixed_offset()));
    row.update(&db).await.unwrap();
    assert!(lookup_in(&db, &token).await.unwrap().is_none());
}

#[tokio::test]
async fn unknown_tokens_miss() {
    let db = database().await;
    created(&db).await;
    let guess = "A".repeat(43);
    assert!(lookup_in(&db, &guess).await.unwrap().is_none());
}

#[test]
fn only_well_formed_tokens_are_looked_up() {
    let token = "a".repeat(43);
    assert_eq!(token_of(&format!("{token}.ics")), Some(token.as_str()));
    assert_eq!(token_of(&token), Some(token.as_str()));
    assert_eq!(token_of("short.ics"), None);
    assert_eq!(token_of(&format!("{}!", "a".repeat(42))), None);
    assert_eq!(token_of(&format!("{token}x.ics")), None);
}

#[tokio::test]
async fn layers_follow_the_bots_capabilities() {
    let db = database().await;
    let (row, _) = created(&db).await;
    let bot = bot::Entity::find_by_id("bot-a")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!layers_of(&row, &bot).events);
    let mut active: bot::ActiveModel = bot.into();
    active.capabilities = Set(crate::entity::json_types::StringList(vec![
        "calendar".into(),
    ]));
    let bot = active.update(&db).await.unwrap();
    let layers = layers_of(&row, &bot);
    assert!(layers.events && !layers.tasks);
}

#[test]
fn the_response_carries_calendar_headers_and_an_etag() {
    let body = "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_string();
    let response = feed_response(body.clone(), "secretary-calendar", None);
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/calendar; charset=utf-8"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "inline; filename=\"secretary-calendar.ics\""
    );
    assert_eq!(headers[header::CACHE_CONTROL], "private, max-age=300");
    assert_eq!(headers[header::ETAG], etag_of(&body).as_str());
}

#[test]
fn a_matching_etag_answers_304_without_a_body() {
    let body = "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_string();
    let etag = etag_of(&body);
    for presented in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
        "*".into(),
    ] {
        let response = feed_response(body.clone(), "s", Some(&presented));
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED, "{presented}");
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert_eq!(response.headers()[header::ETAG], etag.as_str());
    }
    let response = feed_response(body, "s", Some("\"other\""));
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_fetch_is_recorded_at_most_every_five_minutes() {
    let db = database().await;
    let (row, _) = created(&db).await;
    let start = Utc::now();
    touch_in(&db, &row.id, start).await.unwrap();
    let first = feed_in(&db, "user-a", "bot-a")
        .await
        .unwrap()
        .last_fetched_at;
    assert!(first.is_some());
    touch_in(&db, &row.id, start + Duration::minutes(2))
        .await
        .unwrap();
    let second = feed_in(&db, "user-a", "bot-a")
        .await
        .unwrap()
        .last_fetched_at;
    assert_eq!(first, second);
    touch_in(&db, &row.id, start + Duration::minutes(6))
        .await
        .unwrap();
    let third = feed_in(&db, "user-a", "bot-a")
        .await
        .unwrap()
        .last_fetched_at;
    assert!(third > first);
}
