use super::{ObjectPath, ObjectStore, ProjectStore};
use flow_like_runtime::state::FlowLikeConfig;
use flow_like_storage::{
    files::store::local_store::LocalObjectStore,
    lance::{Error, Result},
    lance_io::object_store::{
        ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
    },
    object_store::{
        self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMode,
        PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, UploadPart,
    },
};
use futures_util::stream::BoxStream;
use std::{collections::HashMap, path::Path, sync::Arc};
use url::Url;

const SCHEME: &str = "standalone-local";

/// Lance selects its unsafe commit handler for this custom scheme, which writes
/// each version manifest with a plain put. Creating manifests conditionally
/// makes a concurrent commit of the same version fail instead of silently
/// replacing the winner and orphaning its rows.
#[derive(Debug)]
struct ConditionalManifests(Arc<dyn ObjectStore>);

impl std::fmt::Display for ConditionalManifests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConditionalManifests({})", self.0)
    }
}

/// Lance switches to a multipart upload once a manifest outgrows one part.
/// Buffering it keeps the whole manifest a single conditional create.
#[derive(Debug)]
struct ManifestUpload {
    store: Arc<dyn ObjectStore>,
    path: ObjectPath,
    opts: PutOptions,
    parts: Vec<bytes::Bytes>,
}

#[async_trait::async_trait]
impl MultipartUpload for ManifestUpload {
    fn put_part(&mut self, data: PutPayload) -> UploadPart {
        self.parts.extend(data);
        Box::pin(std::future::ready(Ok(())))
    }
    async fn complete(&mut self) -> object_store::Result<PutResult> {
        let payload = std::mem::take(&mut self.parts)
            .into_iter()
            .collect::<PutPayload>();
        self.store
            .put_opts(&self.path, payload, std::mem::take(&mut self.opts))
            .await
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        self.parts.clear();
        Ok(())
    }
}

fn is_manifest(path: &ObjectPath) -> bool {
    let mut parts = path.parts().collect::<Vec<_>>();
    let Some(file) = parts.pop() else {
        return false;
    };
    file.as_ref().ends_with(".manifest")
        && parts
            .last()
            .is_some_and(|directory| directory.as_ref() == "_versions")
}

#[async_trait::async_trait]
impl ObjectStore for ConditionalManifests {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        mut opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        if is_manifest(path) && matches!(opts.mode, PutMode::Overwrite) {
            opts.mode = PutMode::Create;
        }
        self.0.put_opts(path, payload, opts).await
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        opts: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        if !is_manifest(path) {
            return self.0.put_multipart_opts(path, opts).await;
        }
        Ok(Box::new(ManifestUpload {
            store: self.0.clone(),
            path: path.clone(),
            opts: PutOptions {
                mode: PutMode::Create,
                tags: opts.tags,
                attributes: opts.attributes,
                extensions: opts.extensions,
            },
            parts: Vec::new(),
        }))
    }
    async fn get_opts(
        &self,
        path: &ObjectPath,
        opts: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.0.get_opts(path, opts).await
    }
    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        self.0.delete_stream(locations)
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.0.list(prefix)
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        self.0.list_with_delimiter(prefix).await
    }
    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.0.copy_opts(from, to, options).await
    }
    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.0.rename_opts(from, to, options).await
    }
}

#[derive(Clone, Debug)]
struct LocalProvider {
    store: Arc<ProjectStore>,
    cache_key: String,
}

impl LocalProvider {
    fn path(&self, url: &Url) -> Result<ObjectPath> {
        if url.scheme() != SCHEME
            || url.host_str() != Some("runtime")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path().contains(['%', '\\'])
        {
            return Err(Error::invalid_input("Invalid private runtime database URI"));
        }
        let path = ObjectPath::parse(url.path())
            .map_err(|_| Error::invalid_input("Invalid private runtime database path"))?;
        self.store.list_path(Some(&path)).map_err(|_| {
            Error::invalid_input("Database is outside the private runtime directories")
        })?;
        Ok(path)
    }
}

#[async_trait::async_trait]
impl ObjectStoreProvider for LocalProvider {
    async fn new_store(&self, url: Url, params: &ObjectStoreParams) -> Result<LanceStore> {
        self.path(&url)?;
        let mut params = params.clone();
        #[allow(deprecated)]
        {
            params.object_store = Some((self.store.clone(), url.clone()));
        }
        params.object_store_wrapper = None;
        params.list_is_lexically_ordered = Some(false);
        let registry = Arc::new(ObjectStoreRegistry::empty());
        registry.insert(SCHEME, Arc::new(self.clone()));
        let (store, _) = LanceStore::from_uri_and_params(registry, url.as_str(), &params).await?;
        let mut store = store.as_ref().clone();
        store.inner = self.store.clone();
        // A custom scheme keeps Lance's direct local-file fast path disabled.
        // Every read, write and copy passes through the directory-scoped store.
        if store.scheme() != SCHEME || store.is_local() {
            return Err(Error::invalid_input(
                "Private runtime store lost its custom scheme",
            ));
        }
        Ok(store)
    }
    fn extract_path(&self, url: &Url) -> Result<ObjectPath> {
        self.path(url)
    }
    fn calculate_object_store_prefix(
        &self,
        url: &Url,
        _options: Option<&HashMap<String, String>>,
    ) -> Result<String> {
        self.path(url)?;
        Ok(self.cache_key.clone())
    }
}

