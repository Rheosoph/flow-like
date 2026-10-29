use flow_like_runtime::state::FlowLikeStores;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{
    self, memory::InMemory, path::Path, CopyOptions, GetOptions, GetResult, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions,
    PutPayload, PutResult,
};
use flow_like_wasm::host_functions::linker::{register_host_functions, StoreData};
use flow_like_wasm::host_functions::storage::{StorageFlowPath, StorageStore};
use flow_like_wasm::host_functions::StorageContext;
use flow_like_wasm::{WasmAbi, WasmCapabilities};
use futures::{stream::BoxStream, StreamExt};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{Engine, Linker, Module, Store};

const UPLOAD: &str = "dirs__upload_apps/app/upload";
const IMPORTED: &str = "imported-store";

fn flow_path(store_ref: &str, path: &str) -> StorageFlowPath {
    StorageFlowPath {
        path: path.to_owned(),
        store_ref: store_ref.to_owned(),
        cache_store_ref: None,
    }
}

async fn fixture() -> (StorageContext, FlowLikeStore) {
    // Desktop stores contain keys from several apps in the same backing store.
    let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
    for path in [
        "apps/app/upload/visible.txt",
        "apps/app/upload/nested/visible.txt",
        "apps/app/upload-sibling/secret.txt",
        "apps/app/storage/node-a/visible.txt",
        "apps/app/storage/node-b/secret.txt",
        "apps/victim/upload/secret.txt",
        "apps/victim/storage/secret.txt",
        "users/alice/apps/app/visible.txt",
        "users/alice/apps/victim/secret.txt",
        "users/bob/apps/app/secret.txt",
        "tmp/global/apps/app/visible.txt",
        "tmp/global/apps/victim/secret.txt",
        "tmp/user/alice/apps/app/visible.txt",
        "tmp/user/bob/apps/app/secret.txt",
        "external/shared/visible.txt",
        "external/second/visible.txt",
        "external/shared-sibling/secret.txt",
        "external/private/secret.txt",
    ] {
        store
            .as_generic()
            .put(&Path::from(path), bytes::Bytes::from(path).into())
            .await
            .unwrap();
    }
    let context = StorageContext {
        stores: FlowLikeStores {
            app_storage_store: Some(store.clone()),
            user_store: Some(store.clone()),
            temporary_store: Some(store.clone()),
            ..Default::default()
        },
        store_cache: RwLock::new(HashMap::from([(
            IMPORTED.to_owned(),
            StorageStore {
                store: store.clone(),
                roots: vec![Path::from("external/shared"), Path::from("external/second")],
            },
        )])),
        credentials_store: None,
        app_id: "app".into(),
        board_dir: Path::from("apps/app"),
        board_id: "board".into(),
        node_id: "node-a".into(),
        sub: "alice".into(),
    };
    (context, store)
}

#[derive(Clone, Copy, Debug)]
enum Runtime {
    Core,
    #[cfg(feature = "component-model")]
    Component,
}

const RUNTIMES: &[Runtime] = &[
    Runtime::Core,
    #[cfg(feature = "component-model")]
    Runtime::Component,
];

enum Guest {
    Core {
        store: Store<StoreData>,
        instance: wasmtime::Instance,
    },
    #[cfg(feature = "component-model")]
    Component {
        store: Store<flow_like_wasm::component::linker::ComponentStoreData>,
        instance: wasmtime::component::Instance,
    },
}

