use flow_like::flow::execution::{
    ExecutionEnvironment,
    context::{ExecutionContext, ExecutionContextCache},
};
use flow_like_storage::{
    lance::session::Session,
    lancedb::{Connection, connection::ConnectBuilder},
    object_store::path::Path,
};
use flow_like_types::Cacheable;
use std::sync::{Arc, LazyLock};

#[path = "connection_cache.rs"]
mod connection_cache;

static CONNECTIONS: LazyLock<connection_cache::ConnectionCache<Connection>> =
    LazyLock::new(connection_cache::ConnectionCache::default);

fn connection_key(
    authority: [u8; 32],
    app_id: &str,
    subject: &str,
    user_scoped: bool,
    path: &Path,
) -> flow_like_types::Result<[u8; 32]> {
    let key =
        flow_like_types::json::to_vec(&(authority, app_id, subject, user_scoped, path.as_ref()))?;
    Ok(*blake3::hash(&key).as_bytes())
}

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

/// Reuse server connections only for the same fixed credential and caller scope.
/// Tables and their mutable snapshots remain in the run cache: opening them in
/// the next run resolves the current schema and version against storage again.
pub(crate) async fn open_shared(
    context: &ExecutionContext,
    user_scoped: bool,
) -> flow_like_types::Result<Connection> {
    let path = database_path(context, user_scoped)?;
    let scope = if user_scoped { "user" } else { "project" };
    let cache_key = format!("lance_connection_{scope}_{path}");
    let identity = if context.app_state.execution_environment == ExecutionEnvironment::Server {
        context
            .credentials
            .as_ref()
            .map(|credentials| credentials.database_cache_identity())
            .transpose()?
            .flatten()
    } else {
        None
    };
    if let Some(cached) = context
        .cache
        .read()
        .await
        .get(&cache_key)
        .and_then(|cached| cached.as_any().downcast_ref::<CachedConnection>())
    {
        return Ok(cached.0.clone());
    }

    let connection = if let Some((authority, expires_at)) = identity {
        let execution = execution_cache(context)?;
        let key = connection_key(
            authority,
            &execution.app_id,
            &execution.sub,
            user_scoped,
            &path,
        )?;
        let connection = CONNECTIONS
            .get_or_try_init(key, expires_at, || async {
                // The session must not share object stores or metadata with a
                // different authority. Sixteen entries cap retained Lance
                // caches at 320 MiB, excluding clients and active/evicted runs.
                let session = Arc::new(Session::new(
                    16 * 1024 * 1024,
                    4 * 1024 * 1024,
                    Arc::default(),
                ));
                let builder = database_builder(context, user_scoped, path, session).await?;
                Ok::<_, flow_like_types::Error>(builder.execute().await?)
            })
            .await?;
        if expires_at <= std::time::SystemTime::now() {
            return Err(flow_like_types::anyhow!(
                "Database storage credentials have expired"
            ));
        }
        connection.as_ref().clone()
    } else {
        database_builder(
            context,
            user_scoped,
            path,
            context.app_state.lance_session.clone(),
        )
        .await?
        .execute()
        .await?
    };
    let cacheable: Arc<dyn Cacheable> = Arc::new(CachedConnection(connection.clone()));
    context.cache.write().await.insert(cache_key, cacheable);
    Ok(connection)
}

async fn database_builder(
    context: &ExecutionContext,
    user_scoped: bool,
    path: Path,
    session: Arc<Session>,
) -> flow_like_types::Result<ConnectBuilder> {
    let execution = execution_cache(context)?;
    if let Some(credentials) = &context.credentials {
        return if user_scoped {
            credentials
                .to_db_scoped_with_session(&execution.sub, &execution.app_id, session)
                .await
        } else {
            credentials
                .to_db_with_session(&execution.app_id, session)
                .await
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
    Ok(build(path).session(session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_keys_separate_every_caller_and_storage_scope() {
        let path = Path::from("apps/app/storage/db");
        let base = connection_key([1; 32], "app", "user", false, &path).unwrap();
        for changed in [
            connection_key([2; 32], "app", "user", false, &path),
            connection_key([1; 32], "other-app", "user", false, &path),
            connection_key([1; 32], "app", "other-user", false, &path),
            connection_key([1; 32], "app", "user", true, &path),
            connection_key([1; 32], "app", "user", false, &Path::from("other/db")),
        ] {
            assert_ne!(base, changed.unwrap());
        }
        assert_eq!(
            base,
            connection_key([1; 32], "app", "user", false, &path).unwrap()
        );
    }

    #[tokio::test]
    async fn reused_connection_opens_the_latest_schema_after_external_writes() {
        use flow_like_storage::arrow_schema::{DataType, Field, Schema};
        use flow_like_storage::databases::vector::lancedb::connect_lance;

        let directory = tempfile::tempdir().unwrap();
        let uri = directory.path().to_str().unwrap();
        let cache = connection_cache::ConnectionCache::default();
        let expires = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let connection = cache
            .get_or_try_init([1; 32], expires, || async {
                connect_lance(uri)
                    .session(Arc::new(Session::new(
                        1024 * 1024,
                        1024 * 1024,
                        Arc::default(),
                    )))
                    .execute()
                    .await
            })
            .await
            .unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "original",
            DataType::Int32,
            false,
        )]));
        connection
            .create_empty_table("data", schema)
            .execute()
            .await
            .unwrap();
        let first = connection.open_table("data").execute().await.unwrap();
        assert_eq!(first.schema().await.unwrap().field(0).name(), "original");
        drop(first);
        drop(connection);

        let writer = connect_lance(uri).execute().await.unwrap();
        writer.drop_table("data", &[]).await.unwrap();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "replacement",
            DataType::Utf8,
            true,
        )]));
        writer
            .create_empty_table("data", schema)
            .execute()
            .await
            .unwrap();

        let reused = cache
            .get_or_try_init([1; 32], expires, || async {
                panic!("the second run should reuse its connection");
                #[allow(unreachable_code)]
                connect_lance(uri).execute().await
            })
            .await
            .unwrap();
        let latest = reused.open_table("data").execute().await.unwrap();
        assert_eq!(
            latest.schema().await.unwrap().field(0).name(),
            "replacement"
        );
    }
}
