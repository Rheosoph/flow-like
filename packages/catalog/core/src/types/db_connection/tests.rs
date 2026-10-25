use super::*;
use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{LogLevel, internal_node::InternalNode},
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::databases::vector::lancedb::DatabaseSelector;
use flow_like_types::{Result, json::json, sync::Mutex};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct DatabaseTestNode;

#[async_trait]
impl NodeLogic for DatabaseTestNode {
    fn get_node(&self) -> Node {
        Node::new("database_test", "Database test", "", "Tests")
    }

    async fn run(&self, _: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }
}

async fn context() -> ExecutionContext {
    let logic: Arc<dyn NodeLogic> = Arc::new(DatabaseTestNode);
    let current = Arc::new(InternalNode::new(
        logic.get_node(),
        AHashMap::new(),
        logic,
        AHashMap::new(),
    ));
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    ExecutionContext::new(
        Arc::new(AHashMap::from_iter([(
            current.node_id().to_string(),
            current.clone(),
        )])),
        &Weak::new(),
        &state,
        &current,
        &Arc::new(Mutex::new(AHashMap::new())),
        &Arc::new(RwLock::new(AHashMap::new())),
        LogLevel::Debug,
        ExecutionStage::Dev,
        Arc::new(Profile::default()),
        None,
        Arc::new(RwLock::new(Vec::new())),
        None,
        None,
        Arc::new(AHashMap::new()),
        None,
    )
    .await
}

async fn database() -> Result<(TestDirectory, LanceDBVectorStore)> {
    let path = std::env::temp_dir().join(format!(
        "flow-like-cached-database-{}",
        flow_like_types::create_id()
    ));
    std::fs::create_dir_all(&path)?;
    let directory = TestDirectory(path);
    let mut store = LanceDBVectorStore::new(directory.0.clone(), "records".into()).await?;
    store
        .insert(vec![json!({"id": 1, "value": "initial"})])
        .await?;
    Ok((directory, store))
}

async fn cache(
    context: &ExecutionContext,
    key: &str,
    store: LanceDBVectorStore,
) -> NodeDBConnection {
    context
        .set_cache(
            key,
            Arc::new(CachedDB {
                db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 100))),
            }),
        )
        .await;
    NodeDBConnection {
        cache_key: key.into(),
    }
}

