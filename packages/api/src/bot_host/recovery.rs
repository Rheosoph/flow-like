//! Checkpoints belong to the current lease generation, while their deadline belongs to the run.

use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{bot, bot_turn, bot_turn_checkpoint, conversation},
    error::ApiError,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like::flow::copilot::run_recovery::BotRunRecovery;
use flow_like_types::bot_turn::{
    BotRunCheckpoint, INTERACTIVE_HOST_PREFIX, STORED_DONE, STORED_QUEUED, STORED_RUNNING,
    TurnHostKind,
};
use flow_like_types::inbox::{ConversationEncryption, EnvelopeError, EnvelopeKind, check_envelope};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QuerySelect, QueryTrait,
    sea_query::{Expr, ExprTrait, OnConflict},
};
use std::sync::atomic::{AtomicI64, Ordering};

pub(crate) const CONFLICT: &str = "BOT_CHECKPOINT_CONFLICT";

pub(crate) use super::monitor_baseline::previous_result_for_turn as previous_monitor_result_for_turn;

pub(crate) async fn previous_monitor_result_in<C: ConnectionTrait>(
    db: &C,
    claim: &Claim,
    now: DateTime<FixedOffset>,
) -> Result<Option<String>, ApiError> {
    let current = bot_turn::Entity::find()
        .filter(claim.held(now))
        .one(db)
        .await?
        .ok_or_else(|| claim.lost())?;
    previous_monitor_result_for_turn(db, &current).await
}

/// Claim generations never reset. This separate counter limits automatic failure recovery.
pub(crate) async fn ensure_metadata<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<(), sea_orm::DbErr> {
    bot_turn_checkpoint::Entity::insert(bot_turn_checkpoint::ActiveModel {
        turn_id: Set(turn.id.clone()),
        revision: Set(0),
        claim_attempt: Set(0),
        recovery_attempts: Set(0),
        started_at_ms: Set(0),
        deadline_at_ms: Set(0),
        state: Set(serde_json::json!({})),
        updated_at: Set(turn.updated_at),
    })
    .on_conflict(
        OnConflict::column(bot_turn_checkpoint::Column::TurnId)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

pub(crate) async fn failures<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<i32, sea_orm::DbErr> {
    ensure_metadata(db, turn).await?;
    Ok(bot_turn_checkpoint::Entity::find_by_id(&turn.id)
        .one(db)
        .await?
        .map_or(0, |row| row.recovery_attempts))
}

/// The caller holds the turn row in its transaction, preventing a second charge for this claim.
pub(crate) async fn charge_failure<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<i32, sea_orm::DbErr> {
    ensure_metadata(db, turn).await?;
    bot_turn_checkpoint::Entity::update_many()
        .col_expr(
            bot_turn_checkpoint::Column::RecoveryAttempts,
            Expr::col(bot_turn_checkpoint::Column::RecoveryAttempts).add(1),
        )
        .filter(bot_turn_checkpoint::Column::TurnId.eq(&turn.id))
        .exec(db)
        .await?;
    failures(db, turn).await
}

#[derive(Clone, Debug)]
pub(crate) struct Claim {
    pub turn_id: String,
    pub owner_id: String,
    pub host_id: String,
    pub attempt: u32,
}

impl Claim {
    pub fn interactive(&self) -> bool {
        self.attempt == 0 && self.host_id.starts_with(INTERACTIVE_HOST_PREFIX)
    }
    fn held(&self, now: DateTime<FixedOffset>) -> Condition {
        let held = Condition::all()
            .add(bot_turn::Column::Id.eq(&self.turn_id))
            .add(bot_turn::Column::OwnerId.eq(&self.owner_id))
            .add(
                bot_turn::Column::BotId.in_subquery(
                    bot::Entity::find()
                        .select_only()
                        .column(bot::Column::Id)
                        .filter(bot::Column::DeletedAt.is_null())
                        .into_query(),
                ),
            )
            .add(bot_turn::Column::Attempts.eq(i32::try_from(self.attempt).unwrap_or(-1)))
            .add(bot_turn::Column::LeaseUntil.gt(now));
        if self.interactive() {
            held.add(bot_turn::Column::Status.eq(STORED_QUEUED))
                .add(bot_turn::Column::HostKind.eq(TurnHostKind::Desktop.stored()))
                .add(
                Condition::any()
                    .add(bot_turn::Column::HostId.is_null())
                    .add(bot_turn::Column::HostId.eq(&self.host_id)),
            )
        } else {
            held.add(bot_turn::Column::Status.eq(STORED_RUNNING))
                .add(bot_turn::Column::HostId.eq(&self.host_id))
        }
    }

    fn lost(&self) -> ApiError {
        ApiError::coded(
            StatusCode::CONFLICT,
            super::ledger::BOT_TURN_LEASE_LOST,
            "The current turn lease is required to access its checkpoint",
        )
    }
}

fn checkpoint(row: bot_turn_checkpoint::Model) -> Result<BotRunCheckpoint, ApiError> {
    let saved = BotRunCheckpoint {
        revision: u64::try_from(row.revision).unwrap_or(0),
        started_at_ms: row.started_at_ms,
        deadline_at_ms: row.deadline_at_ms,
        state: row.state,
    };
    saved.validate().map_err(ApiError::bad_request)?;
    Ok(saved)
}

pub(crate) async fn load_in<C: ConnectionTrait>(
    db: &C,
    claim: &Claim,
    now: DateTime<FixedOffset>,
) -> Result<Option<BotRunCheckpoint>, ApiError> {
    if bot_turn::Entity::find()
        .filter(claim.held(now))
        .one(db)
        .await?
        .is_none()
    {
        return Err(claim.lost());
    }
    bot_turn_checkpoint::Entity::find_by_id(&claim.turn_id)
        .one(db)
        .await?
        .filter(|row| row.revision > 0)
        .map(checkpoint)
        .transpose()
}

fn conflict() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        CONFLICT,
        "Checkpoint revision or original execution deadline changed",
    )
}

