use crate::FlowPath;
use flow_like::flow::execution::{LogLevel, context::ExecutionContext, egress::GuardedHttpClient};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::{Path, files::store::FlowLikeStore};
use flow_like_types::{
    Cacheable,
    json::{Deserialize, Serialize},
    reqwest,
};
use schemars::JsonSchema;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "A2UIFileInputFile")]
pub struct UploadedFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Path of the file inside the picked folder, when the upload came from a folder selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_path: Option<FlowPath>,
}

impl UploadedFile {
    fn signed_url(&self) -> Option<&str> {
        self.url
            .as_deref()
            .or(self.backend_url.as_deref())
            .filter(|url| !url.is_empty())
    }
}

fn sanitize_cache_key(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();

    sanitized.truncate(64);

    if sanitized.is_empty() {
        "file_input".to_string()
    } else {
        sanitized
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let segment = value.rsplit(['/', '\\']).next().unwrap_or(value).trim();
    let sanitized = segment
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "file".to_string()
    } else {
        sanitized
    }
}

/// Decodes a desktop "local file" URL (Tauri `convertFileSrc`: `asset://localhost/…`
/// or `http://asset.localhost/…`) back to its absolute on-disk path. Returns `None`
/// for regular http(s) URLs, which are downloaded instead.
pub fn decode_local_file_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str().unwrap_or("");
    let is_local = parsed.scheme() == "file"
        || (parsed.scheme() == "asset" && host == "localhost")
        || ((parsed.scheme() == "http" || parsed.scheme() == "https") && host == "asset.localhost");
    if !is_local {
        return None;
    }

    let decoded = urlencoding::decode(parsed.path().trim_start_matches('/'))
        .ok()?
        .into_owned();
    if decoded.is_empty() {
        return None;
    }

    // Windows drive paths (C:/…) are already absolute; POSIX paths need the slash back.
    let is_windows_drive = decoded.as_bytes().get(1) == Some(&b':');
    Some(if is_windows_drive {
        decoded
    } else {
        format!("/{decoded}")
    })
}

fn file_name_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let segment = parsed
        .path_segments()?
        .rfind(|segment| !segment.is_empty())?;
    Some(
        urlencoding::decode(segment)
            .map(|decoded| decoded.into_owned())
            .unwrap_or_else(|_| segment.to_string()),
    )
}

fn flow_path_file_name(file: &UploadedFile, index: usize) -> String {
    let raw_name = file
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .or_else(|| file.signed_url().and_then(file_name_from_url))
        .unwrap_or_else(|| "file".to_string());

    format!("{:03}_{}", index + 1, sanitize_path_segment(&raw_name))
}

async fn create_memory_store(context: &mut ExecutionContext, element_id: &str) -> String {
    let store_ref = format!(
        "a2ui_file_input_files_{}_{}",
        sanitize_cache_key(element_id),
        Uuid::new_v4()
    );
    let store = FlowLikeStore::Memory(Arc::new(
        flow_like_storage::object_store::memory::InMemory::new(),
    ));
    let store: Arc<dyn Cacheable> = Arc::new(store);
    context.set_cache(&store_ref, store).await;
    store_ref
}

/// Upper bound on a single frontend-supplied upload fetched into memory. Mirrors
/// the API's `MAX_ATTACHMENT_BYTES` upload cap so the executor cannot be driven to
/// OOM by an oversized or dishonest download.
const MAX_FILE_INPUT_DOWNLOAD_BYTES: usize = 512 * 1024 * 1024;

