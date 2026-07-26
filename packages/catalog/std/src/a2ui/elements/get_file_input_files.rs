use super::element_utils::{extract_element_id_from_pin, find_element};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_storage::{Path, files::store::FlowLikeStore};
use flow_like_types::{
    Cacheable, Value, async_trait,
    json::{Deserialize, Serialize, json},
    reqwest,
};
use schemars::JsonSchema;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct A2UIFileInputFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_path: Option<FlowPath>,
}

impl A2UIFileInputFile {
    fn signed_url(&self) -> Option<&str> {
        self.url
            .as_deref()
            .or(self.backend_url.as_deref())
            .filter(|url| !url.is_empty())
    }
}

/// Gets files selected in an A2UI fileInput element.
///
/// FileInput uploads through the same temporary upload path as chat. This node returns the
/// current uploaded file objects, their signed/local URLs, and any FlowPaths provided by the
/// frontend upload endpoint.
#[crate::register_node]
#[derive(Default)]
pub struct GetFileInputFiles;

impl GetFileInputFiles {
    pub fn new() -> Self {
        Self
    }
}

fn decode_bound_value(raw_value: &Value) -> Value {
    let Some(obj) = raw_value.as_object() else {
        return raw_value.clone();
    };

    if let Some(v) = obj.get("literalJson") {
        return v
            .as_str()
            .and_then(|s| flow_like_types::json::from_str::<Value>(s).ok())
            .unwrap_or_else(|| v.clone());
    }

    if let Some(v) = obj.get("literalString") {
        if let Some(s) = v.as_str() {
            return flow_like_types::json::from_str::<Value>(s)
                .unwrap_or_else(|_| Value::String(s.to_string()));
        }
        return v.clone();
    }

    if let Some(v) = obj.get("literalOptions") {
        return v.clone();
    }

    raw_value.clone()
}

