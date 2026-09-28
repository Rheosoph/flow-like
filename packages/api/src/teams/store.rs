use super::{Connection, now};
use crate::{
    error::ApiError,
    state::AppState,
    utils::crypto::{decrypt_secret, encrypt_secret},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};
use serde::{Serialize, de::DeserializeOwned};
use std::time::{Duration, Instant};

const SWEEP_BATCH: u64 = 1000;
const SWEEP_BUDGET: Duration = Duration::from_secs(20);

pub(super) fn sql(query: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

#[derive(FromQueryResult)]
struct Row {
    data: String,
    revision: i32,
}

fn encode<T: Serialize>(state: &AppState, value: &T) -> Result<String, ApiError> {
    let json = serde_json::to_string(value)
        .map_err(|_| ApiError::internal("Cannot encode Teams state"))?;
    Ok(encrypt_secret(&json, &state.encryption_key))
}

fn decode<T: DeserializeOwned>(state: &AppState, value: &str) -> Result<T, ApiError> {
    let json = decrypt_secret(value, &state.encryption_key)
        .ok_or_else(|| ApiError::internal("Cannot decrypt Teams state"))?;
    serde_json::from_str(&json).map_err(|_| ApiError::internal("Cannot decode Teams state"))
}

pub(super) async fn connection(
    state: &AppState,
    id: &str,
) -> Result<Option<(Connection, i32)>, ApiError> {
    let row = Row::find_by_statement(sql(
        r#"SELECT "data","revision" FROM "TeamsBotConnection" WHERE "id"=$1"#,
        vec![id.into()],
    ))
    .one(&state.db)
    .await?;
    row.map(|row| Ok((decode(state, &row.data)?, row.revision)))
        .transpose()
}

pub(super) async fn event_connection(
    state: &AppState,
    event: &str,
) -> Result<Option<(Connection, i32)>, ApiError> {
    let row = Row::find_by_statement(sql(
        r#"SELECT "data","revision" FROM "TeamsBotConnection" WHERE "eventId"=$1"#,
        vec![event.into()],
    ))
    .one(&state.db)
    .await?;
    row.map(|row| Ok((decode(state, &row.data)?, row.revision)))
        .transpose()
}

pub(super) async fn insert_connection(
    state: &AppState,
    value: &Connection,
) -> Result<bool, ApiError> {
    Ok(state.db.execute_raw(sql(
        r#"INSERT INTO "TeamsBotConnection" ("id","appId","eventId","data","revision","managed","updatedAt") VALUES ($1,$2,$3,$4,0,$5,$6) ON CONFLICT DO NOTHING"#,
        vec![value.id.clone().into(), value.app_id.clone().into(), value.event_id.clone().into(), encode(state,value)?.into(), value.holds_managed_slot().into(), now().into()],
    )).await?.rows_affected() == 1)
}

pub(super) async fn save_connection(
    state: &AppState,
    value: &Connection,
    revision: i32,
) -> Result<(), ApiError> {
    let changed = state.db.execute_raw(sql(
        r#"UPDATE "TeamsBotConnection" SET "data"=$1,"revision"="revision"+1,"managed"=$2,"updatedAt"=$3 WHERE "id"=$4 AND "revision"=$5"#,
        vec![encode(state,value)?.into(), value.holds_managed_slot().into(), now().into(), value.id.clone().into(), revision.into()],
    )).await?.rows_affected();
    if changed != 1 {
        return Err(ApiError::conflict(
            "Teams setup changed. Refresh and try again.",
        ));
    }
    Ok(())
}

pub(super) async fn managed_bots(
    state: &AppState,
    app: &str,
    excluding: &str,
) -> Result<i64, ApiError> {
    let row = state
        .db
        .query_one_raw(sql(
            r#"SELECT COUNT(*) AS count FROM "TeamsBotConnection" WHERE "appId"=$1 AND "managed" AND "id"<>$2"#,
            vec![app.into(), excluding.into()],
        ))
        .await?;
    Ok(row
        .map(|row| row.try_get::<i64>("", "count"))
        .transpose()?
        .unwrap_or_default())
}

pub(super) async fn get<T: DeserializeOwned>(
    state: &AppState,
    id: &str,
) -> Result<Option<(T, i32)>, ApiError> {
    let row = Row::find_by_statement(sql(
        r#"SELECT "data","revision" FROM "TeamsBotState" WHERE "id"=$1 AND "expiresAt">$2"#,
        vec![id.into(), now().into()],
    ))
    .one(&state.db)
    .await?;
    row.map(|row| Ok((decode(state, &row.data)?, row.revision)))
        .transpose()
}

pub(super) async fn insert<T: Serialize>(
    state: &AppState,
    id: &str,
    connection: &str,
    value: &T,
    expires: i64,
) -> Result<bool, ApiError> {
    Ok(state.db.execute_raw(sql(
        r#"INSERT INTO "TeamsBotState" ("id","connectionId","data","revision","expiresAt") VALUES ($1,$2,$3,0,$4) ON CONFLICT ("id") DO UPDATE SET "data"=EXCLUDED."data","revision"=0,"expiresAt"=EXCLUDED."expiresAt" WHERE "TeamsBotState"."expiresAt"<=$5"#,
        vec![id.into(),connection.into(),encode(state,value)?.into(),expires.into(),now().into()],
    )).await?.rows_affected() == 1)
}

pub(super) async fn update<T: Serialize>(
    state: &AppState,
    id: &str,
    value: &T,
    revision: i32,
) -> Result<bool, ApiError> {
    Ok(state.db.execute_raw(sql(
        r#"UPDATE "TeamsBotState" SET "data"=$1,"revision"="revision"+1 WHERE "id"=$2 AND "revision"=$3 AND "expiresAt">$4"#,
        vec![encode(state,value)?.into(),id.into(),revision.into(),now().into()],
    )).await?.rows_affected() == 1)
}

/// Like [`update`], and keeps the row alive until at least `expires`.
pub(super) async fn update_until<T: Serialize>(
    state: &AppState,
    id: &str,
    value: &T,
    revision: i32,
    expires: i64,
) -> Result<bool, ApiError> {
    Ok(state.db.execute_raw(sql(
        r#"UPDATE "TeamsBotState" SET "data"=$1,"revision"="revision"+1,"expiresAt"=GREATEST("expiresAt",$2) WHERE "id"=$3 AND "revision"=$4 AND "expiresAt">$5"#,
        vec![encode(state,value)?.into(),expires.into(),id.into(),revision.into(),now().into()],
    )).await?.rows_affected() == 1)
}

pub(crate) async fn sweep(state: &AppState) -> Result<u64, ApiError> {
    let started = Instant::now();
    let mut total = 0;
    loop {
        let deleted = state.db.execute_raw(sql(
            r#"DELETE FROM "TeamsBotState" WHERE "id" IN (SELECT "id" FROM "TeamsBotState" WHERE "expiresAt"<=$1 ORDER BY "expiresAt" LIMIT $2)"#,
            vec![now().into(), (SWEEP_BATCH as i64).into()],
        )).await?.rows_affected();
        total += deleted;
        if deleted < SWEEP_BATCH || started.elapsed() >= SWEEP_BUDGET {
            return Ok(total);
        }
    }
}

pub(super) async fn remove_connection(state: &AppState, id: &str) -> Result<(), ApiError> {
    // State expires independently. Revoke it immediately without an unbounded delete.
    loop {
        let deleted = state.db.execute_raw(sql(
            r#"DELETE FROM "TeamsBotState" WHERE "id" IN (SELECT "id" FROM "TeamsBotState" WHERE "connectionId"=$1 LIMIT 500)"#,
            vec![id.into()],
        )).await?.rows_affected();
        if deleted == 0 {
            break;
        }
    }
    state
        .db
        .execute_raw(sql(
            r#"DELETE FROM "TeamsBotConnection" WHERE "id"=$1"#,
            vec![id.into()],
        ))
        .await?;
    Ok(())
}

pub(crate) async fn app_event_ids(state: &AppState, app: &str) -> Result<Vec<String>, ApiError> {
    #[derive(FromQueryResult)]
    struct EventRow {
        event_id: String,
    }
    Ok(EventRow::find_by_statement(sql(
        r#"SELECT "eventId" AS event_id FROM "TeamsBotConnection" WHERE "appId"=$1 ORDER BY "id" LIMIT 20"#,
        vec![app.into()],
    )).all(&state.db).await?.into_iter().map(|row| row.event_id).collect())
}
