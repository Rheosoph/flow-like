use super::*;
use crate::databases::vector::buffered::BufferedVectorStore;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct TestDatabase(std::path::PathBuf);
impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct LocalAdapter {
    store: flow_like_types::sync::RwLock<LanceDBVectorStore>,
    sequence: AtomicU64,
    reject: AtomicBool,
    denied: AtomicBool,
}

#[async_trait]
impl LogicalTableMutationAdapter for LocalAdapter {
    async fn apply(&self, mutation: LogicalTableMutation) -> Result<LocalWriteReceipt> {
        if self.reject.load(Ordering::Acquire) {
            return Err(anyhow!("outbox capacity exceeded"));
        }
        let mut store = self.store.write().await;
        match mutation {
            LogicalTableMutation::Insert { items } => store.insert(items).await?,
            LogicalTableMutation::Upsert { items, id_field } => {
                store.upsert(items, id_field).await?;
            }
            LogicalTableMutation::Delete { filter } => store.delete(&filter).await?,
            LogicalTableMutation::Update { filter, updates } => {
                let table = store.raw().await?;
                let mut update = table.update().only_if(filter);
                for (column, expression) in updates {
                    update = update.column(column, expression);
                }
                update.execute().await?;
            }
        }
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        Ok(LocalWriteReceipt {
            operation_id: format!("operation-{sequence}"),
            sequence,
            state: "pending".into(),
        })
    }

    async fn read_table(&self) -> Result<Option<Table>> {
        if self.denied.load(Ordering::Acquire) {
            return Err(anyhow!("authorization denied"));
        }
        Ok(self.store.read().await.table.clone())
    }

    fn generation(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }
}

async fn fixture() -> Result<(TestDatabase, LanceDBVectorStore, Arc<LocalAdapter>)> {
    let directory = TestDatabase(std::env::temp_dir().join(format!(
        "flow-like-mutations-{}",
        flow_like_types::create_id()
    )));
    std::fs::create_dir(&directory.0)?;
    let store = LanceDBVectorStore::new(directory.0.join("mirror"), "rows".into()).await?;
    let adapter = Arc::new(LocalAdapter {
        store: flow_like_types::sync::RwLock::new(store.clone()),
        sequence: AtomicU64::new(0),
        reject: AtomicBool::new(false),
        denied: AtomicBool::new(false),
    });
    let store = LanceDBVectorStore::from_connection_for_overlay(
        store.connection()?.clone(),
        "rows".into(),
        DatabaseSelector::default(),
    )?;
    assert!(store.table.is_none());
    Ok((
        directory,
        store.with_mutation_adapter(adapter.clone()),
        adapter,
    ))
}

