use flow_like::flow::execution::LogLevel;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{Value, anyhow, json::json, reqwest};

pub const NOTION_API_VERSION: &str = "2026-03-11";

pub fn auth_header(access_token: &str) -> String {
    format!("Bearer {}", access_token)
}

pub async fn notion_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let error_text = resp.text().await.unwrap_or_default();
    format!("Notion API error {}: {}", status, error_text)
}

pub fn is_empty_object(value: &Value) -> bool {
    value.as_object().map(|obj| obj.is_empty()).unwrap_or(false)
}

pub fn is_empty_array(value: &Value) -> bool {
    value.as_array().map(|arr| arr.is_empty()).unwrap_or(false)
}

pub fn optional_value(value: Value) -> Option<Value> {
    if value.is_null() || is_empty_object(&value) || is_empty_array(&value) {
        None
    } else {
        Some(value)
    }
}

pub fn optional_json_value(value: Value, label: &str) -> flow_like_types::Result<Option<Value>> {
    if let Some(raw) = value.as_str() {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }

        let parsed = flow_like_types::json::from_str::<Value>(raw)
            .map_err(|err| anyhow!("Invalid {} JSON: {}", label, err))?;
        return Ok(optional_value(parsed));
    }

    Ok(optional_value(value))
}

pub fn rich_text_from_plain(text: &str) -> Value {
    json!([{ "type": "text", "text": { "content": text } }])
}

pub fn plain_text_from_rich_text(rich_text: &Value) -> String {
    rich_text
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item["plain_text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

pub fn title_from_page_properties(properties: &Value) -> String {
    properties
        .as_object()
        .and_then(|props| {
            props.values().find_map(|property| {
                if property["type"].as_str() == Some("title") {
                    Some(plain_text_from_rich_text(&property["title"]))
                } else {
                    None
                }
            })
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Untitled".to_string())
}

pub fn title_from_object(value: &Value) -> String {
    let title = plain_text_from_rich_text(&value["title"]);
    if !title.is_empty() {
        return title;
    }

    if let Some(name) = value["name"].as_str()
        && !name.is_empty()
    {
        return name.to_string();
    }

    if let Some(properties) = value.get("properties") {
        return title_from_page_properties(properties);
    }

    "Untitled".to_string()
}

pub fn block_plain_text(block: &Value) -> String {
    let block_type = block["type"].as_str().unwrap_or("");

    match block_type {
        "paragraph" | "heading_1" | "heading_2" | "heading_3" | "heading_4"
        | "bulleted_list_item" | "numbered_list_item" | "quote" | "callout" | "toggle" => {
            plain_text_from_rich_text(&block[block_type]["rich_text"])
        }
        "code" => plain_text_from_rich_text(&block["code"]["rich_text"]),
        "to_do" => {
            let text = plain_text_from_rich_text(&block["to_do"]["rich_text"]);
            let checked = block["to_do"]["checked"].as_bool().unwrap_or(false);
            format!("[{}] {}", if checked { "x" } else { " " }, text)
        }
        "divider" => "---".to_string(),
        _ => String::new(),
    }
}

pub fn file_object_from_upload_id(file_upload_id: &str) -> Value {
    json!({
        "type": "file_upload",
        "file_upload": {
            "id": file_upload_id
        }
    })
}

pub fn filename_from_path(path: &str) -> Option<String> {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .map(str::to_string)
}

pub fn content_type_from_filename(filename: &str) -> String {
    let extension = filename.rsplit_once('.').map(|(_, ext)| ext.to_lowercase());
    match extension.as_deref() {
        Some("txt") => "text/plain",
        Some("md") => "text/markdown",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("html") | Some("htm") => "text/html",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub async fn log_and_error(
    context: &mut ExecutionContext,
    message: impl AsRef<str>,
) -> flow_like_types::Result<()> {
    context.log_message(message.as_ref(), LogLevel::Error);
    context.activate_exec_pin("error").await
}
