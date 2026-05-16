use super::element_utils::{extract_element_id_from_pin, find_element};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{
    Value, async_trait,
    json::{Deserialize, Serialize, json},
};
use schemars::JsonSchema;

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

#[async_trait]
impl NodeLogic for GetFileInputFiles {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_file_input_files",
            "Get File Input Files",
            "Gets uploaded files, signed URLs, and FlowPaths from an A2UI fileInput element",
            "UI/Elements/Files",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "File input element ID or element object from Get Element",
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
            let files = extract_files(element_value);
            let signed_urls: Vec<String> = files
                .iter()
                .filter_map(|file| file.url.clone().or_else(|| file.backend_url.clone()))
                .collect();
            let flow_paths: Vec<FlowPath> = files
                .iter()
                .filter_map(|file| file.flow_path.clone())
                .collect();

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