/// Call inside a transaction. Updating the lease row locks it against cancellation or takeover
/// until the checkpoint commits, without extending the lease.
pub(crate) async fn save_in<C: ConnectionTrait>(
    db: &C,
    claim: &Claim,
    saved: &BotRunCheckpoint,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    saved.validate().map_err(ApiError::bad_request)?;
    // Serialize against soft deletion so retirement cannot purge a checkpoint and then have
    // an already-running writer recreate it after the bot was deleted.
    let alive = bot::Entity::update_many()
        .col_expr(bot::Column::UpdatedAt, Expr::col(bot::Column::UpdatedAt))
        .filter(bot::Column::DeletedAt.is_null())
        .filter(
            bot::Column::Id.in_subquery(
                bot_turn::Entity::find()
                    .select_only()
                    .column(bot_turn::Column::BotId)
                    .filter(claim.held(now))
                    .into_query(),
            ),
        )
        .exec(db)
        .await?;
    if alive.rows_affected != 1 {
        return Err(claim.lost());
    }
    let mut update = bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::UpdatedAt,
            Expr::col(bot_turn::Column::UpdatedAt),
        )
        .filter(claim.held(now));
    if claim.interactive() {
        update = update
            .col_expr(
                bot_turn::Column::HostId,
                Expr::value(claim.host_id.as_str()),
            )
            .col_expr(
                bot_turn::Column::ErrorCode,
                Expr::value(super::ledger::ERROR_RESUMING),
            );
    }
    let held = update.exec(db).await?;
    if held.rows_affected != 1 {
        return Err(claim.lost());
    }
    let turn = bot_turn::Entity::find_by_id(&claim.turn_id)
        .one(db)
        .await?
        .ok_or_else(|| claim.lost())?;
    let room = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(db)
        .await?;
    if room
        .as_ref()
        .and_then(|room| ConversationEncryption::parse(&room.encryption))
        == Some(ConversationEncryption::E2e)
    {
        let state = saved
            .state
            .as_object()
            .expect("validated checkpoint object");
        let sealed = state.len() == 2
            && state
                .get("sealed_checkpoint")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
            && state
                .get("envelope")
                .is_some_and(|envelope| match check_envelope(envelope) {
                    Ok(EnvelopeKind::Sealed) => true,
                    // Checkpoint size is validated separately; inbox messages have a smaller cap.
                    Err(EnvelopeError::TooLarge(_)) => envelope
                        .get("enc")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|enc| enc != "none"),
                    _ => false,
                });
        if !sealed {
            return Err(ApiError::bad_request(
                "An end-to-end encrypted turn requires a sealed checkpoint",
            ));
        }
    }
    let existing = bot_turn_checkpoint::Entity::find_by_id(&claim.turn_id)
        .one(db)
        .await?;
    if let Some(row) = existing.as_ref().filter(|row| row.revision > 0) {
        let previous = checkpoint(row.clone())?;
        // A response can be lost after commit. Retrying the identical save is safe.
        if previous == *saved {
            return Ok(());
        }
        if saved.revision != previous.revision + 1
            || saved.started_at_ms != previous.started_at_ms
            || saved.deadline_at_ms != previous.deadline_at_ms
        {
            return Err(conflict());
        }
        let changed = bot_turn_checkpoint::Entity::update_many()
            .col_expr(
                bot_turn_checkpoint::Column::Revision,
                Expr::value(saved.revision as i64),
            )
            .col_expr(
                bot_turn_checkpoint::Column::ClaimAttempt,
                Expr::value(claim.attempt as i32),
            )
            .col_expr(
                bot_turn_checkpoint::Column::State,
                Expr::value(saved.state.clone()),
            )
            .col_expr(bot_turn_checkpoint::Column::UpdatedAt, Expr::value(now))
            .filter(bot_turn_checkpoint::Column::TurnId.eq(&claim.turn_id))
            .filter(bot_turn_checkpoint::Column::Revision.eq(previous.revision as i64))
            .exec(db)
            .await?;
        if changed.rows_affected != 1 {
            return Err(conflict());
        }
    } else {
        if saved.revision != 1 {
            return Err(conflict());
        }
        if saved.started_at_ms > now.timestamp_millis().saturating_add(60_000) {
            return Err(ApiError::bad_request(
                "Checkpoint start time is ahead of the server clock",
            ));
        }
        if existing.is_some() {
            let changed = bot_turn_checkpoint::Entity::update_many()
                .col_expr(
                    bot_turn_checkpoint::Column::Revision,
                    Expr::value(saved.revision as i64),
                )
                .col_expr(
                    bot_turn_checkpoint::Column::ClaimAttempt,
                    Expr::value(claim.attempt as i32),
                )
                .col_expr(
                    bot_turn_checkpoint::Column::StartedAtMs,
                    Expr::value(saved.started_at_ms),
                )
                .col_expr(
                    bot_turn_checkpoint::Column::DeadlineAtMs,
                    Expr::value(saved.deadline_at_ms),
                )
                .col_expr(
                    bot_turn_checkpoint::Column::State,
                    Expr::value(saved.state.clone()),
                )
                .col_expr(bot_turn_checkpoint::Column::UpdatedAt, Expr::value(now))
                .filter(bot_turn_checkpoint::Column::TurnId.eq(&claim.turn_id))
                .filter(bot_turn_checkpoint::Column::Revision.eq(0))
                .exec(db)
                .await?;
            if changed.rows_affected != 1 {
                return Err(conflict());
            }
            return Ok(());
        }
        bot_turn_checkpoint::ActiveModel {
            turn_id: Set(claim.turn_id.clone()),
            revision: Set(saved.revision as i64),
            claim_attempt: Set(claim.attempt as i32),
            recovery_attempts: Set(0),
            started_at_ms: Set(saved.started_at_ms),
            deadline_at_ms: Set(saved.deadline_at_ms),
            state: Set(saved.state.clone()),
            updated_at: Set(now),
        }
        .insert(db)
        .await?;
    }
    Ok(())
}

