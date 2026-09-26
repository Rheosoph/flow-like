use super::{ObjectPath, ObjectStore, ProjectStore};
use flow_like_runtime::state::FlowLikeConfig;
use flow_like_storage::{
    files::store::local_store::LocalObjectStore,
    lance::{Error, Result},
    lance_io::object_store::{
        ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
    },
};
use std::{collections::HashMap, path::Path, sync::Arc};
use url::Url;

const SCHEME: &str = "standalone-local";

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
    let store: Arc<dyn ObjectStore> = Arc::new(LocalObjectStore::new(local_data.to_path_buf())?);
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
}
