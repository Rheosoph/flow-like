use flow_like::flow::execution::context::{ExecutionContext, ExecutionContextCache};
use flow_like_storage::{
    lancedb::{Connection, connection::ConnectBuilder},
    object_store::path::Path,
};
use flow_like_types::Cacheable;
use std::sync::Arc;

#[derive(Clone)]
struct CachedConnection(Connection);

impl Cacheable for CachedConnection {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn execution_cache(context: &ExecutionContext) -> flow_like_types::Result<&ExecutionContextCache> {
    context
        .execution_cache
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("No execution cache found"))
}

pub(crate) fn database_path(
    context: &ExecutionContext,
    user_scoped: bool,
) -> flow_like_types::Result<Path> {
    let execution = execution_cache(context)?;
    let root = if user_scoped {
        execution.get_user_dir(false)?
    } else {
        execution.get_storage(false)?
    };
    Ok(root.join("db"))
}

/// Returns the run's LanceDB connection for the project or user database.
/// Credentials are fixed for a run, so the run cache already scopes them.
pub(crate) async fn open_shared(
    context: &ExecutionContext,
    user_scoped: bool,
) -> flow_like_types::Result<Connection> {
    let path = database_path(context, user_scoped)?;
    let scope = if user_scoped { "user" } else { "project" };
    let cache_key = format!("lance_connection_{scope}_{path}");
    if let Some(cached) = context
        .cache
        .read()
        .await
        .get(&cache_key)
        .and_then(|cached| cached.as_any().downcast_ref::<CachedConnection>())
    {
        return Ok(cached.0.clone());
    }

    let builder = database_builder(context, user_scoped, path).await?;
    let connection = context
        .app_state
        .with_lance_session(builder)
        .execute()
        .await?;
    let cacheable: Arc<dyn Cacheable> = Arc::new(CachedConnection(connection.clone()));
    context.cache.write().await.insert(cache_key, cacheable);
    Ok(connection)
}

async fn database_builder(
    context: &ExecutionContext,
    user_scoped: bool,
    path: Path,
) -> flow_like_types::Result<ConnectBuilder> {
    let execution = execution_cache(context)?;
    if let Some(credentials) = &context.credentials {
        return if user_scoped {
            credentials
                .to_db_scoped(&execution.sub, &execution.app_id)
                .await
        } else {
            credentials.to_db(&execution.app_id).await
        };
    }
    let callbacks = context.app_state.config.read().await.callbacks.clone();
    let (build, missing) = if user_scoped {
        (
            callbacks.build_user_database,
            "No user database builder found",
        )
    } else {
        (
            callbacks.build_project_database,
            "No database builder found",
        )
    };
    let build = build.ok_or_else(|| flow_like_types::anyhow!(missing))?;
    Ok(build(path))
}