/// The only deadline change: an owner's approval answer or retry grants the saved turn a fresh
/// runtime. Revision and state stay, because sealed checkpoints bind the revision.
pub(crate) async fn rebase_deadline_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), sea_orm::DbErr> {
    let started = now.timestamp_millis();
    bot_turn_checkpoint::Entity::update_many()
        .col_expr(bot_turn_checkpoint::Column::StartedAtMs, Expr::value(started))
        .col_expr(
            bot_turn_checkpoint::Column::DeadlineAtMs,
            Expr::value(started.saturating_add(super::worker::MAX_TURN_RUNTIME_MS)),
        )
        .filter(bot_turn_checkpoint::Column::TurnId.eq(turn_id))
        .filter(bot_turn_checkpoint::Column::Revision.gt(0))
        .exec(db)
        .await?;
    Ok(())
}

/// A finished turn keeps no checkpoint: `DONE` and `CANCELLED` are final, while `FAILED` keeps
/// its checkpoint so the owner can retry. A `DONE` monitoring turn keeps its checkpoint, the
/// baseline of the next check.
pub(crate) async fn clear_finished_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
) -> Result<(), sea_orm::DbErr> {
    let Some(turn) = bot_turn::Entity::find_by_id(turn_id).one(db).await? else {
        return Ok(());
    };
    if turn.status == STORED_DONE && super::monitor_baseline::is_monitoring(db, &turn).await? {
        return Ok(());
    }
    clear_in(db, turn_id).await
}