impl Guest {
    async fn new(
        runtime: Runtime,
        context: StorageContext,
        capabilities: WasmCapabilities,
    ) -> Self {
        let engine = Engine::default();
        match runtime {
            Runtime::Core => {
                let wasm = wat::parse_str(
                    r#"(module
                        (import "flowlike_storage" "read_request"
                            (func $read (param i32 i32) (result i64)))
                        (import "flowlike_storage" "list_request"
                            (func $list (param i32 i32) (result i64)))
                        (import "flowlike_storage" "write_request"
                            (func $write (param i32 i32 i32 i32) (result i32)))
                        (memory (export "memory") 1)
                        (func (export "read") (param i32 i32) (result i64)
                            local.get 0 local.get 1 call $read)
                        (func (export "list") (param i32 i32) (result i64)
                            local.get 0 local.get 1 call $list)
                        (func (export "write") (param i32 i32 i32 i32) (result i32)
                            local.get 0 local.get 1 local.get 2 local.get 3 call $write))"#,
                )
                .unwrap();
                let module = Module::new(&engine, wasm).unwrap();
                let mut linker = Linker::new(&engine);
                register_host_functions(&mut linker).unwrap();
                let mut data = StoreData::new(capabilities);
                data.host_state.storage_context = Some(context);
                let mut store = Store::new(&engine, data);
                let instance = linker.instantiate_async(&mut store, &module).await.unwrap();
                store.data_mut().memory = instance.get_memory(&mut store, "memory");
                Self::Core { store, instance }
            }
            #[cfg(feature = "component-model")]
            Runtime::Component => {
                use flow_like_wasm::component::linker::{
                    register_component_host_functions, ComponentStoreData,
                };
                use flow_like_wasm::WasmSecurityConfig;

                let component = wasmtime::component::Component::new(
                    &engine,
                    wat::parse_str(COMPONENT_PROBE).unwrap(),
                )
                .unwrap();
                let security = WasmSecurityConfig {
                    capabilities,
                    ..Default::default()
                };
                let mut linker = wasmtime::component::Linker::new(&engine);
                register_component_host_functions(&mut linker, &security).unwrap();
                let mut data = ComponentStoreData::new(&security);
                data.host_state.storage_context = Some(context);
                let mut store = Store::new(&engine, data);
                let instance = linker
                    .instantiate_async(&mut store, &component)
                    .await
                    .unwrap();
                Self::Component { store, instance }
            }
        }
    }

    async fn read_or_list(&mut self, operation: &str, path: &StorageFlowPath) -> Option<Vec<u8>> {
        self.read_or_list_json(operation, serde_json::to_string(path).unwrap())
            .await
    }

    async fn read_or_list_json(&mut self, operation: &str, json: String) -> Option<Vec<u8>> {
        match self {
            Self::Core { store, instance } => {
                store
                    .data()
                    .memory
                    .unwrap()
                    .write(&mut *store, 0, json.as_bytes())
                    .unwrap();
                let packed = instance
                    .get_typed_func::<(i32, i32), i64>(&mut *store, operation)
                    .unwrap()
                    .call_async(&mut *store, (0, json.len() as i32))
                    .await
                    .unwrap();
                if packed == 0 {
                    return None;
                }
                let (offset, length) = WasmAbi::unpack_ptr_len(packed);
                Some(
                    store.data().host_state.result_buffer.read()
                        [offset as usize..(offset + length) as usize]
                        .to_vec(),
                )
            }
            #[cfg(feature = "component-model")]
            Self::Component { store, instance } => {
                if operation == "read" {
                    instance
                        .get_typed_func::<(String,), (Option<Vec<u8>>,)>(&mut *store, "read")
                        .unwrap()
                        .call_async(&mut *store, (json,))
                        .await
                        .unwrap()
                        .0
                } else {
                    instance
                        .get_typed_func::<(String,), (Option<String>,)>(&mut *store, "list")
                        .unwrap()
                        .call_async(&mut *store, (json,))
                        .await
                        .unwrap()
                        .0
                        .map(String::into_bytes)
                }
            }
        }
    }

    async fn write(&mut self, path: &StorageFlowPath, data: &[u8]) -> bool {
        let json = serde_json::to_string(path).unwrap();
        match self {
            Self::Core { store, instance } => {
                let memory = store.data().memory.unwrap();
                memory.write(&mut *store, 0, json.as_bytes()).unwrap();
                memory.write(&mut *store, 4096, data).unwrap();
                instance
                    .get_typed_func::<(i32, i32, i32, i32), i32>(&mut *store, "write")
                    .unwrap()
                    .call_async(&mut *store, (0, json.len() as i32, 4096, data.len() as i32))
                    .await
                    .unwrap()
                    == 0
            }
            #[cfg(feature = "component-model")]
            Self::Component { store, instance } => {
                instance
                    .get_typed_func::<(String, Vec<u8>), (bool,)>(&mut *store, "write")
                    .unwrap()
                    .call_async(&mut *store, (json, data.to_vec()))
                    .await
                    .unwrap()
                    .0
            }
        }
    }
}