fn value_to_file(value: Value) -> Option<A2UIFileInputFile> {
    match value {
        Value::String(url) if !url.is_empty() => Some(A2UIFileInputFile {
            url: Some(url),
            ..Default::default()
        }),
        Value::Object(obj) => {
            let flow_path = obj
                .get("flowPath")
                .or_else(|| obj.get("flow_path"))
                .and_then(|v| flow_like_types::json::from_value::<FlowPath>(v.clone()).ok());

            Some(A2UIFileInputFile {
                name: obj.get("name").and_then(|v| v.as_str()).map(str::to_string),
                size: obj.get("size").and_then(|v| v.as_u64()),
                mime_type: obj
                    .get("type")
                    .or_else(|| obj.get("mimeType"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                url: obj
                    .get("url")
                    .or_else(|| obj.get("backendUrl"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                backend_url: obj
                    .get("backendUrl")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                flow_path,
            })
        }
        _ => None,
    }
}

fn extract_files(element_value: &Value) -> Vec<A2UIFileInputFile> {
    let raw_value = element_value
        .get("component")
        .and_then(|c| c.get("value"))
        .cloned()
        .unwrap_or(Value::Null);

    let value = decode_bound_value(&raw_value);

    match value {
        Value::Array(values) => values.into_iter().filter_map(value_to_file).collect(),
        Value::Null => Vec::new(),
        other => value_to_file(other).into_iter().collect(),
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
fn decode_local_file_url(url: &str) -> Option<String> {
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
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    Some(
        urlencoding::decode(segment)
            .map(|decoded| decoded.into_owned())
            .unwrap_or_else(|_| segment.to_string()),
    )
}

fn flow_path_file_name(file: &A2UIFileInputFile, index: usize) -> String {
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
    client: &reqwest::Client,
    url: &str,
    name: &str,
) -> flow_like_types::Result<(Vec<u8>, Option<String>)> {
    use futures::StreamExt;

    // Frontend-supplied URLs are dereferenced from inside the executor. Reject
    // non-http(s) schemes so local-resource URLs (file:, data:, etc.) are never
    // fetched. NOTE: this does not block http(s) URLs targeting private/loopback
    // hosts (SSRF residue) — self-hosted upload backends legitimately live on
    // loopback/private ranges, so a blanket IP blocklist would break them.
    let scheme = reqwest::Url::parse(url)
        .map(|parsed| parsed.scheme().to_string())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(flow_like_types::anyhow!(
            "Refusing to download uploaded file \"{}\": unsupported URL scheme",
            name
        ));
    }

    let response = client.get(url).send().await.map_err(|err| {
        flow_like_types::anyhow!("Failed to download uploaded file \"{}\": {}", name, err)
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let body = body.chars().take(512).collect::<String>();
        return Err(flow_like_types::anyhow!(
            "Failed to download uploaded file \"{}\" with status {}: {}",
            name,
            status,
            body
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
            flow_like_types::anyhow!("Failed to download uploaded file \"{}\": {}", name, err)
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

async fn flow_path_exists(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
) -> flow_like_types::Result<bool> {
    let runtime = flow_path.to_runtime(context).await?;
    match runtime.store.as_generic().head(&runtime.path).await {
        Ok(_) => Ok(true),
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(false),
        Err(error) => Err(flow_like_types::anyhow!(
            "Failed to check uploaded file at {}: {}",
            flow_path.path,
            error
        )),
    }
}

async fn materialize_missing_flow_paths(
    context: &mut ExecutionContext,
    element_id: &str,
    files: &mut [A2UIFileInputFile],
) -> flow_like_types::Result<Vec<FlowPath>> {
    let needs_memory_store = files
        .iter()
        .any(|file| file.flow_path.is_none() && file.signed_url().is_some());

    let store_ref = if needs_memory_store {
        Some(create_memory_store(context, element_id).await)
    } else {
        None
    };
    let client = reqwest::Client::new();
    let mut flow_paths = Vec::new();

    for (index, file) in files.iter_mut().enumerate() {
        let supplied_flow_path = file.flow_path.clone();
        let target_flow_path = match supplied_flow_path {
            Some(flow_path) if flow_path_exists(context, &flow_path).await? => {
                flow_paths.push(flow_path);
                continue;
            }
            Some(flow_path) => flow_path,
            None => {
                let file_name = flow_path_file_name(file, index);
                let object_path = Path::from("files").child(file_name.as_str());
                FlowPath::new(
                    object_path.as_ref().to_string(),
                    store_ref
                        .as_ref()
                        .expect("memory store exists when a file URL must be materialized")
                        .clone(),
                    None,
                )
            }
        };

        let Some(url) = file.signed_url().map(str::to_string) else {
            if file.flow_path.is_some() {
                // Preserve the prior behavior for FlowPaths that cannot be checked or repaired
                // because the frontend did not provide a signed/local source URL.
                flow_paths.push(target_flow_path);
            } else {
                context.log_message(
                    "File input item did not contain a URL or FlowPath; skipping FlowPath creation",
                    LogLevel::Warn,
                );
            }
            continue;
        };

        // Desktop "local file" URLs (Tauri convertFileSrc) can't be HTTP-fetched by
        // the engine. Resolve them to their on-disk path and register a store for
        // that file instead of downloading.
        if let Some(local_path) = decode_local_file_url(&url) {
            let local_path = PathBuf::from(local_path);
            if local_path.is_file() {
                let flow_path = FlowPath::from_pathbuf(local_path, context).await?;
                file.flow_path = Some(flow_path.clone());
                flow_paths.push(flow_path);
                continue;
            }
        }

        let file_name = flow_path_file_name(file, index);
        if file.flow_path.is_some() {
            context.log_message(
                &format!(
                    "Uploaded file \"{}\" was missing from its execution store; materializing it from the signed URL",
                    file_name
                ),
                LogLevel::Info,
            );
        }
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

#[async_trait]
impl NodeLogic for GetFileInputFiles {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_file_input_files",
            "Get File Input Files",
            "Gets uploaded files, signed URLs, and FlowPaths from an A2UI fileInput or voiceInput element",
            "UI/Elements/Files",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "File or voice input element ID or element object from Get Element",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "files",
            "Files",
            "Uploaded file objects",
            VariableType::Struct,
        )
        .set_schema::<A2UIFileInputFile>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "signed_urls",
            "Signed URLs",
            "Signed or local URLs for the uploaded files",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "flow_paths",
            "FlowPaths",
            "Temporary FlowPaths for uploaded files when available",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the file input element exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;

        let elements = context.get_frontend_elements().await?;
        let element = elements.as_ref().and_then(|e| find_element(e, &element_id));

        if let Some((_found_id, element_value)) = element {
            let mut files = extract_files(element_value);
            let signed_urls: Vec<String> = files
                .iter()
                .filter_map(|file| file.url.clone().or_else(|| file.backend_url.clone()))
                .collect();
            let flow_paths =
                materialize_missing_flow_paths(context, &element_id, &mut files).await?;

            context.set_pin_value("files", json!(files)).await?;
            context
                .set_pin_value("signed_urls", json!(signed_urls))
                .await?;
            context
                .set_pin_value("flow_paths", json!(flow_paths))
                .await?;
            context.set_pin_value("exists", json!(true)).await?;
        } else {
            context.set_pin_value("files", json!([])).await?;
            context.set_pin_value("signed_urls", json!([])).await?;
            context.set_pin_value("flow_paths", json!([])).await?;
            context.set_pin_value("exists", json!(false)).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, Run, internal_node::InternalNode, internal_pin::InternalPin},
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

    fn internal_node() -> Arc<InternalNode> {
        let logic: Arc<dyn NodeLogic> = Arc::new(GetFileInputFiles::new());
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
        for pin in internal.pins.values() {
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
        let mut files = vec![A2UIFileInputFile {
            name: Some("file.md".to_string()),
            url: Some(url),
            flow_path: Some(flow_path.clone()),
            ..Default::default()
        }];

        let result = materialize_missing_flow_paths(&mut context, "file-input", &mut files)
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
        let mut files = vec![A2UIFileInputFile {
            name: Some("file.md".to_string()),
            url: Some("http://127.0.0.1:1/must-not-be-requested".to_string()),
            flow_path: Some(flow_path.clone()),
            ..Default::default()
        }];

        let result = materialize_missing_flow_paths(&mut context, "file-input", &mut files)
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

        let mut files = vec![A2UIFileInputFile {
            name: Some("local-upload.md".to_string()),
            url: Some(format!("asset://localhost{}", file_path.display())),
            ..Default::default()
        }];

        let result = materialize_missing_flow_paths(&mut context, "file-input", &mut files)
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