pub(crate) async fn clear_in<C: ConnectionTrait>(
    db: &C,
    turn_id: &str,
) -> Result<(), sea_orm::DbErr> {
    bot_turn_checkpoint::Entity::delete_many()
        .filter(bot_turn_checkpoint::Column::TurnId.eq(turn_id))
        .exec(db)
        .await?;
    Ok(())
}

pub(crate) async fn save(
    db: &DatabaseConnection,
    dialect: DbDialect,
    claim: &Claim,
    saved: &BotRunCheckpoint,
) -> Result<(), ApiError> {
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), |txn| {
        let (claim, saved) = (claim.clone(), saved.clone());
        Box::pin(async move { save_in(txn, &claim, &saved, now_millis()).await })
    })
    .await
}

pub(crate) struct ServerRecovery {
    db: DatabaseConnection,
    dialect: DbDialect,
    claim: Claim,
    deadline: AtomicI64,
}

impl std::fmt::Debug for ServerRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRecovery")
            .field("claim", &self.claim)
            .finish_non_exhaustive()
    }
}

impl ServerRecovery {
    pub fn new(state: &AppState, turn: &bot_turn::Model, host_id: &str) -> Self {
        Self {
            db: state.db.clone(),
            dialect: state.db_dialect,
            claim: Claim {
                turn_id: turn.id.clone(),
                owner_id: turn.owner_id.clone(),
                host_id: host_id.to_owned(),
                attempt: u32::try_from(turn.attempts).unwrap_or(0),
            },
            deadline: AtomicI64::new(0),
        }
    }
    pub fn set_initial_deadline(&self, deadline: i64) {
        let _ = self
            .deadline
            .compare_exchange(0, deadline, Ordering::Relaxed, Ordering::Relaxed);
    }
    pub async fn read(&self) -> Result<Option<BotRunCheckpoint>, ApiError> {
        let saved = load_in(&self.db, &self.claim, now_millis()).await?;
        if let Some(saved) = &saved {
            self.set_initial_deadline(saved.deadline_at_ms);
        }
        Ok(saved)
    }
}

#[async_trait::async_trait]
impl BotRunRecovery for ServerRecovery {
    async fn load(&self) -> flow_like_types::Result<Option<BotRunCheckpoint>> {
        self.read()
            .await
            .map_err(|error| flow_like_types::anyhow!("{error}"))
    }
    async fn save(&self, checkpoint: &BotRunCheckpoint) -> flow_like_types::Result<()> {
        save(&self.db, self.dialect, &self.claim, checkpoint)
            .await
            .map_err(|error| flow_like_types::anyhow!("{error}"))
    }
    fn deadline_at_ms(&self) -> Option<i64> {
        let deadline = self.deadline.load(Ordering::Relaxed);
        (deadline != 0).then_some(deadline)
    }
}
