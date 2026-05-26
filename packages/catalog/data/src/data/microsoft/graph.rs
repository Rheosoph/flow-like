use super::provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_storage::Path;
use flow_like_types::{Value, async_trait, json::json, reqwest};

const SIMPLE_UPLOAD_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
const UPLOAD_CHUNK_SIZE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GraphUploadResult {
    pub item: Value,
    pub used_upload_session: bool,
    pub size: u64,
}

pub fn normalize_graph_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() || path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

pub fn filename_from_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
}

pub fn graph_version_url(provider: &MicrosoftGraphProvider, version: &str, path: &str) -> String {
    let base = provider.base_url.trim_end_matches('/');
    let root = base
        .strip_suffix("/v1.0")
        .or_else(|| base.strip_suffix("/beta"))
        .unwrap_or(base);
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{root}/{version}{path}")
}

pub fn flow_path_filename(flow_path: &FlowPath) -> flow_like_types::Result<String> {
    filename_from_path(&flow_path.path).ok_or_else(|| {
        flow_like_types::anyhow!(
            "Destination path is empty and FlowPath has no filename: {}",
            flow_path.path
        )
    })
}

pub async fn graph_error_message(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if body.is_empty() {
        format!("API error {}", status)
    } else {
        format!("API error {}: {}", status, body)
    }
}

pub async fn graph_get_json(
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    url: impl AsRef<str>,
) -> flow_like_types::Result<Value> {
    let resp = client
        .get(url.as_ref())
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .header("Content-Type", "application/json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(flow_like_types::anyhow!(graph_error_message(resp).await));
    }

    Ok(resp.json().await?)
}

pub async fn graph_get_paginated_values(
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    first_url: impl Into<String>,
) -> flow_like_types::Result<Vec<Value>> {
    let mut url = first_url.into();
    let mut values = Vec::new();

    loop {
        let body = graph_get_json(client, provider, &url).await?;
        if let Some(items) = body["value"].as_array() {
            values.extend(items.iter().cloned());
        }

        match body["@odata.nextLink"].as_str() {
            Some(next_link) if !next_link.is_empty() => url = next_link.to_string(),
            _ => break,
        }
    }

    Ok(values)
}

pub async fn graph_send_json(
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    method: &str,
    url: impl AsRef<str>,
    body: Option<&Value>,
) -> flow_like_types::Result<(u16, Value)> {
    let mut request = match method {
        "GET" => client.get(url.as_ref()),
        "POST" => client.post(url.as_ref()),
        "PATCH" => client.patch(url.as_ref()),
        "PUT" => client.put(url.as_ref()),
        "DELETE" => client.delete(url.as_ref()),
        _ => {
            return Err(flow_like_types::anyhow!(
                "Unsupported HTTP method: {}",
                method
            ));
        }
    };

    request = request
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .header("Content-Type", "application/json");

    if let Some(body) = body {
        request = request.json(body);
    }

    let resp = request.send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(flow_like_types::anyhow!(graph_error_message(resp).await));
    }

    if status.as_u16() == 204 {
        return Ok((status.as_u16(), json!(null)));
    }

    let text = resp.text().await?;
    if text.trim().is_empty() {
        return Ok((status.as_u16(), json!(null)));
    }

    Ok((status.as_u16(), flow_like_types::json::from_str(&text)?))
}

async fn flow_path_size(
    context: &mut ExecutionContext,
    flow_path: &FlowPath,
) -> flow_like_types::Result<u64> {
    let runtime = flow_path.to_runtime(context).await?;
    let meta = runtime
        .store
        .as_generic()
        .head(&Path::from(runtime.path.as_ref()))
        .await?;
    Ok(meta.size)
}

async fn upload_with_simple_put(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    content_url: String,
    source_file: &FlowPath,
    conflict_behavior: &str,
) -> flow_like_types::Result<GraphUploadResult> {
    let bytes = source_file.get(context, false).await?;
    let size = bytes.len() as u64;
    let resp = client
        .put(content_url)
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .header("Content-Type", "application/octet-stream")
        .query(&[("@microsoft.graph.conflictBehavior", conflict_behavior)])
        .body(bytes)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(flow_like_types::anyhow!(graph_error_message(resp).await));
    }

    Ok(GraphUploadResult {
        item: resp.json().await?,
        used_upload_session: false,
        size,
    })
}

