use flow_like_storage::object_store::path::Path;
use futures_util::{StreamExt, stream};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, Query},
};

use super::delete_prefix;
use crate::deletion::drain::{CHUNK, Flow, Pass};
use crate::entity::{event_sink, execution_event, execution_run};
use crate::error::ApiError;
use crate::routes::sink::service::sink_types;
use crate::state::AppState;

/// Object deletes in flight while one page of references is cleared.
const PAYLOAD_DELETE_CONCURRENCY: usize = 16;

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

/// Remove the staged payload objects of the app's execution events.
///
/// Event payloads over the offload threshold live on the content store under a
/// path keyed by the *run*, not the app, so `ExecutionEvent.payloadRef` is the
/// only way to find them. Once those rows drain — or the run rows cascade them
/// away — the objects are unreachable, which is why this runs before the drain.
///
/// Each page clears the references it just deleted. That is what makes the step
/// terminate: a [`Pass`] carries no per-step cursor, so a suspended step starts
/// over, and only a shrinking remainder guarantees the next pass gets further.
pub async fn delete_execution_event_payloads(
    state: &AppState,
    app_id: &str,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    loop {
        let page: Vec<(String, Option<String>)> = execution_event::Entity::find()
            .select_only()
            .column(execution_event::Column::Id)
            .column(execution_event::Column::PayloadRef)
            .filter(execution_event::Column::PayloadRef.is_not_null())
            .filter(
                execution_event::Column::RunId.in_subquery(
                    Query::select()
                        .column(execution_run::Column::Id)
                        .from(execution_run::Entity)
                        .and_where(execution_run::Column::AppId.eq(app_id))
                        .to_owned(),
                ),
            )
            .order_by_asc(execution_event::Column::Id)
            .limit(CHUNK as u64)
            .into_tuple()
            .all(&state.db)
            .await?;
        if page.is_empty() {
            return Ok(Flow::Continue);
        }

        let store = state.content_bucket.clone();
        stream::iter(page.iter().filter_map(|(_, reference)| reference.clone()))
            .for_each_concurrent(PAYLOAD_DELETE_CONCURRENCY, |reference| {
                let store = store.clone();
                async move {
                    crate::execution::state::delete_staged_payload(&store, &reference).await;
                }
            })
            .await;

        let ids: Vec<String> = page.into_iter().map(|(id, _)| id).collect();
        let cleared = ids.len() as u64;
        execution_event::Entity::update_many()
            .col_expr(
                execution_event::Column::PayloadRef,
                Expr::value(Option::<String>::None),
            )
            .filter(execution_event::Column::Id.is_in(ids))
            .exec(&state.db)
            .await?;

        tracing::info!(app_id, cleared, "Deleted staged execution-event payloads");
        if pass.after_chunk(cleared).await? == Flow::Suspend {
            return Ok(Flow::Suspend);
        }
    }
}

/// `apps/{id}` on the meta and content stores and `media/apps/{id}` on the
/// content store, paginated and re-runnable.
pub async fn delete_storage_prefixes(
    state: &AppState,
    app_id: &str,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    let credentials = state.master_credentials().await?;
    let meta = credentials.to_store(true).await?.as_generic();
    let content = credentials.to_store(false).await?.as_generic();
    let app_prefix = Path::from("apps").child(app_id);
    let media_prefix = Path::from("media").child("apps").child(app_id);
    for (store, prefix, label) in [
        (&meta, &app_prefix, "meta"),
        (&content, &app_prefix, "content"),
        (&content, &media_prefix, "media"),
    ] {
        if delete_prefix(store, prefix, label, pass).await? == Flow::Suspend {
            return Ok(Flow::Suspend);
        }
    }
    Ok(Flow::Continue)
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