async fn download_file_input_url(
    client: &GuardedHttpClient,
    url: &str,
    name: &str,
) -> flow_like_types::Result<(Vec<u8>, Option<String>)> {
    use futures::StreamExt;

    // Frontend-supplied URLs are dereferenced from inside the executor. Reject
    // non-http(s) schemes so local-resource URLs (file:, data:, etc.) are never
    // fetched. Server-side the guarded client also refuses the host plane
    // (metadata, loopback, link-local); private ranges stay reachable for
    // self-hosted upload backends.
    let scheme = reqwest::Url::parse(url)
        .map(|parsed| parsed.scheme().to_string())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(flow_like_types::anyhow!(
            "Refusing to download uploaded file \"{}\": unsupported URL scheme",
            name
        ));
    }

    let response = client.get(url)?.send().await.map_err(|err| {
        flow_like_types::anyhow!(
            "Failed to download uploaded file \"{}\": {}",
            name,
            err.without_url()
        )
    })?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(flow_like_types::anyhow!(
            "Failed to download uploaded file \"{}\" with status {}",
            name,
            status
        ));
    }

    if response
        .content_length()
        .is_some_and(|len| len > MAX_FILE_INPUT_DOWNLOAD_BYTES as u64)
    {
        return Err(flow_like_types::anyhow!(
            "Uploaded file \"{}\" exceeds the {} byte download limit",
            name,
            MAX_FILE_INPUT_DOWNLOAD_BYTES
        ));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .filter(|value| !value.is_empty());

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| {
            flow_like_types::anyhow!(
                "Failed to download uploaded file \"{}\": {}",
                name,
                err.without_url()
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_FILE_INPUT_DOWNLOAD_BYTES {
            return Err(flow_like_types::anyhow!(
                "Uploaded file \"{}\" exceeds the {} byte download limit",
                name,
                MAX_FILE_INPUT_DOWNLOAD_BYTES
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok((bytes, content_type))
}

/// Existence checks per flight. A folder upload can hand us thousands of files, and
/// one round trip at a time would dominate the node's runtime.
const FLOW_PATH_CHECK_CONCURRENCY: usize = 16;

/// Checks every supplied FlowPath concurrently. Store resolution stays sequential
/// because it needs the execution context, but it only reads the context cache.
async fn existing_flow_paths(
    context: &mut ExecutionContext,
    files: &[UploadedFile],
) -> flow_like_types::Result<Vec<bool>> {
    use futures::StreamExt;

    let mut runtimes = Vec::with_capacity(files.len());
    for file in files {
        match file.flow_path.as_ref() {
            Some(flow_path) => runtimes.push(Some((
                flow_path.path.clone(),
                flow_path.to_runtime(context).await?,
            ))),
            None => runtimes.push(None),
        }
    }

    let mut present = vec![false; files.len()];
    let mut checks = futures::stream::iter(runtimes.into_iter().enumerate().map(
        |(index, runtime)| async move {
            let Some((path, runtime)) = runtime else {
                return Ok::<(usize, bool), flow_like_types::Error>((index, false));
            };

            match runtime.store.as_generic().head(&runtime.path).await {
                Ok(_) => Ok((index, true)),
                Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok((index, false)),
                Err(error) => Err(flow_like_types::anyhow!(
                    "Failed to check uploaded file at {}: {}",
                    path,
                    error
                )),
            }
        },
    ))
    .buffer_unordered(FLOW_PATH_CHECK_CONCURRENCY);

    while let Some(result) = checks.next().await {
        let (index, exists) = result?;
        present[index] = exists;
    }

    Ok(present)
}

/// Makes uploaded bytes available in this run and returns their FlowPaths.
pub async fn materialize_uploaded_files(
    context: &mut ExecutionContext,
    scope_label: &str,
    files: &mut [UploadedFile],
) -> flow_like_types::Result<Vec<FlowPath>> {
    let needs_memory_store = files
        .iter()
        .any(|file| file.flow_path.is_none() && file.signed_url().is_some());

    let store_ref = if needs_memory_store {
        Some(create_memory_store(context, scope_label).await)
    } else {
        None
    };
    let client = GuardedHttpClient::new(context.execution_environment())?;
    let mut flow_paths = Vec::new();
    let present = existing_flow_paths(context, files).await?;

    for (index, file) in files.iter_mut().enumerate() {
        let supplied_flow_path = file.flow_path.clone();

        if let Some(flow_path) = supplied_flow_path.clone()
            && present[index]
        {
            flow_paths.push(flow_path);
            continue;
        }

        let Some(url) = file.signed_url().map(str::to_string) else {
            match supplied_flow_path {
                // Preserve the prior behavior for FlowPaths that cannot be checked or repaired
                // because the frontend did not provide a signed/local source URL.
                Some(flow_path) => flow_paths.push(flow_path),
                None => context.log_message(
                    "File input item did not contain a URL or FlowPath; skipping FlowPath creation",
                    LogLevel::Warn,
                ),
            }
            continue;
        };

        // Desktop "local file" URLs (Tauri convertFileSrc) can't be HTTP-fetched by
        // the engine. Resolve them to their on-disk path and register a store for
        // that file instead of downloading.
        if let Some(local_path) = decode_local_file_url(&url) {
            context
                .execution_environment()
                .ensure_host_filesystem_access("File input local file URL")?;
            let local_path = PathBuf::from(local_path);
            if local_path.is_file() {
                let flow_path = FlowPath::from_pathbuf(local_path, context).await?;
                file.flow_path = Some(flow_path.clone());
                flow_paths.push(flow_path);
                continue;
            }
        }

        let file_name = flow_path_file_name(file, index);
        let target_flow_path = match supplied_flow_path {
            Some(flow_path) => {
                context.log_message(
                    &format!(
                        "Uploaded file \"{}\" was missing from its execution store; materializing it from the signed URL",
                        file_name
                    ),
                    LogLevel::Info,
                );
                flow_path
            }
            None => {
                let store_ref = store_ref.as_ref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "No execution store was prepared for uploaded file \"{}\"",
                        file_name
                    )
                })?;
                let object_path = Path::from("files").join(file_name.as_str());
                FlowPath::new(object_path.as_ref().to_string(), store_ref.clone(), None)
            }
        };

        let (bytes, content_type) = download_file_input_url(&client, &url, &file_name).await?;
        if file.mime_type.is_none() {
            file.mime_type = content_type;
        }
        target_flow_path.put(context, bytes, true).await?;

        file.flow_path = Some(target_flow_path.clone());
        flow_paths.push(target_flow_path);
    }

    Ok(flow_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, Run, internal_node::InternalNode, internal_pin::InternalPin},
            node::{Node, NodeLogic},
            variable::Variable,
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::files::store::local_store::LocalObjectStore;
    use flow_like_types::{
        sync::{Mutex, RwLock},
        tokio::io::{AsyncReadExt, AsyncWriteExt},
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Weak},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("flow-like-file-input-test-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct UploadTestNode;

    #[flow_like_types::async_trait]
    impl NodeLogic for UploadTestNode {
        fn get_node(&self) -> Node {
            Node::new("uploaded_files_test", "Uploaded files test", "", "Tests")
        }

        async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    fn internal_node() -> Arc<InternalNode> {
        let logic: Arc<dyn NodeLogic> = Arc::new(UploadTestNode);
        let node = logic.get_node();
        let mut pins = AHashMap::new();
        let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

        for pin in node.pins.values() {
            let internal_pin = Arc::new(InternalPin::new(pin, false));
            name_cache
                .entry(pin.name.clone())
                .or_default()
                .push(internal_pin.clone());
            pins.insert(pin.id.clone(), internal_pin);
        }

        let internal = Arc::new(InternalNode::new(node, pins, logic, name_cache));
        for pin in internal.pins.iter() {
            pin.init_node(Arc::downgrade(&internal));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        internal
    }

    async fn test_context() -> ExecutionContext {
        let current = internal_node();
        let mut node_map = AHashMap::new();
        node_map.insert(current.node_id().to_string(), current.clone());

        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let variables = Arc::new(Mutex::new(AHashMap::<String, Variable>::new()));
        let cache = Arc::new(RwLock::new(AHashMap::<String, Arc<dyn Cacheable>>::new()));
        let run: Weak<Mutex<Run>> = Weak::new();

        ExecutionContext::new(
            Arc::new(node_map),
            &run,
            &state,
            &current,
            &variables,
            &cache,
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

    async fn serve_once(body: Vec<u8>) -> String {
        let listener = flow_like_types::tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");

        flow_like_types::tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket
                .write_all(headers.as_bytes())
                .await
                .expect("write response headers");
            socket.write_all(&body).await.expect("write response body");
        });

        format!("http://{address}/uploaded-file")
    }

    async fn register_local_store(
        context: &ExecutionContext,
        root: &TestDirectory,
        store_ref: &str,
    ) {
        let store = LocalObjectStore::new(root.0.clone()).expect("create local store");
        let store: Arc<dyn Cacheable> = Arc::new(FlowLikeStore::Local(Arc::new(store)));
        context.set_cache(store_ref, store).await;
    }

    #[flow_like_types::tokio::test]
    async fn missing_supplied_flow_path_is_materialized_into_its_registered_store() {
        let mut context = test_context().await;
        let root = TestDirectory::new();
        let store_ref = "request-files";
        register_local_store(&context, &root, store_ref).await;

        let expected = b"uploaded markdown".to_vec();
        let url = serve_once(expected.clone()).await;
        let flow_path = FlowPath::new(
            "tmp/user/test/apps/app/file.md".to_string(),
            store_ref.to_string(),
            None,
        );
        let mut files = vec![UploadedFile {
            name: Some("file.md".to_string()),
            url: Some(url),
            flow_path: Some(flow_path.clone()),
            ..Default::default()
        }];

        let result = materialize_uploaded_files(&mut context, "file-input", &mut files)
            .await
            .expect("materialize missing uploaded file");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, flow_path.path);
        assert_eq!(result[0].store_ref, flow_path.store_ref);
        assert_eq!(
            std::fs::read(root.0.join("tmp/user/test/apps/app/file.md"))
                .expect("read materialized file"),
            expected
        );
    }

    #[flow_like_types::tokio::test]
    async fn existing_supplied_flow_path_does_not_fetch_its_url_again() {
        let mut context = test_context().await;
        let root = TestDirectory::new();
        let store_ref = "request-files";
        register_local_store(&context, &root, store_ref).await;

        let flow_path = FlowPath::new(
            "tmp/user/test/apps/app/file.md".to_string(),
            store_ref.to_string(),
            None,
        );
        flow_path
            .put(&mut context, b"already present".to_vec(), true)
            .await
            .expect("seed uploaded file");
        let mut files = vec![UploadedFile {
            name: Some("file.md".to_string()),
            url: Some("http://127.0.0.1:1/must-not-be-requested".to_string()),
            flow_path: Some(flow_path.clone()),
            ..Default::default()
        }];

        let result = materialize_uploaded_files(&mut context, "file-input", &mut files)
            .await
            .expect("reuse existing uploaded file");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, flow_path.path);
        assert_eq!(
            std::fs::read(root.0.join("tmp/user/test/apps/app/file.md"))
                .expect("read existing file"),
            b"already present"
        );
    }

    #[flow_like_types::tokio::test]
    async fn local_asset_url_without_flow_path_registers_its_parent_store() {
        let mut context = test_context().await;
        let root = TestDirectory::new();
        let file_path = root.0.join("local-upload.md");
        let expected = b"local desktop upload";
        std::fs::write(&file_path, expected).expect("write local upload");

        let mut files = vec![UploadedFile {
            name: Some("local-upload.md".to_string()),
            url: Some(format!("asset://localhost{}", file_path.display())),
            ..Default::default()
        }];

        let result = materialize_uploaded_files(&mut context, "file-input", &mut files)
            .await
            .expect("materialize local asset URL");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "local-upload.md");
        let runtime = result[0]
            .to_runtime(&mut context)
            .await
            .expect("resolve registered local store");
        let FlowLikeStore::Local(store) = runtime.store.as_ref() else {
            panic!("local asset URL must register a LocalObjectStore");
        };
        let resolved_path = store
            .path_to_filesystem(&runtime.path)
            .expect("resolve local file path")
            .canonicalize()
            .expect("canonicalize resolved local file path");
        assert_eq!(
            resolved_path,
            file_path
                .canonicalize()
                .expect("canonicalize expected local file path")
        );
        assert_eq!(
            result[0]
                .get(&mut context, true)
                .await
                .expect("read local upload through FlowPath"),
            expected
        );
    }
}