#[tokio::test]
async fn storage_hosts_enforce_app_user_and_origin_boundaries() {
    for &runtime in RUNTIMES {
        let (context, _) = fixture().await;
        let mut guest = Guest::new(runtime, context, WasmCapabilities::STORAGE_READ).await;

        for (reference, path) in [
            (UPLOAD, "apps/app/upload/visible.txt"),
            (
                "wasm_dirs__upload_apps/app/upload",
                "apps/app/upload/visible.txt",
            ),
            (
                "cache_dirs__upload_apps/app/upload",
                "apps/app/upload/visible.txt",
            ),
            (
                "dirs__upload_apps/app/upload/nested",
                "apps/app/upload/nested/visible.txt",
            ),
            (
                "dirs__storage_apps/app/storage/node-a",
                "apps/app/storage/node-a/visible.txt",
            ),
            (
                "dirs__user_users/alice/apps/app",
                "users/alice/apps/app/visible.txt",
            ),
            (
                "dirs__cache_tmp/global/apps/app",
                "tmp/global/apps/app/visible.txt",
            ),
            (
                "dirs__cache_tmp/user/alice/apps/app",
                "tmp/user/alice/apps/app/visible.txt",
            ),
            (IMPORTED, "external/shared/visible.txt"),
            (IMPORTED, "external/second/visible.txt"),
        ] {
            assert_eq!(
                guest
                    .read_or_list("read", &flow_path(reference, path))
                    .await,
                Some(path.as_bytes().to_vec()),
                "{runtime:?}: {reference}: {path}"
            );
        }

        for (reference, path) in [
            (UPLOAD, "apps/victim/upload/secret.txt"),
            (UPLOAD, "apps/app/upload-sibling/secret.txt"),
            (UPLOAD, "apps/app/storage/node-b/secret.txt"),
            (
                "dirs__upload_apps/victim/upload",
                "apps/victim/upload/secret.txt",
            ),
            (
                "wasm_dirs__upload_apps/victim/upload",
                "apps/victim/upload/secret.txt",
            ),
            (
                "cache_dirs__upload_apps/app/upload",
                "apps/victim/upload/secret.txt",
            ),
            (
                "cache_dirs__upload_apps/victim/upload",
                "apps/victim/upload/secret.txt",
            ),
            (
                "dirs__upload_apps/app",
                "apps/app/storage/node-b/secret.txt",
            ),
            (
                "dirs__upload_apps/app/upload/nested",
                "apps/app/upload/visible.txt",
            ),
            (
                "dirs__storage_apps/app/storage/node-a",
                "apps/app/storage/node-b/secret.txt",
            ),
            (
                "dirs__storage_apps/victim/storage",
                "apps/victim/storage/secret.txt",
            ),
            (
                "dirs__user_users/alice/apps/app",
                "users/bob/apps/app/secret.txt",
            ),
            (
                "dirs__user_users/bob/apps/app",
                "users/bob/apps/app/secret.txt",
            ),
            (
                "dirs__user_users/alice/apps/victim",
                "users/alice/apps/victim/secret.txt",
            ),
            (
                "dirs__cache_tmp/global/apps/victim",
                "tmp/global/apps/victim/secret.txt",
            ),
            (
                "dirs__cache_tmp/user/bob/apps/app",
                "tmp/user/bob/apps/app/secret.txt",
            ),
            (IMPORTED, "external/shared-sibling/secret.txt"),
            (IMPORTED, "external/private/secret.txt"),
        ] {
            assert_eq!(
                guest
                    .read_or_list("read", &flow_path(reference, path))
                    .await,
                None,
                "{runtime:?}: {reference}: {path}"
            );
        }

        let listed = guest
            .read_or_list("list", &flow_path(UPLOAD, "apps/app/upload"))
            .await
            .unwrap();
        let entries: Vec<StorageFlowPath> = serde_json::from_slice(&listed).unwrap();
        let mut paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            [
                "apps/app/upload/nested/visible.txt",
                "apps/app/upload/visible.txt"
            ]
        );
        for (reference, path) in [
            (UPLOAD, "apps"),
            (UPLOAD, "apps/app/upload-sibling"),
            ("dirs__upload_apps/victim/upload", "apps/victim/upload"),
            (IMPORTED, "external"),
        ] {
            assert!(
                guest
                    .read_or_list("list", &flow_path(reference, path))
                    .await
                    .is_none(),
                "{runtime:?}: listing escaped {reference}: {path}"
            );
        }
    }
}

