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

async fn download_file_input_url(
    client: &reqwest::Client,
    url: &str,
    name: &str,
) -> flow_like_types::Result<Vec<u8>> {
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

    Ok(response.bytes().await?.to_vec())
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
        if let Some(flow_path) = file.flow_path.clone() {
            flow_paths.push(flow_path);
            continue;
        }

        let Some(url) = file.signed_url().map(str::to_string) else {
            context.log_message(
                "File input item did not contain a URL or FlowPath; skipping FlowPath creation",
                LogLevel::Warn,
            );
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
        let object_path = Path::from("files").child(file_name.as_str());
        let flow_path = FlowPath::new(
            object_path.as_ref().to_string(),
            store_ref
                .as_ref()
                .expect("memory store exists when a file URL must be materialized")
                .clone(),
            None,
        );
        let bytes = download_file_input_url(&client, &url, &file_name).await?;
        flow_path.put(context, bytes, true).await?;

        file.flow_path = Some(flow_path.clone());
        flow_paths.push(flow_path);
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
