//! Assistant and Flux: the built-in bots every person has. They are created on the first list,
//! not at sign-up, so accounts that never open bots carry no rows.

use super::{model::BotSettings, validation::system_defaults};
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::bot,
    error::ApiError,
    routes::user::ensure_user_exists,
    state::AppState,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{bot::BotSystemKind, create_id};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, sea_query::OnConflict,
};

/// A new bot row with a fresh `bot_` id. `system` is set for Assistant and Flux only.
pub(super) fn new_bot_row(
    owner_id: &str,
    system: Option<BotSystemKind>,
    preset_id: Option<String>,
    settings: &BotSettings,
    now: DateTime<FixedOffset>,
) -> Result<bot::ActiveModel, ApiError> {
    let mut row = bot::ActiveModel {
        id: Set(format!("bot_{}", create_id())),
        owner_id: Set(owner_id.to_string()),
        system_key: Set(system.map(|kind| kind.as_str().to_string())),
        preset_id: Set(preset_id),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
        ..Default::default()
    };
    settings.write(&mut row)?;
    Ok(row)
}

fn list_rank(row: &bot::Model) -> u8 {
    match row.system_key.as_deref().and_then(BotSystemKind::parse) {
        Some(BotSystemKind::General) => 0,
        Some(BotSystemKind::Flux) => 1,
        None => 2,
    }
}

/// The owner's bots that are not deleted: Assistant, Flux, then the rest oldest first.
pub(super) async fn live_bots<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<Vec<bot::Model>, DbErr> {
    let mut bots = bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .order_by_asc(bot::Column::CreatedAt)
        .order_by_asc(bot::Column::Id)
        .all(db)
        .await?;
    bots.sort_by_key(list_rank);
    Ok(bots)
}

/// Inserts `kinds` for the owner unless they exist. Concurrent first visits both insert; the
/// unique (owner, key) index keeps one row per key and the loser inserts nothing.
pub(super) async fn insert_system_bots(
    db: &DatabaseConnection,
    dialect: DbDialect,
    owner_id: &str,
    kinds: &[BotSystemKind],
) -> Result<(), ApiError> {
    let now = chrono::Utc::now().fixed_offset();
    let rows = kinds
        .iter()
        .map(|kind| new_bot_row(owner_id, Some(*kind), None, &system_defaults(*kind), now))
        .collect::<Result<Vec<_>, _>>()?;
    retry_transaction(db, dialect, None, &RetryPolicy::default(), |txn| {
        let rows = rows.clone();
        Box::pin(async move {
            bot::Entity::insert_many(rows)
                .on_conflict(
                    OnConflict::columns([bot::Column::OwnerId, bot::Column::SystemKey])
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(txn)
                .await?;
            Ok::<_, ApiError>(())
        })
    })
    .await
}

/// The owner's bots, after creating the built-in bots they don't have yet.
pub(super) async fn bots_with_system(
    state: &AppState,
    owner_id: &str,
) -> Result<Vec<bot::Model>, ApiError> {
    let bots = live_bots(&state.db, owner_id).await?;
    let missing: Vec<BotSystemKind> = BotSystemKind::ALL
        .into_iter()
        .filter(|kind| {
            !bots
                .iter()
                .any(|row| row.system_key.as_deref() == Some(kind.as_str()))
        })
        .collect();
    if missing.is_empty() {
        return Ok(bots);
    }
    ensure_user_exists(state, owner_id).await?;
    insert_system_bots(&state.db, state.db_dialect, owner_id, &missing).await?;
    Ok(live_bots(&state.db, owner_id).await?)
}