#[tokio::test]
async fn storage_writes_obey_the_same_boundaries() {
    for &runtime in RUNTIMES {
        let (context, store) = fixture().await;
        let mut guest = Guest::new(runtime, context, WasmCapabilities::STORAGE_WRITE).await;
        assert!(
            guest
                .write(&flow_path(UPLOAD, "apps/app/upload/new.txt"), b"allowed")
                .await
        );
        let written = store
            .as_generic()
            .get(&Path::from("apps/app/upload/new.txt"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(written.as_ref(), b"allowed");
        for (reference, path) in [
            (UPLOAD, "apps/victim/upload/secret.txt"),
            (UPLOAD, "apps/app/upload-sibling/secret.txt"),
            (
                "dirs__upload_apps/victim/upload",
                "apps/victim/upload/secret.txt",
            ),
            (
                "cache_dirs__upload_apps/victim/upload",
                "apps/victim/upload/secret.txt",
            ),
            (IMPORTED, "external/private/secret.txt"),
        ] {
            assert!(
                !guest
                    .write(&flow_path(reference, path), b"overwritten")
                    .await,
                "{runtime:?}: write escaped {reference}: {path}"
            );
            let bytes = store
                .as_generic()
                .get(&Path::from(path))
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            assert_eq!(bytes.as_ref(), path.as_bytes());
        }
    }
}

#[tokio::test]
async fn valid_storage_paths_still_require_capabilities() {
    for &runtime in RUNTIMES {
        let (context, _) = fixture().await;
        let mut guest = Guest::new(runtime, context, WasmCapabilities::NONE).await;
        let path = flow_path(UPLOAD, "apps/app/upload/visible.txt");
        assert!(guest.read_or_list("read", &path).await.is_none());
        assert!(guest
            .read_or_list("list", &flow_path(UPLOAD, "apps/app/upload"))
            .await
            .is_none());
        assert!(!guest.write(&path, b"denied").await);
    }
}

#[derive(Debug)]
struct PrunedListingStore {
    inner: InMemory,
    visited: Arc<RwLock<Vec<String>>>,
}

impl std::fmt::Display for PrunedListingStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PrunedListingStore")
    }
}

fn unexpected_listing(message: &str) -> object_store::Error {
    object_store::Error::Generic {
        store: "PrunedListingStore",
        source: message.to_owned().into(),
    }
}

#[async_trait::async_trait]
impl ObjectStore for PrunedListingStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(&self, _: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        futures::stream::once(async {
            Err(unexpected_listing(
                "Recursive listing opens excluded directories",
            ))
        })
        .boxed()
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        let prefix = prefix.cloned().unwrap_or_default();
        self.visited.write().push(prefix.to_string());
        if prefix
            .parts()
            .any(|part| matches!(part.as_ref(), "node_modules" | ".git"))
        {
            return Err(unexpected_listing("An excluded directory was opened"));
        }
        self.inner.list_with_delimiter(Some(&prefix)).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }
}

async fn pruned_listing_fixture() -> (StorageContext, Arc<RwLock<Vec<String>>>) {
    let visited = Arc::new(RwLock::new(Vec::new()));
    let backend = Arc::new(PrunedListingStore {
        inner: InMemory::new(),
        visited: visited.clone(),
    });
    for path in [
        "project/package.json",
        "project/src/pages/index.astro",
        "project/src/node_modules",
        "project/node_modules/package/index.js",
        "project/src/pages/node_modules/package/index.js",
        "project/.git/objects/pack/data",
        "project-sibling/private.txt",
    ] {
        backend
            .put(
                &Path::from(path),
                bytes::Bytes::from_static(b"contents").into(),
            )
            .await
            .unwrap();
    }
    let (context, _) = fixture().await;
    context.store_cache.write().insert(
        "project-store".into(),
        StorageStore {
            store: FlowLikeStore::Other(backend),
            roots: vec![Path::from("project")],
        },
    );
    (context, visited)
}

fn assert_pruned_listing(visited: &RwLock<Vec<String>>, mut paths: Vec<String>) {
    paths.sort();
    assert_eq!(
        paths,
        [
            "project/package.json",
            "project/src/node_modules",
            "project/src/pages/index.astro",
        ]
    );
    let mut directories = visited.read().clone();
    directories.sort();
    assert_eq!(directories, ["project", "project/src", "project/src/pages"]);
}

#[tokio::test]
async fn legacy_listing_prunes_excluded_subtrees_before_opening_them() {
    for &runtime in RUNTIMES {
        let (context, visited) = pruned_listing_fixture().await;
        let mut guest = Guest::new(runtime, context, WasmCapabilities::STORAGE_READ).await;
        let request = |path: &str| {
            serde_json::json!({
                "path": path,
                "store_ref": "project-store",
                "exclude_directories": ["node_modules", ".git"],
            })
            .to_string()
        };
        let bytes = guest
            .read_or_list_json("list", request("project"))
            .await
            .expect("Excluded directories must not be opened by either WASM ABI");
        let entries: Vec<StorageFlowPath> = serde_json::from_slice(&bytes).unwrap();
        assert_pruned_listing(
            &visited,
            entries.into_iter().map(|entry| entry.path).collect(),
        );
        visited.write().clear();
        assert!(guest
            .read_or_list_json("list", request("project-sibling"))
            .await
            .is_none());
        assert!(
            visited.read().is_empty(),
            "A denied prefix reached the backend"
        );

        let (context, visited) = pruned_listing_fixture().await;
        let mut denied = Guest::new(runtime, context, WasmCapabilities::NONE).await;
        assert!(denied
            .read_or_list_json("list", request("project"))
            .await
            .is_none());
        assert!(
            visited.read().is_empty(),
            "An unpermitted request reached the backend"
        );
    }
}