#[tokio::test]
async fn cached_readonly_latest_sees_committed_writes_and_schema_changes() -> Result<()> {
    let (_directory, store) = database().await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let writer = cache(&context, "writer", store).await;
    let (before, initial_generation) = reader.load_with_generation(&mut context).await?;
    assert_eq!(before.db.read().await.count(None).await?, 1);
    let old_provider = before.db.read().await.inner().to_datafusion().await?;

    let writer = writer.load(&mut context).await?;
    writer
        .insert_from(&context, vec![json!({"id": 2, "value": "committed"})])
        .await?;
    writer.ensure_flushed().await?;
    writer
        .db
        .read()
        .await
        .inner()
        .add_column("revision", "7")
        .await?;

    let (after, updated_generation) = reader.load_with_generation(&mut context).await?;
    assert!(Arc::ptr_eq(&before.db, &after.db));
    {
        let store = after.db.read().await;
        assert_eq!(store.count(None).await?, 2);
        assert_ne!(updated_generation, initial_generation);
        assert_eq!(
            store.filter("id = 2", None, 10, 0).await?,
            vec![json!({"id": 2, "value": "committed", "revision": 7})]
        );
        assert!(store.schema().await?.field_with_name("revision").is_ok());
        assert!(
            store
                .inner()
                .to_datafusion()
                .await?
                .schema()
                .field_with_name("revision")
                .is_ok()
        );
    }
    assert!(old_provider.schema().field_with_name("revision").is_err());
    assert_eq!(
        reader.load_with_generation(&mut context).await?.1,
        updated_generation
    );
    assert!(
        after
            .insert_from(&context, vec![json!({"id": 3, "value": "forbidden"})])
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn cached_version_and_resolved_tag_stay_pinned_after_writes() -> Result<()> {
    let (_directory, store) = database().await?;
    let original_version = store.reference().await?.version;
    store.create_tag("saved").await?;
    let pinned = store
        .checkout(DatabaseSelector {
            version: Some(original_version),
            read_only: true,
            ..Default::default()
        })
        .await?;
    let tagged = store
        .checkout(DatabaseSelector {
            tag: Some("saved".into()),
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let pinned = cache(&context, "pinned", pinned).await;
    let tagged = cache(&context, "tagged", tagged).await;
    let writer = cache(&context, "writer", store).await;
    let generations = [
        pinned.load_with_generation(&mut context).await?.1,
        tagged.load_with_generation(&mut context).await?.1,
    ];
    let writer = writer.load(&mut context).await?;
    writer
        .insert_from(&context, vec![json!({"id": 2, "value": "later"})])
        .await?;
    writer.ensure_flushed().await?;
    writer.db.read().await.inner().update_tag("saved").await?;

    for (reader, initial_generation) in [pinned, tagged].into_iter().zip(generations) {
        let (cached, generation) = reader.load_with_generation(&mut context).await?;
        assert_eq!(generation, initial_generation);
        let store = cached.db.read().await;
        assert_eq!(store.count(None).await?, 1);
        let reference = store.inner().reference().await?;
        assert_eq!(reference.version, original_version);
        assert!(reference.pinned && reference.read_only);
    }
    Ok(())
}

#[tokio::test]
async fn cached_readonly_latest_stays_on_its_selected_branch() -> Result<()> {
    let (_directory, store) = database().await?;
    let branch = store.create_branch("experiment").await?;
    let readonly = branch
        .checkout(DatabaseSelector {
            branch: "experiment".into(),
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let main = cache(&context, "main", store).await;
    let branch = cache(&context, "branch", branch).await;
    let initial_generation = reader.load_with_generation(&mut context).await?.1;

    let main = main.load(&mut context).await?;
    main.insert_from(&context, vec![json!({"id": 2, "value": "main"})])
        .await?;
    main.ensure_flushed().await?;
    let (unchanged, generation) = reader.load_with_generation(&mut context).await?;
    assert_eq!(generation, initial_generation);
    assert_eq!(unchanged.db.read().await.count(None).await?, 1);

    let branch = branch.load(&mut context).await?;
    branch
        .insert_from(&context, vec![json!({"id": 3, "value": "branch"})])
        .await?;
    branch.ensure_flushed().await?;
    let (updated, generation) = reader.load_with_generation(&mut context).await?;
    assert_ne!(generation, initial_generation);
    let store = updated.db.read().await;
    assert_eq!(store.inner().reference().await?.branch, "experiment");
    assert_eq!(store.count(None).await?, 2);
    assert_eq!(
        store.filter("id > 1", None, 10, 0).await?,
        vec![json!({"id": 3, "value": "branch"})]
    );
    Ok(())
}

#[tokio::test]
async fn cached_readonly_latest_flushes_buffered_upserts_and_repeated_inserts() -> Result<()> {
    let (_directory, store) = database().await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let writer = cache(&context, "writer", store).await;
    let (before, mut generation) = reader.load_with_generation(&mut context).await?;
    assert_eq!(before.db.read().await.count(None).await?, 1);
    let writer = writer.load(&mut context).await?;

    writer
        .upsert_from(
            &context,
            vec![json!({"id": 1, "value": "archived"})],
            "id".into(),
        )
        .await?;
    assert!(writer.has_buffered_writes().await);
    let (after, updated_generation) = reader.load_with_generation(&mut context).await?;
    assert!(Arc::ptr_eq(&before.db, &after.db));
    assert!(!writer.has_buffered_writes().await);
    assert_ne!(generation, updated_generation);
    generation = updated_generation;
    assert_eq!(
        after.db.read().await.filter("id = 1", None, 10, 0).await?,
        vec![json!({"id": 1, "value": "archived"})]
    );

    for id in [2, 3] {
        writer
            .insert_from(&context, vec![json!({"id": id, "value": "saved"})])
            .await?;
        assert!(writer.has_buffered_writes().await);
        let (after, updated_generation) = reader.load_with_generation(&mut context).await?;
        assert!(!writer.has_buffered_writes().await);
        assert_ne!(generation, updated_generation);
        generation = updated_generation;
        assert_eq!(after.db.read().await.count(None).await?, id as usize);
        assert_eq!(
            after
                .db
                .read()
                .await
                .filter(&format!("id = {id}"), None, 10, 0)
                .await?,
            vec![json!({"id": id, "value": "saved"})]
        );
    }
    Ok(())
}

#[tokio::test]
async fn cached_readonly_latest_sees_updates_and_deletes() -> Result<()> {
    let (_directory, store) = database().await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let writer = cache(&context, "writer", store).await;
    assert_eq!(
        reader
            .load(&mut context)
            .await?
            .db
            .read()
            .await
            .count(None)
            .await?,
        1
    );
    let writer = writer.load(&mut context).await?;

    writer
        .db
        .read()
        .await
        .inner()
        .update(
            "id = 1",
            HashMap::from([("value".into(), json!("archived"))]),
        )
        .await?;
    let updated = reader.load(&mut context).await?;
    assert_eq!(
        updated.db.read().await.list(None, 10, 0).await?,
        vec![json!({"id": 1, "value": "archived"})]
    );

    writer.db.read().await.delete("id = 1").await?;
    let deleted = reader.load(&mut context).await?;
    assert_eq!(deleted.db.read().await.count(None).await?, 0);
    Ok(())
}

#[tokio::test]
async fn cached_readonly_flushes_only_matching_writers_and_preserves_snapshot_reads() -> Result<()>
{
    let (_directory, store) = database().await?;
    let (_other_directory, other_database) = database().await?;
    let branch = store.create_branch("experiment").await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let pinned = store
        .checkout(DatabaseSelector {
            version: Some(store.reference().await?.version),
            read_only: true,
            ..Default::default()
        })
        .await?;
    store.create_tag("saved").await?;
    let tagged = store
        .checkout(DatabaseSelector {
            tag: Some("saved".into()),
            ..Default::default()
        })
        .await?;
    let mut other_table =
        LanceDBVectorStore::from_connection(store.connection()?.clone(), "other".into()).await;
    other_table
        .insert(vec![json!({"id": 1, "value": "initial"})])
        .await?;

    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let pinned = cache(&context, "pinned", pinned).await;
    let tagged = cache(&context, "tagged", tagged).await;
    let writer = cache(&context, "writer", store).await;
    let other_database = cache(&context, "other_database", other_database).await;
    let other_table = cache(&context, "other_table", other_table).await;
    let branch = cache(&context, "branch", branch).await;
    assert_eq!(
        reader
            .load(&mut context)
            .await?
            .db
            .read()
            .await
            .count(None)
            .await?,
        1
    );
    let writer = writer.load(&mut context).await?;
    let other_database = other_database.load(&mut context).await?;
    let other_table = other_table.load(&mut context).await?;
    let branch = branch.load(&mut context).await?;
    for cached in [&writer, &other_database, &other_table, &branch] {
        cached
            .insert_from(&context, vec![json!({"id": 2, "value": "pending"})])
            .await?;
    }

    for snapshot in [pinned, tagged] {
        let cached = snapshot.load(&mut context).await?;
        assert_eq!(cached.db.read().await.count(None).await?, 1);
        assert!(writer.has_buffered_writes().await);
    }
    let (metadata, _) = reader.load_unflushed_with_generation(&mut context).await?;
    assert_eq!(metadata.db.read().await.count(None).await?, 1);
    for cached in [&writer, &other_database, &other_table, &branch] {
        assert!(cached.has_buffered_writes().await);
    }

    let refreshed = reader.load(&mut context).await?;
    assert_eq!(refreshed.db.read().await.count(None).await?, 2);
    assert!(!writer.has_buffered_writes().await);
    for cached in [&other_database, &other_table, &branch] {
        assert!(cached.has_buffered_writes().await);
        assert_eq!(cached.db.read().await.inner().count(None).await?, 1);
    }
    Ok(())
}

#[tokio::test]
async fn cached_readonly_latest_propagates_buffered_write_failures_on_every_read() -> Result<()> {
    let (_directory, store) = database().await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let writer = cache(&context, "writer", store).await;
    reader.load(&mut context).await?;
    let writer = writer.load(&mut context).await?;
    writer.insert_from(&context, vec![json!({"id": 2})]).await?;
    assert!(writer.has_buffered_writes().await);

    for _ in 0..2 {
        let error = reader
            .load(&mut context)
            .await
            .err()
            .expect("a read must report the writer's rejected row");
        let failures = error
            .downcast_ref::<BufferedWriteError>()
            .expect("the rejected row should retain its write failure");
        assert_eq!(failures.failures.len(), 1);
        assert_eq!(failures.failures[0].operation, BufferedWriteKind::Insert);
        assert_eq!(
            failures.failures[0].origin.as_ref(),
            Some(&CachedDB::write_origin(&context))
        );
        assert!(!writer.has_buffered_writes().await);
        assert!(writer.db.read().await.has_write_failures());
    }
    assert_eq!(writer.db.read().await.inner().count(None).await?, 1);
    Ok(())
}

struct PendingWriterRefresher {
    writer: CachedDB,
    reject: AtomicBool,
    calls: AtomicUsize,
}

#[async_trait]
impl CachedDBRefresher for PendingWriterRefresher {
    async fn refresh(&self, _: &ExecutionContext) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(self.writer.has_buffered_writes().await);
        assert_eq!(self.writer.db.read().await.inner().count(None).await?, 1);
        if self.reject.load(Ordering::SeqCst) {
            return Err(anyhow!("writer credentials unavailable"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn cached_readonly_latest_refreshes_writer_before_flushing() -> Result<()> {
    let (_directory, store) = database().await?;
    let readonly = store
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    let mut context = context().await;
    let reader = cache(&context, "reader", readonly).await;
    let writer = cache(&context, "writer", store).await;
    reader.load(&mut context).await?;
    let cached_writer = writer.load(&mut context).await?;
    cached_writer
        .insert_from(&context, vec![json!({"id": 2, "value": "pending"})])
        .await?;
    let refresher = Arc::new(PendingWriterRefresher {
        writer: cached_writer.clone(),
        reject: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
    });
    context
        .set_cache(
            &NodeDBConnection::refresh_cache_key(&writer.cache_key),
            Arc::new(CachedDBRefreshHook::new(refresher.clone())),
        )
        .await;

    let error = reader
        .load(&mut context)
        .await
        .err()
        .expect("a failed writer refresh must prevent a stale read");
    assert!(error.to_string().contains("writer credentials unavailable"));
    assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    assert!(cached_writer.has_buffered_writes().await);

    refresher.reject.store(false, Ordering::SeqCst);
    let refreshed = reader.load(&mut context).await?;
    assert_eq!(refreshed.db.read().await.count(None).await?, 2);
    assert_eq!(refresher.calls.load(Ordering::SeqCst), 2);
    assert!(!cached_writer.has_buffered_writes().await);
    Ok(())
}
