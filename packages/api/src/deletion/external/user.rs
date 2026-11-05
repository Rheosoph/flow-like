//! Work an account's deletion hands to other deletion jobs.

use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

use crate::deletion::{DeletionRoot, job};
use crate::entity::{app, bot};
use crate::error::ApiError;
use crate::state::AppState;

/// Queues the deletion of every workspace of the account's bots, deleted bots included. A
/// workspace is an app with storage, members and runs, so it goes through its own plan; the
/// account's job only hands it over. Re-runnable: a queued job is returned as it is.
pub async fn queue_bot_workspaces(state: &AppState, owner_id: &str) -> Result<usize, ApiError> {
    let workspaces: Vec<String> = app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(
            app::Column::BotId.in_subquery(
                Query::select()
                    .column(bot::Column::Id)
                    .from(bot::Entity)
                    .and_where(bot::Column::OwnerId.eq(owner_id))
                    .to_owned(),
            ),
        )
        .into_tuple()
        .all(&state.db)
        .await?;
    for app_id in &workspaces {
        job::enqueue(state, DeletionRoot::App, app_id, Some(owner_id)).await?;
    }
    Ok(workspaces.len())
}