#[tokio::test]
async fn paginated_listing_prunes_excluded_subtrees_across_cursors() {
    use flow_like_wasm::host_functions::storage_ops::{
        list_files_page, ListPageRequest, ListSessions,
    };

    let (context, visited) = pruned_listing_fixture().await;
    let sessions = ListSessions::default();
    let mut request = ListPageRequest {
        path: flow_path("project-store", "project"),
        limit: 1,
        cursor: None,
        exclude_directories: vec!["node_modules".into(), ".git".into()],
        recursive: true,
    };
    let mut paths = Vec::new();
    loop {
        let page = list_files_page(&context, &sessions, &request)
            .await
            .unwrap();
        paths.extend(page.entries.into_iter().map(|entry| entry.flow_path.path));
        request.cursor = page.cursor;
        if request.cursor.is_none() {
            break;
        }
    }
    assert_pruned_listing(&visited, paths);
    visited.write().clear();
    request.path.path = "project-sibling".into();
    assert_eq!(
        list_files_page(&context, &sessions, &request)
            .await
            .unwrap_err()
            .code,
        "permission_denied"
    );
    assert!(
        visited.read().is_empty(),
        "A denied prefix reached the backend"
    );
}

// Adapt the component imports through linear memory so the test crosses the
// same canonical ABI as a guest package, including optional read/list results.
#[cfg(feature = "component-model")]
const COMPONENT_PROBE: &str = r#"
    (component
        (import "flow-like:node/storage@0.1.0" (instance $storage
            (export "read-file" (func (param "path" string) (result (option (list u8)))))
            (export "list-files" (func (param "path" string) (result (option string))))
            (export "write-file" (func (param "path" string) (param "data" (list u8)) (result bool)))))
        (alias export $storage "read-file" (func $host-read))
        (alias export $storage "list-files" (func $host-list))
        (alias export $storage "write-file" (func $host-write))
        (core module $allocation
            (memory (export "memory") 2)
            (global $heap (mut i32) (i32.const 1024))
            (func (export "realloc") (param i32 i32) (param $align i32) (param $size i32) (result i32)
                (local $pointer i32)
                global.get $heap local.get $align i32.const 1 i32.sub i32.add
                i32.const 0 local.get $align i32.sub i32.and local.tee $pointer
                local.get $size i32.add global.set $heap local.get $pointer))
        (core instance $allocation (instantiate $allocation))
        (alias core export $allocation "memory" (core memory $memory))
        (alias core export $allocation "realloc" (core func $realloc))
        (core func $read (canon lower (func $host-read) (memory $memory) (realloc $realloc)))
        (core func $list (canon lower (func $host-list) (memory $memory) (realloc $realloc)))
        (core func $write (canon lower (func $host-write) (memory $memory) (realloc $realloc)))
        (core module $guest
            (import "host" "read" (func $read (param i32 i32 i32)))
            (import "host" "list" (func $list (param i32 i32 i32)))
            (import "host" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (func (export "read") (param i32 i32) (result i32)
                local.get 0 local.get 1 i32.const 0 call $read i32.const 0)
            (func (export "list") (param i32 i32) (result i32)
                local.get 0 local.get 1 i32.const 0 call $list i32.const 0)
            (export "write" (func $write)))
        (core instance $host
            (export "read" (func $read))
            (export "list" (func $list))
            (export "write" (func $write)))
        (core instance $guest (instantiate $guest (with "host" (instance $host))))
        (func (export "read") (param "path" string) (result (option (list u8)))
            (canon lift (core func $guest "read") (memory $memory) (realloc $realloc)))
        (func (export "list") (param "path" string) (result (option string))
            (canon lift (core func $guest "list") (memory $memory) (realloc $realloc)))
        (func (export "write") (param "path" string) (param "data" (list u8)) (result bool)
            (canon lift (core func $guest "write") (memory $memory) (realloc $realloc))))
"#;
