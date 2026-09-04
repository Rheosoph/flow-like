use flow_like_storage::object_store::path::Path;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::delete_prefix;
use crate::entity::event_sink;
use crate::error::ApiError;
use crate::routes::sink::service::sink_types;
use crate::state::AppState;

/// Remove the external cron schedules of the app's sinks. Runs before the
/// `EventSink` rows drain, since they are the only record of which schedules
/// exist. Best effort per schedule: a missing schedule must not block the job.
pub async fn delete_sink_schedules(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    let Some(scheduler) = state.sink_scheduler.as_ref() else {
        return Ok(());
    };
    let sinks = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::SinkType.eq(sink_types::CRON))
        .all(&state.db)
        .await?;
    for sink in sinks {
        if let Err(error) = scheduler.delete_schedule(&sink.event_id).await {
            tracing::warn!(
                app_id,
                event_id = %sink.event_id,
                error = %error,
                "Failed to delete external schedule of an app being deleted"
            );
        }
    }
    Ok(())
}

/// `apps/{id}` on the meta and content stores and `media/apps/{id}` on the
/// content store, paginated and re-runnable.
pub async fn delete_storage_prefixes(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let meta = credentials.to_store(true).await?.as_generic();
    let content = credentials.to_store(false).await?.as_generic();
    let app_prefix = Path::from("apps").child(app_id);
    let media_prefix = Path::from("media").child("apps").child(app_id);
    delete_prefix(&meta, &app_prefix, "meta").await?;
    delete_prefix(&content, &app_prefix, "content").await?;
    delete_prefix(&content, &media_prefix, "media").await?;
    Ok(())
}

/// Best effort: entries on redis/dynamo/cosmos/firestore expire on their own,
/// and a failure here must not keep the job from finishing.
pub async fn delete_cache_backend(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    match state.cache.store().await {
        Ok(store) => match store.delete_app(app_id).await {
            Ok(deleted) if deleted > 0 => {
                tracing::info!(deleted, app_id, "Removed cache entries of deleted app");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = %error, app_id, "Failed to remove cache entries of deleted app");
            }
        },
        Err(error) => {
            tracing::warn!(error = %error, app_id, "Cache backend unavailable; cache entries of deleted app were not removed");
        }
    }
    Ok(())
}
