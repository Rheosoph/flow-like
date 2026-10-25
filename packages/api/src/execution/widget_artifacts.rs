use super::compiled_artifacts::{
    EnsuredArtifact, draft_object_trusted_until, refresh_draft_artifact, stored_artifact_written,
};
use flow_like::a2ui::widget::Widget;
use flow_like::flow::compiled::{self, ArtifactWidgetEntry, ArtifactWidgetSnapshot};
use flow_like::utils::compression::{from_compressed_json_with_meta, from_compressed_with_meta};
use flow_like_storage::Path;
use flow_like_storage::object_store::{
    Error as StoreError, ObjectMeta, ObjectStore, ObjectStoreExt,
};
use flow_like_types::{Result, anyhow};
use futures::{StreamExt, TryStreamExt};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, SystemTime},
};

const MAX_WIDGETS: usize = 64;
const READ_CONCURRENCY: usize = 8;
const MAX_BASE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INVENTORY_BYTES: usize = 16 * 1024 * 1024;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(250);

struct BaseArtifact {
    bytes: Vec<u8>,
    selectors: Vec<String>,
}

struct CachedWidget {
    widget: Widget,
    bytes: u32,
}

/// Every cached body is keyed by the revision read from storage. Dispatch still
/// checks current revisions, including name-resolution candidates in manifest order.
pub(crate) struct WidgetArtifactCache {
    bases: moka::sync::Cache<String, Arc<BaseArtifact>>,
    manifests: moka::sync::Cache<String, Arc<Vec<String>>>,
    widgets: moka::sync::Cache<String, Arc<CachedWidget>>,
    bundles: moka::sync::Cache<String, SystemTime>,
}

impl Default for WidgetArtifactCache {
    fn default() -> Self {
        Self {
            bases: moka::sync::Cache::builder()
                .max_capacity(64 * 1024 * 1024)
                .weigher(|_: &String, value: &Arc<BaseArtifact>| value.bytes.len() as u32)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            manifests: moka::sync::Cache::builder()
                .max_capacity(256)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            widgets: moka::sync::Cache::builder()
                .max_capacity(32 * 1024 * 1024)
                .weigher(|_: &String, value: &Arc<CachedWidget>| value.bytes)
                .time_to_idle(Duration::from_secs(30 * 60))
                .build(),
            bundles: moka::sync::Cache::builder()
                .max_capacity(4_096)
                .time_to_idle(Duration::from_secs(12 * 60 * 60))
                .build(),
        }
    }
}

fn cache_key(path: &Path, revision: &str) -> String {
    // JSON framing prevents object paths and opaque ETags from aliasing one another.
    serde_json::to_string(&(path.as_ref(), revision)).expect("strings serialize")
}

fn etag(meta: &ObjectMeta) -> Result<&str> {
    meta.e_tag
        .as_deref()
        .filter(|etag| !etag.is_empty())
        .ok_or_else(|| anyhow!("widget dependency has no storage revision"))
}

fn require_revision(head: &ObjectMeta, read: &ObjectMeta) -> Result<()> {
    if etag(head)? != etag(read)? {
        return Err(anyhow!(
            "widget dependency changed during snapshot creation"
        ));
    }
    Ok(())
}

fn widget_path(app_id: &str, widget_id: &str) -> Result<Path> {
    if widget_id.is_empty()
        || matches!(widget_id, "." | "..")
        || widget_id.contains('/')
        || widget_id.contains('\\')
    {
        return Err(anyhow!("widget inventory contains an invalid identifier"));
    }
    Ok(Path::from("apps")
        .join(app_id)
        .join(format!("{widget_id}.widget")))
}