#[tokio::test]
async fn mounted_sql_provider_rechecks_authorization_without_a_generation_change() -> Result<()> {
    let (_directory, mut store, adapter) = fixture().await?;
    store
        .insert(vec![serde_json::json!({"id": 1, "value": "private"})])
        .await?;
    let session = SessionContext::new();
    let mounted = store.to_datafusion().await?;
    // Optimizers must not answer COUNT from retained statistics or inline an
    // unfenced native plan instead of calling the authorization-aware scan.
    assert!(mounted.statistics().is_none());
    assert!(mounted.get_logical_plan().is_none());
    session.register_table("rows", mounted)?;
    assert_eq!(
        session
            .sql("SELECT value FROM rows")
            .await?
            .collect()
            .await?[0]
            .num_rows(),
        1
    );
    let generation = store.mutation_generation();
    adapter.denied.store(true, Ordering::Release);
    assert_eq!(store.mutation_generation(), generation);
    for query in ["SELECT value FROM rows", "SELECT COUNT(*) FROM rows"] {
        let error = session.sql(query).await?.collect().await.unwrap_err();
        assert!(
            error.to_string().contains("authorization denied"),
            "{error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn deferred_overlay_open_preserves_readonly_and_rejects_cloud_selectors() -> Result<()> {
    let (_directory, mut writable, adapter) = fixture().await?;
    writable
        .insert(vec![serde_json::json!({"id": 1, "value": "local"})])
        .await?;
    let connection = adapter.store.read().await.connection()?.clone();
    for selector in [
        DatabaseSelector {
            branch: "other".into(),
            ..Default::default()
        },
        DatabaseSelector {
            version: Some(1),
            ..Default::default()
        },
        DatabaseSelector {
            tag: Some("tag".into()),
            ..Default::default()
        },
    ] {
        assert!(
            LanceDBVectorStore::from_connection_for_overlay(
                connection.clone(),
                "rows".into(),
                selector
            )
            .is_err()
        );
    }
    let mut readonly = LanceDBVectorStore::from_connection_for_overlay(
        connection,
        "rows".into(),
        DatabaseSelector {
            read_only: true,
            ..Default::default()
        },
    )?
    .with_mutation_adapter(adapter);
    assert_eq!(readonly.count(None).await?, 1);
    assert!(readonly.reference().await?.read_only);
    assert!(
        readonly
            .insert(vec![serde_json::json!({"id": 2, "value": "blocked"})])
            .await
            .is_err()
    );
    assert_eq!(writable.count(None).await?, 1);
    Ok(())
}

#[tokio::test]
async fn durable_mutations_bypass_volatile_batches_and_read_the_local_view() -> Result<()> {
    let (_directory, store, _adapter) = fixture().await?;
    assert!(!store.table_exists().await?);
    let mut buffered = BufferedVectorStore::new(store, 10_000);
    buffered
        .insert(vec![serde_json::json!({"id": 1, "value": "first"})])
        .await?;
    assert!(!buffered.is_dirty());
    assert_eq!(buffered.count(None).await?, 1);
    assert_eq!(buffered.inner().last_write_receipt().unwrap().sequence, 1);

    buffered
        .upsert(
            vec![serde_json::json!({"id": 1, "value": "second"})],
            "id".into(),
        )
        .await?;
    assert_eq!(
        buffered.filter("id = 1", None, 10, 0).await?[0]["value"],
        "second"
    );
    buffered
        .inner()
        .update(
            "id = 1",
            std::collections::HashMap::from([("value".into(), serde_json::json!("third"))]),
        )
        .await?;
    assert_eq!(buffered.list(None, 10, 0).await?[0]["value"], "third");
    let result = buffered
        .inner()
        .sql("rows", "SELECT value FROM rows WHERE id = 1")
        .await?
        .collect()
        .await?;
    assert_eq!(record_batches_to_vec(Some(result))?[0]["value"], "third");

    buffered.delete("id = 1").await?;
    assert_eq!(buffered.count(None).await?, 0);
    assert_eq!(buffered.inner().last_write_receipt().unwrap().sequence, 4);
    assert_eq!(buffered.inner().mutation_generation(), 4);
    buffered.flush().await?;
    assert!(!buffered.is_dirty());
    Ok(())
}

#[tokio::test]
async fn rejected_durable_enqueue_is_not_hidden_in_a_volatile_buffer() -> Result<()> {
    let (_directory, store, adapter) = fixture().await?;
    let mut buffered = BufferedVectorStore::new(store, 10_000);
    adapter.reject.store(true, Ordering::Release);
    let error = buffered
        .insert(vec![serde_json::json!({"id": 1})])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outbox capacity exceeded"));
    assert!(!buffered.is_dirty());
    assert!(buffered.inner().last_write_receipt().is_none());
    assert!(!buffered.inner().table_exists().await?);
    Ok(())
}

#[tokio::test]
async fn managed_tables_reject_raw_and_untracked_mutations() -> Result<()> {
    let (_directory, mut store, _adapter) = fixture().await?;
    store
        .insert(vec![serde_json::json!({"id": 1, "value": "one"})])
        .await?;
    assert!(store.raw().await.is_err());
    assert!(store.connection().is_err());
    assert!(store.checkout(DatabaseSelector::default()).await.is_err());
    assert!(store.drop_table().await.is_err());
    assert!(store.add_column("extra", "1").await.is_err());
    assert!(store.index("id", None).await.is_err());
    assert!(store.optimize(true).await.is_err());
    for sql in [
        "INSERT INTO rows (id, value) VALUES (2, 'two')",
        "UPDATE rows SET value = 'changed' WHERE id = 1",
        "DELETE FROM rows WHERE id = 1",
    ] {
        let failed = match store.sql("rows", sql).await {
            Err(_) => true,
            Ok(query) => query.collect().await.is_err(),
        };
        assert!(failed, "untracked SQL mutation succeeded: {sql}");
    }
    assert_eq!(
        store.list(None, 10, 0).await?,
        vec![serde_json::json!({"id": 1, "value": "one"})]
    );
    assert_eq!(store.last_write_receipt().unwrap().sequence, 1);
    Ok(())
}