pub(super) fn configure(
    local_data: &Path,
    runtime: &mut FlowLikeConfig,
    registry: &Arc<ObjectStoreRegistry>,
) -> anyhow::Result<()> {
    let store: Arc<dyn ObjectStore> = Arc::new(ConditionalManifests(Arc::new(
        LocalObjectStore::new(local_data.to_path_buf())?,
    )));
    let provider = LocalProvider {
        store: Arc::new(ProjectStore {
            routes: vec![("logs/".into(), store)],
        }),
        cache_key: format!(
            "standalone-local:{}",
            blake3::hash(local_data.to_string_lossy().as_bytes()).to_hex()
        ),
    };
    registry.insert(SCHEME, Arc::new(provider));
    runtime.register_build_logs_database(Arc::new(|path| {
        flow_like_storage::databases::vector::lancedb::connect_lance(&format!(
            "{SCHEME}://runtime/logs/{path}"
        ))
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::ObjectStoreExt;

    #[tokio::test]
    async fn local_runtime_databases_use_scoped_store_without_local_file_bypass() {
        let directory = tempfile::tempdir().unwrap();
        let config = super::super::tests::config(directory.path());
        let registry = Arc::new(ObjectStoreRegistry::empty());
        let mut runtime = FlowLikeConfig::new();
        configure(&config.project_path, &mut runtime, &registry).unwrap();
        let store = registry
            .get_store(
                Url::parse("standalone-local://runtime/logs/db").unwrap(),
                &ObjectStoreParams::default(),
            )
            .await
            .unwrap();
        assert!(!store.is_local());
        store
            .inner
            .put(
                &ObjectPath::from("logs/db/value"),
                bytes::Bytes::from_static(b"private").into(),
            )
            .await
            .unwrap();
        assert!(directory.path().join("logs/db/value").is_file());
        assert!(
            store
                .inner
                .get(&ObjectPath::from(".secrets/placement/token.secret"))
                .await
                .is_err()
        );
        assert!(
            registry
                .get_store(
                    Url::parse("file:///etc/passwd").unwrap(),
                    &ObjectStoreParams::default()
                )
                .await
                .is_err()
        );
        assert!(
            registry
                .get_store(
                    Url::parse("standalone-local://runtime/apps/project/db").unwrap(),
                    &ObjectStoreParams::default()
                )
                .await
                .is_err()
        );
        assert!(
            registry
                .get_store(
                    Url::parse("standalone-local://runtime/user%2fescape/db").unwrap(),
                    &ObjectStoreParams::default()
                )
                .await
                .is_err()
        );
        let session = Arc::new(flow_like_storage::lance::session::Session::new(
            flow_like_storage::lance::dataset::DEFAULT_INDEX_CACHE_SIZE,
            flow_like_storage::lance::dataset::DEFAULT_METADATA_CACHE_SIZE,
            registry,
        ));
        let connection =
            flow_like_storage::lancedb::connect("standalone-local://runtime/logs/history")
                .session(session)
                .execute()
                .await
                .unwrap();
        assert!(connection.table_names().execute().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_manifest_commits_cannot_replace_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(ObjectStoreRegistry::empty());
        configure(directory.path(), &mut FlowLikeConfig::new(), &registry).unwrap();
        let store = registry
            .get_store(
                Url::parse("standalone-local://runtime/logs/history").unwrap(),
                &ObjectStoreParams::default(),
            )
            .await
            .unwrap();
        let manifest = ObjectPath::from("logs/history/run.lance/_versions/2.manifest");
        store
            .inner
            .put(&manifest, bytes::Bytes::from_static(b"first").into())
            .await
            .unwrap();
        assert!(matches!(
            store
                .inner
                .put(&manifest, bytes::Bytes::from_static(b"second").into())
                .await
                .unwrap_err(),
            object_store::Error::AlreadyExists { .. }
        ));
        assert_eq!(
            store
                .inner
                .get(&manifest)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            "first"
        );
        let hint = ObjectPath::from("logs/history/run.lance/_versions/version_hint.json");
        for value in [&b"1"[..], &b"2"[..]] {
            store
                .inner
                .put(&hint, bytes::Bytes::copy_from_slice(value).into())
                .await
                .unwrap();
        }
        assert!(!is_manifest(&ObjectPath::from(
            "logs/history/run.lance/data/2.manifest"
        )));
    }

    #[tokio::test]
    async fn large_manifests_uploaded_in_parts_are_created_conditionally() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(ObjectStoreRegistry::empty());
        configure(directory.path(), &mut FlowLikeConfig::new(), &registry).unwrap();
        let store = registry
            .get_store(
                Url::parse("standalone-local://runtime/logs/history").unwrap(),
                &ObjectStoreParams::default(),
            )
            .await
            .unwrap();
        let manifest = ObjectPath::from("logs/history/run.lance/_versions/3.manifest");
        let upload = |parts: [&'static [u8]; 2]| {
            let store = store.inner.clone();
            let manifest = manifest.clone();
            async move {
                let mut upload = store.put_multipart(&manifest).await?;
                for part in parts {
                    upload
                        .put_part(bytes::Bytes::from_static(part).into())
                        .await?;
                }
                upload.complete().await
            }
        };
        upload([b"first ", b"winner"]).await.unwrap();
        assert!(matches!(
            upload([b"second ", b"loser"]).await.unwrap_err(),
            object_store::Error::AlreadyExists { .. }
        ));
        assert_eq!(
            store
                .inner
                .get(&manifest)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            "first winner"
        );
    }
}