fn needed_inventory<'a>(selectors: &[String], ids: &'a [String]) -> Result<Vec<&'a String>> {
    if selectors.len() > MAX_WIDGETS {
        return Err(anyhow!("widget selector count exceeds the snapshot limit"));
    }
    let needs_names = selectors.iter().any(|selector| !ids.contains(selector));
    let mut seen = BTreeSet::new();
    let needed: Vec<_> = ids
        .iter()
        .filter(|id| needs_names || selectors.contains(id))
        .filter(|id| seen.insert(id.as_str()))
        .collect();
    if needed.len() > MAX_WIDGETS {
        return Err(anyhow!(
            "widget name lookup exceeds the snapshot inventory limit"
        ));
    }
    Ok(needed)
}

fn revision(
    app_id: &str,
    board_id: &str,
    board_identity: &str,
    base: &EnsuredArtifact,
    manifest_etag: &str,
    selectors: &[String],
    dependencies: &[(String, Option<ObjectMeta>)],
) -> Result<String> {
    let dependencies: Vec<_> = dependencies
        .iter()
        .map(|(id, meta)| Ok((id, meta.as_ref().map(etag).transpose()?)))
        .collect::<Result<_>>()?;
    let bytes = serde_json::to_vec(&(
        "flow-like/widget-artifact/v1",
        app_id,
        board_id,
        board_identity,
        base.path.as_ref(),
        base.registry_fingerprint,
        manifest_etag,
        selectors,
        dependencies,
    ))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

impl WidgetArtifactCache {
    pub(crate) fn seed_base(
        &self,
        path: &Path,
        fingerprint: [u8; 32],
        board: &compiled::CompiledBoard,
        bytes: &[u8],
    ) {
        if bytes.len() as u64 > MAX_BASE_BYTES {
            return;
        }
        let fingerprint = blake3::Hash::from_bytes(fingerprint).to_hex();
        self.bases.insert(
            cache_key(path, &fingerprint),
            Arc::new(BaseArtifact {
                bytes: bytes.to_vec(),
                selectors: compiled::static_widget_selectors(board),
            }),
        );
    }

    async fn base(
        &self,
        store: &Arc<dyn ObjectStore>,
        base: &EnsuredArtifact,
        board_id: &str,
    ) -> Result<Arc<BaseArtifact>> {
        let key = cache_key(
            &base.path,
            &blake3::Hash::from_bytes(base.registry_fingerprint).to_hex(),
        );
        if let Some(base) = self.bases.get(&key) {
            return Ok(base);
        }
        let read = store.get(&base.path).await?;
        if read.meta.size > MAX_BASE_BYTES {
            return Err(anyhow!(
                "board artifact exceeds the widget snapshot cache limit"
            ));
        }
        let bytes = read.bytes().await?.to_vec();
        let compiled = compiled::decode_artifact(&bytes, Some(&base.registry_fingerprint))?;
        if compiled.id != board_id {
            return Err(anyhow!(
                "widget snapshot board identity does not match the artifact"
            ));
        }
        let cached = Arc::new(BaseArtifact {
            selectors: compiled::static_widget_selectors(&compiled),
            bytes,
        });
        self.bases.insert(key, cached.clone());
        Ok(cached)
    }

    async fn inventory(
        &self,
        store: &Arc<dyn ObjectStore>,
        app_id: &str,
    ) -> Result<(String, Arc<Vec<String>>)> {
        let path = Path::from("apps").join(app_id).join("manifest.app");
        let head = store.head(&path).await?;
        let revision = etag(&head)?.to_string();
        let key = cache_key(&path, &revision);
        if let Some(ids) = self.manifests.get(&key) {
            return Ok((revision, ids));
        }
        let (app, read) =
            from_compressed_with_meta::<flow_like_types::proto::App>(store.clone(), path).await?;
        require_revision(&head, &read)?;
        if app.id != app_id {
            return Err(anyhow!("widget inventory belongs to another app"));
        }
        let ids = Arc::new(app.widget_ids);
        self.manifests.insert(key, ids.clone());
        Ok((revision, ids))
    }

    async fn widget(
        &self,
        store: &Arc<dyn ObjectStore>,
        app_id: &str,
        id: &str,
        head: &ObjectMeta,
    ) -> Result<Arc<CachedWidget>> {
        let path = widget_path(app_id, id)?;
        let key = cache_key(&path, etag(head)?);
        if let Some(widget) = self.widgets.get(&key) {
            return Ok(widget);
        }
        let (widget, read) = from_compressed_json_with_meta::<Widget>(store.clone(), path).await?;
        require_revision(head, &read)?;
        if widget.id != id {
            return Err(anyhow!(
                "widget storage identifier does not match its definition"
            ));
        }
        let bytes = serde_json::to_vec(&widget)?.len();
        if bytes > compiled::widget_snapshot::MAX_WIDGET_SNAPSHOT_BYTES {
            return Err(anyhow!("widget exceeds the snapshot byte limit"));
        }
        let cached = Arc::new(CachedWidget {
            widget,
            bytes: bytes as u32,
        });
        self.widgets.insert(key, cached.clone());
        Ok(cached)
    }

    async fn enrich_inner(
        &self,
        store: &Arc<dyn ObjectStore>,
        app_id: &str,
        board_id: &str,
        board_identity: &str,
        base: &EnsuredArtifact,
    ) -> Result<EnsuredArtifact> {
        let artifact = self.base(store, base, board_id).await?;
        if artifact.selectors.is_empty() {
            return Ok(base.clone());
        }
        let (manifest_etag, ids) = self.inventory(store, app_id).await?;
        let ids_only = artifact
            .selectors
            .iter()
            .all(|selector| ids.contains(selector));
        let needed: Vec<String> = needed_inventory(&artifact.selectors, &ids)?
            .into_iter()
            .cloned()
            .collect();
        let dependencies: Vec<(String, Option<ObjectMeta>)> = futures::stream::iter(needed)
            .map(|id: String| async move {
                let path = widget_path(app_id, &id)?;
                let meta = match store.head(&path).await {
                    Ok(meta) => {
                        etag(&meta)?;
                        Some(meta)
                    }
                    Err(StoreError::NotFound { .. }) => None,
                    Err(error) => return Err(error.into()),
                };
                flow_like_types::Ok((id, meta))
            })
            .buffered(READ_CONCURRENCY)
            .try_collect()
            .await?;
        let revision = revision(
            app_id,
            board_id,
            board_identity,
            base,
            &manifest_etag,
            &artifact.selectors,
            &dependencies,
        )?;
        let path = compiled::draft_artifact_dir(app_id, board_id)
            .join(format!("widgets-v1-{revision}.flcb"));
        let key = path.to_string();
        let mut enriched = base.clone();
        enriched.path = path.clone();
        if self
            .bundles
            .get(&key)
            .is_some_and(|until| SystemTime::now() < until)
        {
            return Ok(enriched);
        }
        if let Some(written) =
            stored_artifact_written(store.as_ref(), &path, &base.registry_fingerprint).await
        {
            let mut until = draft_object_trusted_until(written);
            if SystemTime::now() >= until {
                until = refresh_draft_artifact(store.as_ref(), &path)
                    .await
                    .ok_or_else(|| anyhow!("widget artifact could not be refreshed"))?;
            }
            self.bundles.insert(key, until);
            return Ok(enriched);
        }
        let (widgets, _) = futures::stream::iter(dependencies.clone())
            .map(|(id, head): (String, Option<ObjectMeta>)| async move {
                match head {
                    Some(head) => self.widget(store, app_id, &id, &head).await.map(Some),
                    None => Ok(None),
                }
            })
            .buffered(READ_CONCURRENCY)
            .try_fold(
                (Vec::new(), 0_usize),
                |(mut widgets, bytes), widget| async move {
                    let Some(widget) = widget else {
                        return flow_like_types::Ok((widgets, bytes));
                    };
                    let bytes = bytes + widget.bytes as usize;
                    if bytes > MAX_INVENTORY_BYTES {
                        return Err(anyhow!("widget inventory exceeds the snapshot cache limit"));
                    }
                    widgets.push(widget);
                    Ok((widgets, bytes))
                },
            )
            .await?;
        let selected: Vec<_> = artifact
            .selectors
            .iter()
            .filter_map(|selector| {
                // A missing ID can fall back to a name elsewhere in the full
                // inventory. Leave that lookup live when only IDs were loaded.
                let selected = widgets
                    .iter()
                    .find(|entry| &entry.widget.id == selector)
                    .or_else(|| {
                        (!ids_only)
                            .then(|| widgets.iter().find(|entry| &entry.widget.name == selector))
                            .flatten()
                    })?;
                Some((selector, selected))
            })
            .collect();
        if selected
            .iter()
            .map(|(_, widget)| widget.bytes as usize)
            .sum::<usize>()
            > compiled::widget_snapshot::MAX_WIDGET_SNAPSHOT_BYTES
        {
            return Err(anyhow!("selected widgets exceed the snapshot byte limit"));
        }
        let entries: Vec<_> = selected
            .into_iter()
            .filter_map(|(selector, selected)| {
                let widget = &selected.widget;
                let source_etag = dependencies
                    .iter()
                    .find(|(id, _)| id == &widget.id)?
                    .1
                    .as_ref()?
                    .e_tag
                    .clone()?;
                Some(ArtifactWidgetEntry {
                    selector: selector.clone(),
                    widget_id: widget.id.clone(),
                    source_etag,
                    widget: widget.clone(),
                })
            })
            .collect();
        if entries.is_empty() {
            return Ok(base.clone());
        }
        let snapshot = ArtifactWidgetSnapshot {
            format_version: compiled::widget_snapshot::WIDGET_SNAPSHOT_FORMAT_VERSION,
            app_id: app_id.to_string(),
            board_id: board_id.to_string(),
            board_identity: board_identity.to_string(),
            revision,
            widgets: entries,
        };
        let bytes = compiled::append_widget_snapshot(&artifact.bytes, &snapshot)?;
        let written = SystemTime::now();
        compiled::persist_artifact(store, &path, None, bytes).await?;
        self.bundles
            .insert(key, draft_object_trusted_until(written));
        Ok(enriched)
    }

    pub(crate) async fn enrich(
        &self,
        store: &Arc<dyn ObjectStore>,
        app_id: &str,
        board_id: &str,
        version: Option<(u32, u32, u32)>,
        base: EnsuredArtifact,
    ) -> EnsuredArtifact {
        let identity = match (version, base.source_etag.as_deref()) {
            (Some((major, minor, patch)), _) => format!("{major}_{minor}_{patch}"),
            (None, Some(etag)) => format!("latest@{etag}"),
            _ => return base,
        };
        match flow_like_types::tokio::time::timeout(
            SNAPSHOT_TIMEOUT,
            self.enrich_inner(store, app_id, board_id, &identity, &base),
        )
        .await
        {
            Ok(Ok(enriched)) => enriched,
            Ok(Err(error)) => {
                tracing::debug!(app_id, board_id, %error, "Using live widget lookup for this artifact");
                base
            }
            Err(_) => {
                tracing::debug!(
                    app_id,
                    board_id,
                    "Widget artifact preparation timed out; using live lookup"
                );
                base
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{board::Board, node::Node, variable::VariableType};
    use flow_like::utils::compression::{compress_to_file, compress_to_file_json};
    use flow_like_storage::object_store::{self, *};
    use futures::stream::BoxStream;
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };

    #[derive(Debug, Default)]
    struct TestStore {
        inner: memory::InMemory,
        reads: AtomicUsize,
        head_delay_ms: AtomicU64,
        after_head: Mutex<Option<(Path, PutPayload)>>,
    }

    impl std::fmt::Display for TestStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "widget-artifact-test-store")
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for TestStore {
        async fn put_opts(
            &self,
            path: &Path,
            payload: PutPayload,
            opts: PutOptions,
        ) -> object_store::Result<PutResult> {
            self.inner.put_opts(path, payload, opts).await
        }
        async fn put_multipart_opts(
            &self,
            path: &Path,
            opts: PutMultipartOptions,
        ) -> object_store::Result<Box<dyn MultipartUpload>> {
            self.inner.put_multipart_opts(path, opts).await
        }
        async fn get_opts(&self, path: &Path, opts: GetOptions) -> object_store::Result<GetResult> {
            let head = opts.head;
            let head_delay = self.head_delay_ms.load(Ordering::SeqCst);
            if head && head_delay != 0 {
                flow_like_types::tokio::time::sleep(Duration::from_millis(head_delay)).await;
            }
            if !head {
                self.reads.fetch_add(1, Ordering::SeqCst);
            }
            let result = self.inner.get_opts(path, opts).await?;
            let replacement = {
                let mut pending = self.after_head.lock().unwrap();
                if head && pending.as_ref().is_some_and(|(target, _)| target == path) {
                    pending.take()
                } else {
                    None
                }
            };
            if let Some((path, bytes)) = replacement {
                self.inner.put(&path, bytes).await?;
            }
            Ok(result)
        }
        fn delete_stream(
            &self,
            paths: BoxStream<'static, object_store::Result<Path>>,
        ) -> BoxStream<'static, object_store::Result<Path>> {
            self.inner.delete_stream(paths)
        }
        fn list(
            &self,
            prefix: Option<&Path>,
        ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            self.inner.list(prefix)
        }
        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }
        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            opts: CopyOptions,
        ) -> object_store::Result<()> {
            self.inner.copy_opts(from, to, opts).await
        }
    }

    async fn base(store: &Arc<dyn ObjectStore>, selectors: &[&str]) -> EnsuredArtifact {
        let mut board = Board::new_detached(Some("board-1".into()), Path::default());
        for selector in selectors {
            let mut node = Node::new("a2ui_instantiate_widget", "Widget", "", "test");
            node.add_input_pin("widget_selector", "Widget", "", VariableType::String)
                .set_default_value(Some(serde_json::json!(selector)));
            board.nodes.insert(node.id.clone(), node);
        }
        let compiled = compiled::compile_board(&board).unwrap();
        let path = Path::from("apps/app-1/compiled/board-1/1_0_0.flcb");
        let bytes = compiled::encode_artifact(&compiled, &[7; 32]).unwrap();
        store.put(&path, bytes.into()).await.unwrap();
        EnsuredArtifact {
            path,
            source_etag: None,
            registry_fingerprint: [7; 32],
        }
    }

    async fn manifest(store: &Arc<dyn ObjectStore>, ids: &[&str]) {
        let app = flow_like_types::proto::App {
            id: "app-1".into(),
            widget_ids: ids.iter().map(|id| id.to_string()).collect(),
            ..Default::default()
        };
        compress_to_file(store.clone(), Path::from("apps/app-1/manifest.app"), &app)
            .await
            .unwrap();
    }

    async fn widget(store: &Arc<dyn ObjectStore>, id: &str, name: &str) {
        compress_to_file_json(
            store.clone(),
            widget_path("app-1", id).unwrap(),
            &Widget::new(id, name, "root"),
        )
        .await
        .unwrap();
    }

    async fn bundled(
        cache: &WidgetArtifactCache,
        store: &Arc<dyn ObjectStore>,
        base: &EnsuredArtifact,
    ) -> EnsuredArtifact {
        fn require_send<T: Send>(future: T) -> T {
            future
        }
        require_send(cache.enrich(store, "app-1", "board-1", Some((1, 0, 0)), base.clone())).await
    }

    async fn snapshot(
        store: &dyn ObjectStore,
        artifact: &EnsuredArtifact,
    ) -> ArtifactWidgetSnapshot {
        compiled::decode_widget_snapshot(
            &store
                .get(&artifact.path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
        )
        .unwrap()
        .expect("widget snapshot")
    }

    #[flow_like_types::tokio::test]
    async fn warm_snapshots_use_heads_and_renames_change_the_artifact_identity() {
        let counted = Arc::new(TestStore::default());
        let store: Arc<dyn ObjectStore> = counted.clone();
        let base = base(&store, &["History"]).await;
        manifest(&store, &["first", "second"]).await;
        widget(&store, "first", "History").await;
        widget(&store, "second", "Other").await;
        let cache = WidgetArtifactCache::default();
        let first = bundled(&cache, &store, &base).await;
        assert_ne!(first.path, base.path);
        let details = snapshot(&counted.inner, &first).await;
        assert_eq!(details.widgets.len(), 1);
        assert_eq!(details.widgets[0].widget_id, "first");
        let reads = counted.reads.load(Ordering::SeqCst);
        assert_eq!(bundled(&cache, &store, &base).await.path, first.path);
        assert_eq!(
            counted.reads.load(Ordering::SeqCst),
            reads,
            "warm dispatch must not download any bodies"
        );

        widget(&store, "first", "Renamed").await;
        widget(&store, "second", "History").await;
        let second = bundled(&cache, &store, &base).await;
        assert_ne!(second.path, first.path);
        assert_eq!(
            snapshot(&counted.inner, &second).await.widgets[0].widget_id,
            "second"
        );
        assert_eq!(
            bundled(&WidgetArtifactCache::default(), &store, &base)
                .await
                .path,
            second.path,
            "independent producers derive the same revision identity"
        );
    }

    #[flow_like_types::tokio::test]
    async fn name_resolution_keeps_id_priority_manifest_order_and_membership() {
        let store: Arc<dyn ObjectStore> = Arc::new(memory::InMemory::new());
        let base = base(&store, &["History"]).await;
        manifest(&store, &["first", "second", "History"]).await;
        widget(&store, "first", "History").await;
        widget(&store, "second", "History").await;
        widget(&store, "History", "Different name").await;
        let cache = WidgetArtifactCache::default();
        let exact = bundled(&cache, &store, &base).await;
        assert_eq!(
            snapshot(store.as_ref(), &exact).await.widgets[0].widget_id,
            "History"
        );

        // The removed definition still exists in storage but is no longer in the app.
        manifest(&store, &["first", "second"]).await;
        let first = bundled(&cache, &store, &base).await;
        assert_eq!(
            snapshot(store.as_ref(), &first).await.widgets[0].widget_id,
            "first"
        );
        manifest(&store, &["second", "first"]).await;
        let reordered = bundled(&cache, &store, &base).await;
        assert_ne!(first.path, reordered.path);
        assert_eq!(
            snapshot(store.as_ref(), &reordered).await.widgets[0].widget_id,
            "second"
        );
        store
            .delete(&widget_path("app-1", "second").unwrap())
            .await
            .unwrap();
        let deleted = bundled(&cache, &store, &base).await;
        assert_eq!(
            snapshot(store.as_ref(), &deleted).await.widgets[0].widget_id,
            "first"
        );
    }

    #[flow_like_types::tokio::test]
    async fn a_widget_changed_between_head_and_get_never_enters_the_old_revision_cache() {
        let counted = Arc::new(TestStore::default());
        let store: Arc<dyn ObjectStore> = counted.clone();
        let base = base(&store, &["first"]).await;
        manifest(&store, &["first"]).await;
        widget(&store, "first", "Before").await;
        let replacement: Arc<dyn ObjectStore> = Arc::new(memory::InMemory::new());
        widget(&replacement, "first", "After").await;
        let path = widget_path("app-1", "first").unwrap();
        let payload = replacement.get(&path).await.unwrap().bytes().await.unwrap();
        *counted.after_head.lock().unwrap() = Some((path, payload.into()));
        let cache = WidgetArtifactCache::default();
        assert_eq!(bundled(&cache, &store, &base).await.path, base.path);
        assert!(cache.widgets.iter().next().is_none());
        let retried = bundled(&cache, &store, &base).await;
        assert_eq!(
            snapshot(&counted.inner, &retried).await.widgets[0]
                .widget
                .name,
            "After"
        );
    }

    #[flow_like_types::tokio::test]
    async fn missing_ids_do_not_resolve_against_a_partial_name_inventory() {
        let store: Arc<dyn ObjectStore> = Arc::new(memory::InMemory::new());
        let base = base(&store, &["missing", "second"]).await;
        manifest(&store, &["first", "missing", "second"]).await;
        widget(&store, "first", "missing").await;
        widget(&store, "second", "missing").await;
        let artifact = bundled(&WidgetArtifactCache::default(), &store, &base).await;
        let entries = snapshot(store.as_ref(), &artifact).await.widgets;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].selector, "second");
        assert_eq!(entries[0].widget_id, "second");
    }

    #[flow_like_types::tokio::test(start_paused = true)]
    async fn slow_optional_reads_fall_back_within_the_budget_and_keep_completed_cache_work() {
        let counted = Arc::new(TestStore::default());
        let store: Arc<dyn ObjectStore> = counted.clone();
        let base = base(&store, &["History"]).await;
        manifest(&store, &["first"]).await;
        widget(&store, "first", "History").await;
        counted.head_delay_ms.store(1_000, Ordering::SeqCst);
        let cache = WidgetArtifactCache::default();
        let start = flow_like_types::tokio::time::Instant::now();
        assert_eq!(bundled(&cache, &store, &base).await.path, base.path);
        assert_eq!(start.elapsed(), Duration::from_millis(250));
        assert!(cache.bases.iter().next().is_some());
        counted.head_delay_ms.store(0, Ordering::SeqCst);
        assert_ne!(bundled(&cache, &store, &base).await.path, base.path);
    }

    #[flow_like_types::tokio::test]
    async fn oversized_snapshots_and_name_inventories_keep_live_lookup() {
        let store: Arc<dyn ObjectStore> = Arc::new(memory::InMemory::new());
        let base = base(&store, &["History"]).await;
        let ids: Vec<_> = (0..=MAX_WIDGETS)
            .map(|index| format!("widget-{index}"))
            .collect();
        manifest(&store, &ids.iter().map(String::as_str).collect::<Vec<_>>()).await;
        let cache = WidgetArtifactCache::default();
        assert_eq!(bundled(&cache, &store, &base).await.path, base.path);
        manifest(&store, &["first"]).await;
        let mut oversized = Widget::new("first", "History", "root");
        oversized.description =
            Some("x".repeat(compiled::widget_snapshot::MAX_WIDGET_SNAPSHOT_BYTES));
        compress_to_file_json(
            store.clone(),
            widget_path("app-1", "first").unwrap(),
            &oversized,
        )
        .await
        .unwrap();
        assert_eq!(bundled(&cache, &store, &base).await.path, base.path);
    }

    #[flow_like_types::tokio::test]
    async fn individually_small_definitions_still_obey_the_combined_snapshot_limit() {
        let store: Arc<dyn ObjectStore> = Arc::new(memory::InMemory::new());
        let base = base(&store, &["first", "second"]).await;
        manifest(&store, &["first", "second"]).await;
        for id in ["first", "second"] {
            let mut definition = Widget::new(id, id, "root");
            definition.description =
                Some("x".repeat(compiled::widget_snapshot::MAX_WIDGET_SNAPSHOT_BYTES / 2));
            compress_to_file_json(
                store.clone(),
                widget_path("app-1", id).unwrap(),
                &definition,
            )
            .await
            .unwrap();
        }
        let error = WidgetArtifactCache::default()
            .enrich_inner(&store, "app-1", "board-1", "1_0_0", &base)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("selected widgets exceed"));
    }
}