async fn upload_with_session(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    session_url: String,
    source_file: &FlowPath,
    destination_path: &str,
    conflict_behavior: &str,
    size: u64,
) -> flow_like_types::Result<GraphUploadResult> {
    let runtime = source_file.to_runtime(context).await?;
    let file_name = filename_from_path(destination_path).unwrap_or_else(|| destination_path.into());
    let body = json!({
        "item": {
            "@microsoft.graph.conflictBehavior": conflict_behavior,
            "name": file_name
        }
    });

    let session = graph_send_json(client, provider, "POST", session_url, Some(&body)).await?;
    let upload_url = session
        .1
        .get("uploadUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| flow_like_types::anyhow!("Upload session response had no uploadUrl"))?
        .to_string();

    let store = runtime.store.as_generic();
    let mut start = 0_u64;

    loop {
        let end_exclusive = std::cmp::min(start + UPLOAD_CHUNK_SIZE_BYTES, size);
        let end_inclusive = end_exclusive.saturating_sub(1);
        let bytes = store
            .get_range(&Path::from(runtime.path.as_ref()), start..end_exclusive)
            .await?;

        let resp = client
            .put(&upload_url)
            .header("Content-Length", bytes.len().to_string())
            .header(
                "Content-Range",
                format!("bytes {}-{}/{}", start, end_inclusive, size),
            )
            .body(bytes)
            .send()
            .await?;

        if resp.status().as_u16() == 202 {
            start = end_exclusive;
            continue;
        }

        if !resp.status().is_success() {
            return Err(flow_like_types::anyhow!(graph_error_message(resp).await));
        }

        return Ok(GraphUploadResult {
            item: resp.json().await?,
            used_upload_session: true,
            size,
        });
    }
}

pub async fn upload_flow_path_to_drive(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    content_url: String,
    upload_session_url: String,
    source_file: &FlowPath,
    destination_path: &str,
    conflict_behavior: &str,
) -> flow_like_types::Result<GraphUploadResult> {
    let size = flow_path_size(context, source_file).await?;
    if size <= SIMPLE_UPLOAD_LIMIT_BYTES {
        return upload_with_simple_put(
            context,
            client,
            provider,
            content_url,
            source_file,
            conflict_behavior,
        )
        .await;
    }

    upload_with_session(
        context,
        client,
        provider,
        upload_session_url,
        source_file,
        destination_path,
        conflict_behavior,
        size,
    )
    .await
}

// =============================================================================
// Generic Graph Request Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct MicrosoftGraphRequestNode {}

impl MicrosoftGraphRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MicrosoftGraphRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_graph_request",
            "Graph Request",
            "Call any Microsoft Graph endpoint with optional collection pagination",
            "Data/Microsoft",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("method", "Method", "HTTP method", VariableType::String)
            .set_default_value(Some(json!("GET")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "GET".to_string(),
                        "POST".to_string(),
                        "PATCH".to_string(),
                        "PUT".to_string(),
                        "DELETE".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "path",
            "Path",
            "Graph path like /me/messages or an absolute Graph URL",
            VariableType::String,
        );
        node.add_input_pin(
            "body",
            "Body",
            "JSON request body for POST, PATCH, or PUT",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Normal)
        .set_default_value(Some(json!(null)));
        node.add_input_pin(
            "paginate",
            "Paginate",
            "Follow @odata.nextLink for GET collection responses",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "status",
            "Status",
            "HTTP status code",
            VariableType::Integer,
        );
        node.add_output_pin(
            "response",
            "Response",
            "Raw JSON response",
            VariableType::Struct,
        );
        node.add_output_pin(
            "values",
            "Values",
            "Paginated collection values",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "next_link",
            "Next Link",
            "@odata.nextLink",
            VariableType::String,
        );
        node.add_output_pin(
            "delta_link",
            "Delta Link",
            "@odata.deltaLink",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["User.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let method: String = context.evaluate_pin("method").await?;
        let path: String = context.evaluate_pin("path").await?;
        let body: Value = context.evaluate_pin("body").await.unwrap_or(json!(null));
        let paginate: bool = context.evaluate_pin("paginate").await.unwrap_or(false);
        let url = if path.starts_with("https://") || path.starts_with("http://") {
            path
        } else {
            provider.api_url(&path)
        };

        let client = reqwest::Client::new();
        let result = if paginate && method.eq_ignore_ascii_case("GET") {
            match graph_get_paginated_values(&client, &provider, url).await {
                Ok(values) => {
                    context.set_pin_value("status", json!(200)).await?;
                    context
                        .set_pin_value("response", json!({ "value": values.clone() }))
                        .await?;
                    context.set_pin_value("values", json!(values)).await?;
                    context.set_pin_value("next_link", json!("")).await?;
                    context.set_pin_value("delta_link", json!("")).await?;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else {
            let method = method.to_uppercase();
            let body_ref = if body.is_null() { None } else { Some(&body) };
            match graph_send_json(&client, &provider, &method, url, body_ref).await {
                Ok((status, response)) => {
                    let values = response["value"].as_array().cloned().unwrap_or_default();
                    let next_link = response["@odata.nextLink"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let delta_link = response["@odata.deltaLink"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    context
                        .set_pin_value("status", json!(status as i64))
                        .await?;
                    context.set_pin_value("response", response).await?;
                    context.set_pin_value("values", json!(values)).await?;
                    context.set_pin_value("next_link", json!(next_link)).await?;
                    context
                        .set_pin_value("delta_link", json!(delta_link))
                        .await?;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };

        match result {
            Ok(()) => context.activate_exec_pin("exec_out").await?,
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
