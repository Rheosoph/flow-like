use std::path::PathBuf;

use flow_like_storage_contracts::database::DatabaseSelector;
use flow_like_types::{Result, json::json};

use super::super::{VectorStore, buffered::BufferedVectorStore};
use super::LanceDBVectorStore;

struct TestDatabase(PathBuf);

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn database() -> Result<(TestDatabase, LanceDBVectorStore)> {
    let path =
        std::env::temp_dir().join(format!("flow-like-refs-{}", flow_like_types::create_id()));
    std::fs::create_dir_all(&path)?;
    let mut store = LanceDBVectorStore::new(path.clone(), "records".into()).await?;
    store
        .insert(vec![json!({"id": 1, "value": "first"})])
        .await?;
    Ok((TestDatabase(path), store))
}

fn version(branch: &str, version: u64) -> DatabaseSelector {
    DatabaseSelector {
        branch: branch.into(),
        version: Some(version),
        ..Default::default()
    }
}

async fn prune_now_for_test(store: &LanceDBVectorStore) -> Result<()> {
    // Exercise Lance's retained-file behavior immediately. Public cleanup keeps at least a day.
    store
        .raw()
        .await?
        .optimize(lancedb::table::OptimizeAction::Prune {
            older_than: Some(lancedb::table::Duration::zero()),
            delete_unverified: Some(false),
            error_if_tagged_old_versions: Some(false),
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn references_fail_closed_for_missing_or_conflicting_selectors() -> Result<()> {
    let (_guard, store) = database().await?;
    for selector in [
        DatabaseSelector {
            branch: "missing".into(),
            ..Default::default()
        },
        version("main", u64::MAX),
        DatabaseSelector {
            tag: Some("missing".into()),
            ..Default::default()
        },
        DatabaseSelector {
            version: Some(1),
            tag: Some("missing".into()),
            ..Default::default()
        },
        DatabaseSelector {
            branch: " ".into(),
            ..Default::default()
        },
        version("main", 0),
    ] {
        assert!(store.checkout(selector).await.is_err());
    }
    assert!(
        LanceDBVectorStore::from_connection_with_selector(
            store.connection()?.clone(),
            "missing_table".into(),
            version("main", 1)
        )
        .await
        .is_err()
    );
    assert_eq!(store.list_tables().await?, vec!["records"]);
    assert_eq!(store.count(None).await?, 1);
    Ok(())
}

#[tokio::test]
async fn references_keep_snapshots_and_branch_writes_independent() -> Result<()> {
    let (_guard, mut main) = database().await?;
    let first = main.reference().await?;
    let mut snapshot = main.checkout(version("main", first.version)).await?;
    let mut branch = snapshot.create_branch("experiment").await?;
    main.insert(vec![json!({"id": 2, "value": "main"})]).await?;
    branch
        .insert(vec![json!({"id": 3, "value": "branch"})])
        .await?;
    assert_eq!(snapshot.count(None).await?, 1);
    assert_eq!(main.count(None).await?, 2);
    assert_eq!(branch.filter("id = 3", None, 10, 0).await?.len(), 1);
    assert!(main.filter("id = 3", None, 10, 0).await?.is_empty());
    assert_eq!(main.reference().await?.branch, "main");
    assert_eq!(branch.reference().await?.branch, "experiment");
    assert!(
        snapshot
            .insert(vec![json!({"id": 4, "value": "bad"})])
            .await
            .is_err()
    );
    assert!(snapshot.delete("true").await.is_err());
    assert!(snapshot.purge().await.is_err());
    assert!(snapshot.add_column("bad", "1").await.is_err());
    assert!(snapshot.drop_columns(&["value"]).await.is_err());
    assert!(snapshot.index("id", Some("btree")).await.is_err());
    assert!(snapshot.drop_index("missing").await.is_err());
    assert!(snapshot.optimize(true).await.is_err());
    assert!(snapshot.cleanup_versions(0).await.is_err());
    assert!(snapshot.drop_table().await.is_err());
    assert!(branch.drop_table().await.is_err());
    let mut buffered = BufferedVectorStore::new(snapshot, 1_000);
    assert!(
        buffered
            .insert(vec![json!({"id": 4, "value": "bad"})])
            .await
            .is_err()
    );
    assert!(
        buffered
            .upsert(vec![json!({"id": 4, "value": "bad"})], "id".into())
            .await
            .is_err()
    );
    assert!(!buffered.is_dirty());
    assert_eq!(main.list_branches().await?.len(), 2);
    branch.purge().await?;
    assert_eq!(branch.count(None).await?, 0);
    assert_eq!(main.count(None).await?, 2);
    main.delete_branch("experiment").await?;
    assert_eq!(main.list_branches().await?.len(), 1);
    assert!(main.delete_branch("main").await.is_err());
    Ok(())
}

#[tokio::test]
async fn references_resolve_tags_once_and_preserve_them_on_reopen() -> Result<()> {
    let (_guard, main) = database().await?;
    let mut branch = main.create_branch("experiment").await?;
    branch
        .insert(vec![json!({"id": 2, "value": "branch"})])
        .await?;
    let first = branch.reference().await?;
    branch.create_tag("training").await?;
    let snapshot = main
        .checkout(DatabaseSelector {
            tag: Some("training".into()),
            ..Default::default()
        })
        .await?;
    assert_eq!(snapshot.selector(), version("experiment", first.version));
    branch
        .insert(vec![json!({"id": 3, "value": "new"})])
        .await?;
    branch.update_tag("training").await?;
    let reopened = snapshot.reopen(main.connection()?.clone()).await?;
    assert_eq!(reopened.reference().await?.version, first.version);
    assert_eq!(reopened.reference().await?.branch, "experiment");
    assert_eq!(reopened.count(None).await?, 2);
    assert_eq!(
        main.checkout(DatabaseSelector {
            tag: Some("training".into()),
            ..Default::default()
        })
        .await?
        .count(None)
        .await?,
        3
    );
    assert!(main.delete_branch("experiment").await.is_err());
    main.delete_tag("training").await?;
    main.delete_branch("experiment").await?;
    Ok(())
}

#[tokio::test]
async fn references_restore_commits_a_new_version_without_unpinning_readers() -> Result<()> {
    let (_guard, mut main) = database().await?;
    let snapshot = main
        .checkout(version("main", main.reference().await?.version))
        .await?;
    main.insert(vec![json!({"id": 2, "value": "later"})])
        .await?;
    let before = main.reference().await?;
    let restored = snapshot.restore().await?;
    assert!(restored.version > before.version);
    assert!(!restored.pinned);
    assert!(!restored.read_only);
    assert_eq!(snapshot.count(None).await?, 1);
    assert!(snapshot.reference().await?.pinned);
    let latest = main.checkout(DatabaseSelector::default()).await?;
    assert_eq!(latest.count(None).await?, 1);
    assert!(
        latest
            .list_versions()
            .await?
            .iter()
            .any(|v| v.version == before.version)
    );
    Ok(())
}

#[tokio::test]
async fn references_restore_branch_keeps_main_unchanged() -> Result<()> {
    let (_guard, main) = database().await?;
    let mut branch = main.create_branch("experiment").await?;
    branch
        .insert(vec![json!({"id": 2, "value": "branch"})])
        .await?;
    let before = branch.reference().await?;
    let snapshot = branch
        .checkout(version("experiment", before.version))
        .await?;
    branch
        .insert(vec![json!({"id": 3, "value": "later"})])
        .await?;
    let restored = snapshot.restore().await?;
    assert_eq!(restored.branch, "experiment");
    assert!(restored.version > before.version);
    assert_eq!(
        branch
            .reopen(main.connection()?.clone())
            .await?
            .count(None)
            .await?,
        2
    );
    assert_eq!(
        main.reopen(main.connection()?.clone())
            .await?
            .count(None)
            .await?,
        1
    );
    assert_eq!(snapshot.reference().await?.version, before.version);
    Ok(())
}

#[tokio::test]
async fn references_reject_all_sql_writes_on_snapshot_and_explicit_read_only_handles() -> Result<()>
{
    let (_guard, main) = database().await?;
    for selector in [
        version("main", main.reference().await?.version),
        DatabaseSelector {
            read_only: true,
            ..Default::default()
        },
    ] {
        let readonly = main.checkout(selector).await?;
        let ctx = datafusion::prelude::SessionContext::new();
        ctx.register_table("records", readonly.to_datafusion().await?)?;
        assert_eq!(
            ctx.sql("SELECT COUNT(*) FROM records")
                .await?
                .collect()
                .await?[0]
                .num_rows(),
            1
        );
        for sql in [
            "INSERT INTO records VALUES (2, 'bad')",
            "UPDATE records SET value = 'bad' WHERE id = 1",
            "DELETE FROM records WHERE id = 1",
        ] {
            let result = match ctx.sql(sql).await {
                Ok(frame) => frame.collect().await.map(|_| ()),
                Err(error) => Err(error),
            };
            assert!(result.is_err(), "read-only SQL executed: {sql}");
        }
        assert_eq!(main.count(None).await?, 1);
    }
    let readonly = main
        .checkout(DatabaseSelector {
            read_only: true,
            ..Default::default()
        })
        .await?;
    assert!(
        readonly
            .checkout(DatabaseSelector::default())
            .await?
            .ensure_writable()
            .is_err()
    );
    assert!(readonly.create_branch("forbidden").await.is_err());
    assert!(readonly.create_tag("forbidden").await.is_err());
    assert!(readonly.clone_table("forbidden").await.is_err());
    assert!(
        readonly
            .checkout(version("main", 1))
            .await?
            .restore()
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn references_cleanup_preserves_tags_and_branches() -> Result<()> {
    let (_guard, mut main) = database().await?;
    main.create_tag("retained").await?;
    let branch = main.create_branch("experiment").await?;
    main.insert(vec![json!({"id": 2, "value": "later"})])
        .await?;
    main.insert(vec![json!({"id": 3, "value": "latest"})])
        .await?;
    assert!(main.cleanup_versions(0).await.is_err());
    main.cleanup_versions(1).await?;
    prune_now_for_test(&main).await?;
    let tagged = main
        .checkout(DatabaseSelector {
            tag: Some("retained".into()),
            ..Default::default()
        })
        .await?;
    assert_eq!(tagged.count(None).await?, 1);
    assert_eq!(
        branch
            .reopen(main.connection()?.clone())
            .await?
            .count(None)
            .await?,
        1
    );
    assert_eq!(main.count(None).await?, 3);
    assert!(main.cleanup_versions(u64::MAX).await.is_err());
    Ok(())
}

#[tokio::test]
async fn references_clone_retains_branch_source_and_protects_shared_files() -> Result<()> {
    let (_guard, mut main) = database().await?;
    let mut branch = main.create_branch("experiment").await?;
    branch
        .insert(vec![json!({"id": 2, "value": "branch"})])
        .await?;
    let mut cloned = branch.clone_table("copy").await?;
    assert_eq!(cloned.count(None).await?, 2);
    assert_eq!(cloned.reference().await?.branch, "main");
    assert!(main.drop_table().await.is_err());
    let mut unopened = main.clone();
    unopened.table = None;
    assert!(unopened.drop_table().await.is_err());
    assert!(main.delete_branch("experiment").await.is_err());
    assert!(main.delete_tag("flow_clone_source_copy").await.is_err());
    assert!(main.update_tag("flow_clone_source_copy").await.is_err());
    assert!(main.clone_table("copy").await.is_err());
    branch.delete("id = 2").await?;
    branch.cleanup_versions(1).await?;
    prune_now_for_test(&branch).await?;
    assert_eq!(cloned.count(None).await?, 2);
    cloned
        .insert(vec![json!({"id": 3, "value": "copy"})])
        .await?;
    assert_eq!(branch.count(None).await?, 1);
    cloned.drop_table().await?;
    main.delete_tag("flow_clone_source_copy").await?;
    main.delete_branch("experiment").await?;
    main.drop_table().await?;
    Ok(())
}
