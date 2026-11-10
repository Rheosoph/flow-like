//! Bots bound to a device that is no longer an active device of their owner. No host will ever
//! claim their turns, so the hub ends each one at once and the bot says why, instead of letting
//! it wait out the queue expiry.

use crate::{
    bot_host::{
        ledger::{self, ClaimRequest},
        worker,
    },
    error::ApiError,
    routes::inbox::now_millis,
    state::AppState,
};
use flow_like_types::bot_turn::{STORED_DEVICE, STORED_QUEUED, TurnHostKind};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const HOST_ID: &str = "hub:device-removed";
const BATCH: i64 = 32;
pub(crate) const DEVICE_REMOVED_TEXT: &str = "I can't answer: the device I run on was removed. My owner can choose another place for me to run in my settings.";

/// `$1` queued, `$2` the stored device kind (a bot's location and a turn's host kind share it),
/// `$3` the batch size.
const ORPHANED: &str = r#"SELECT t."id" AS "id", t."ownerId" AS "ownerId" FROM "BotTurn" t JOIN "Bot" b ON b."id" = t."botId" WHERE t."status" = $1 AND t."hostKind" = $2 AND b."deletedAt" IS NULL AND b."location" = $2 AND NOT EXISTS (SELECT 1 FROM "ManagedDevice" d WHERE d."id" = b."deviceId" AND d."ownerId" = b."ownerId" AND d."status" = 'active') ORDER BY t."createdAt", t."id" LIMIT $3"#;

/// The queued turns (id, owner) of live device bots whose device is revoked, missing or not
/// their owner's, oldest first.
pub(crate) async fn orphaned_in<C: ConnectionTrait>(
    db: &C,
    limit: i64,
) -> Result<Vec<(String, String)>, ApiError> {
    db.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        ORPHANED,
        [STORED_QUEUED.into(), STORED_DEVICE.into(), limit.into()],
    ))
    .await?
    .iter()
    .map(|row| -> Result<(String, String), ApiError> {
        Ok((row.try_get("", "id")?, row.try_get("", "ownerId")?))
    })
    .collect()
}

/// Ends the turns no device will claim: the hub takes each one and posts the reason as the bot.
/// A turn another claim reached first, or whose owner is at the running limit, waits for the
/// next pass. Returns how many it ended.
pub(crate) async fn fail_orphaned(state: &AppState) -> Result<u64, ApiError> {
    let mut ended = 0;
    for (turn_id, owner_id) in orphaned_in(&state.db, BATCH).await? {
        let claimed = ledger::claim(
            state,
            ClaimRequest {
                turn_id,
                owner_id: Some(owner_id),
                host_kind: TurnHostKind::Device,
                host_id: HOST_ID.into(),
                now: now_millis(),
            },
        )
        .await?;
        let Ok(claimed) = claimed else {
            continue;
        };
        let removed = worker::Failure::new(worker::CODE_SETUP, DEVICE_REMOVED_TEXT, false, true);
        worker::finish(state, &claimed.turn, HOST_ID, Err(removed), false).await;
        ended += 1;
    }
    Ok(ended)
}
