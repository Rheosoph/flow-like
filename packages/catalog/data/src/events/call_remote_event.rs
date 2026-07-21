use super::chat_event::{Attachment, ChatAction, ChatUsageStat, ChatWidget, Reasoning, User};
use crate::data::path::FlowPath;
use crate::remote_util::{
    RemoteAppSession, RemoteSseEvent, RemoteSseEventHandler, error_for_status,
    follow_get_redirect_without_credentials, http_client_no_redirect, invoke_and_collect,
    invoke_and_collect_with_handler, post_json, remote_app_session, validate_path_id,
    with_event_registration_headers,
};
use ahash::AHashSet;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_model_provider::{
    history::History, response::Response, response_chunk::ResponseChunk,
};
use flow_like_types::{Value, async_trait, json::json};
use serde::Deserialize;
use std::collections::HashMap;

/// Pin names special-cased by the flow editor. The project/event pins render
/// interactive dropdowns; the meta pin is auto-filled with the event's typed
/// details (RemoteEventDetail JSON) and drives dynamic pin generation.
const PIN_REMOTE_APP_ID: &str = "_flow_remote_app_id";
const PIN_REMOTE_EVENT: &str = "_flow_remote_event";
const PIN_REMOTE_EVENT_META: &str = "_flow_remote_event_meta";

const RESERVED_INPUTS: &[&str] = &[
    "exec_in",
    PIN_REMOTE_APP_ID,
    PIN_REMOTE_EVENT,
    PIN_REMOTE_EVENT_META,
];
const RESERVED_OUTPUTS: &[&str] = &["exec_out"];

const MCP_MODE_CALL_TOOL: &str = "Call Tool";
const MCP_MODE_READ_RESOURCE: &str = "Read Resource";

// ---------------------------------------------------------------------------
// Event metadata (mirror of the API's RemoteEventDetail)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct EventMeta {
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    rest_routes: Vec<RouteMeta>,
    #[serde(default)]
    rest_files: Vec<FileMeta>,
    #[serde(default)]
    mcp_tools: Vec<ToolMeta>,
    #[serde(default)]
    mcp_resources: Vec<ResourceMeta>,
}

#[derive(Debug, Clone, Deserialize)]
struct RouteMeta {
    method: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FileMeta {
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolMeta {
    name: String,
    #[serde(default)]
    input_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResourceMeta {
    uri: String,
    #[serde(default)]
    mime_type: Option<String>,
}

fn parse_meta(node: &Node) -> Option<EventMeta> {
    let raw = pin_string(node, PIN_REMOTE_EVENT_META);
    if raw.trim().is_empty() {
        return None;
    }
    flow_like_types::json::from_str::<EventMeta>(&raw).ok()
}

/// Reads a sibling pin's persisted string value (JSON-encoded bytes → string).
fn pin_string(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Dynamic pin reconciliation
// ---------------------------------------------------------------------------

struct PinSpec {
    name: String,
    friendly: String,
    description: String,
    data_type: VariableType,
    value_type: ValueType,
    schema: Option<String>,
    enforce_schema: bool,
    valid_values: Option<Vec<String>>,
    default: Option<Value>,
}

impl PinSpec {
    fn new(name: &str, friendly: &str, description: &str, data_type: VariableType) -> Self {
        Self {
            name: name.to_string(),
            friendly: friendly.to_string(),
            description: description.to_string(),
            data_type,
            value_type: ValueType::Normal,
            schema: None,
            enforce_schema: false,
            valid_values: None,
            default: None,
        }
    }

    fn array(mut self) -> Self {
        self.value_type = ValueType::Array;
        self
    }

    fn schema(mut self, schema: String) -> Self {
        self.schema = Some(schema);
        self
    }

    fn enforce(mut self) -> Self {
        self.enforce_schema = true;
        self
    }

    fn dropdown(mut self, values: Vec<String>) -> Self {
        self.valid_values = Some(values);
        self
    }

    fn default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self
    }
}

fn build_options(spec: &PinSpec) -> Option<PinOptions> {
    if spec.valid_values.is_none() && !spec.enforce_schema {
        return None;
    }
    let mut options = PinOptions::new();
    if let Some(values) = &spec.valid_values {
        options.set_valid_values(values.clone());
    }
    if spec.enforce_schema {
        options.set_enforce_schema(true);
    }
    Some(options.build())
}

fn apply_spec(pin: &mut flow_like::flow::pin::Pin, spec: &PinSpec) {
    pin.value_type = spec.value_type.clone();
    pin.schema = spec.schema.clone();
    pin.options = build_options(spec);
}

/// Adds missing input pins, refreshes the type/schema/options of existing pins
/// in place (preserving the user's value & connections), and removes input pins
/// that are no longer desired.
fn reconcile_inputs(node: &mut Node, desired: &[PinSpec]) {
    for spec in desired {
        if let Some(pin) = node
            .pins
            .values_mut()
            .find(|pin| pin.pin_type == PinType::Input && pin.name == spec.name)
        {
            pin.data_type = spec.data_type.clone();
            pin.value_type = spec.value_type.clone();
            pin.schema = spec.schema.clone();
            pin.options = build_options(spec);
            if pin.default_value.is_none()
                && let Some(default) = &spec.default
            {
                pin.set_default_value(Some(default.clone()));
            }
        } else {
            let pin = node.add_input_pin(
                &spec.name,
                &spec.friendly,
                &spec.description,
                spec.data_type.clone(),
            );
            apply_spec(pin, spec);
            if let Some(default) = &spec.default {
                pin.set_default_value(Some(default.clone()));
            }
        }
    }

    let keep: Vec<String> = desired.iter().map(|spec| spec.name.clone()).collect();
    node.pins.retain(|_, pin| {
        if pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution {
            RESERVED_INPUTS.contains(&pin.name.as_str()) || keep.contains(&pin.name)
        } else {
            true
        }
    });
}

fn reconcile_outputs(node: &mut Node, desired: &[PinSpec]) {
    for spec in desired {
        if let Some(pin) = node
            .pins
            .values_mut()
            .find(|pin| pin.pin_type == PinType::Output && pin.name == spec.name)
        {
            pin.data_type = spec.data_type.clone();
            apply_spec(pin, spec);
        } else {
            let pin = node.add_output_pin(
                &spec.name,
                &spec.friendly,
                &spec.description,
                spec.data_type.clone(),
            );
            apply_spec(pin, spec);
        }
    }

    let keep: Vec<String> = desired.iter().map(|spec| spec.name.clone()).collect();
    node.pins.retain(|_, pin| {
        if pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution {
            RESERVED_OUTPUTS.contains(&pin.name.as_str()) || keep.contains(&pin.name)
        } else {
            true
        }
    });
}

fn timeout_spec() -> PinSpec {
    PinSpec::new(
        "timeout_seconds",
        "Timeout (s)",
        "Maximum time to wait for the remote request to finish",
        VariableType::Integer,
    )
    .default(json!(120))
}

// ---------------------------------------------------------------------------
// Desired pins per event type
// ---------------------------------------------------------------------------

fn route_label(method: &str, path: &str) -> String {
    format!("{} {}", method.to_uppercase(), path)
}

fn is_file_route(meta: &EventMeta, method: &str, path: &str) -> bool {
    method.eq_ignore_ascii_case("GET") && meta.rest_files.iter().any(|file| file.path == path)
}

fn selected_route<'a>(meta: &'a EventMeta, selection: &str) -> Option<(String, String)> {
    for route in &meta.rest_routes {
        if route_label(&route.method, &route.path) == selection {
            return Some((route.method.clone(), route.path.clone()));
        }
    }
    for file in &meta.rest_files {
        if route_label("GET", &file.path) == selection {
            return Some(("GET".to_string(), file.path.clone()));
        }
    }
    None
}

fn json_schema_type(schema: &Value) -> VariableType {
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("string") => VariableType::String,
        Some("integer") => VariableType::Integer,
        Some("number") => VariableType::Float,
        Some("boolean") => VariableType::Boolean,
        _ => VariableType::Generic,
    }
}

fn chat_desired(node: &Node) -> (Vec<PinSpec>, Vec<PinSpec>) {
    let _ = node;
    let inputs = vec![
        PinSpec::new(
            "message",
            "Message",
            "User message appended to the conversation",
            VariableType::String,
        )
        .default(json!("")),
        PinSpec::new(
            "history",
            "History",
            "Prior conversation history",
            VariableType::Struct,
        )
        .schema(schema_string::<History>())
        .enforce()
        .default(json!(History::new(String::new(), Vec::new()))),
        PinSpec::new(
            "local_session",
            "Local Session",
            "Local session state",
            VariableType::Struct,
        )
        .default(json!({})),
        PinSpec::new(
            "global_session",
            "Global Session",
            "Global session state",
            VariableType::Struct,
        )
        .default(json!({})),
        PinSpec::new(
            "tools",
            "Tools",
            "Tool ids the assistant may use",
            VariableType::String,
        )
        .array()
        .default(json!([])),
        PinSpec::new(
            "attachments",
            "Attachments",
            "Attachments to include",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<Attachment>())
        .enforce()
        .default(json!([])),
        timeout_spec(),
    ];
    let outputs = vec![
        PinSpec::new(
            "response",
            "Response",
            "Full chat response",
            VariableType::Generic,
        ),
        PinSpec::new(
            "response_text",
            "Response Text",
            "Best-effort final text of the response",
            VariableType::String,
        ),
        PinSpec::new("run_id", "Run ID", "Remote run id", VariableType::String),
        PinSpec::new("status", "Status", "Final run status", VariableType::String),
    ];
    (inputs, outputs)
}

/// Stable contract for the dedicated remote-chat node. Unlike the legacy
/// adaptive node, every chat output is present before an event is selected so
/// the node is useful from both the visual editor and FlowScript.
fn remote_chat_desired() -> (Vec<PinSpec>, Vec<PinSpec>) {
    let inputs = vec![
        PinSpec::new(
            "message",
            "Message",
            "User message appended to the conversation",
            VariableType::String,
        )
        .default(json!("")),
        PinSpec::new(
            "history",
            "History",
            "Prior conversation history",
            VariableType::Struct,
        )
        .schema(schema_string::<History>())
        .enforce()
        .default(json!(History::new(String::new(), Vec::new()))),
        PinSpec::new(
            "local_session",
            "Local State",
            "State local to this chat session",
            VariableType::Struct,
        )
        .default(json!({})),
        PinSpec::new(
            "global_session",
            "Global State",
            "State shared for the remote chat user",
            VariableType::Struct,
        )
        .default(json!({})),
        PinSpec::new(
            "tools",
            "Tools",
            "Tool ids the remote assistant may use",
            VariableType::String,
        )
        .array()
        .default(json!([])),
        PinSpec::new(
            "actions",
            "Actions",
            "User actions included with the chat request",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<ChatAction>())
        .enforce()
        .default(json!([])),
        PinSpec::new(
            "attachments",
            "Attachments",
            "Attachments included with the chat request",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<Attachment>())
        .enforce()
        .default(json!([])),
        PinSpec::new(
            "user",
            "User",
            "User information forwarded to the remote chat",
            VariableType::Struct,
        )
        .schema(schema_string::<User>())
        .enforce()
        .default(Value::Null),
        timeout_spec(),
    ];

    let outputs = vec![
        PinSpec::new(
            "chunk",
            "Chunk",
            "Latest streamed response chunk",
            VariableType::Struct,
        )
        .schema(schema_string::<ResponseChunk>())
        .enforce(),
        PinSpec::new(
            "response",
            "Response",
            "Latest complete model response",
            VariableType::Struct,
        )
        .schema(schema_string::<Response>())
        .enforce(),
        PinSpec::new(
            "response_text",
            "Response Text",
            "Text of the latest complete response",
            VariableType::String,
        ),
        PinSpec::new(
            "widgets",
            "Widgets",
            "Widgets emitted by the remote chat update",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<ChatWidget>())
        .enforce(),
        PinSpec::new(
            "attachments_out",
            "Attachments",
            "Attachments emitted by the remote chat update",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<Attachment>())
        .enforce(),
        PinSpec::new(
            "actions_out",
            "Actions",
            "Actions emitted by the remote chat update",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<ChatAction>())
        .enforce(),
        PinSpec::new(
            "plan",
            "Plan",
            "Latest streamed reasoning plan",
            VariableType::Struct,
        )
        .schema(schema_string::<Reasoning>())
        .enforce(),
        PinSpec::new(
            "local_session_out",
            "Local State",
            "Latest remote local session state",
            VariableType::Struct,
        ),
        PinSpec::new(
            "global_session_out",
            "Global State",
            "Latest remote global session state",
            VariableType::Struct,
        ),
        PinSpec::new(
            "usage_stat",
            "Usage Stat",
            "Latest model usage update",
            VariableType::Struct,
        )
        .schema(schema_string::<ChatUsageStat>())
        .enforce(),
        PinSpec::new(
            "model_id",
            "Model ID",
            "Model reported by the remote chat",
            VariableType::String,
        ),
        PinSpec::new(
            "event_type",
            "Event Type",
            "Type of the latest streamed remote event",
            VariableType::String,
        ),
        PinSpec::new(
            "event_payload",
            "Event Payload",
            "Raw payload of the latest streamed remote event",
            VariableType::Generic,
        ),
        PinSpec::new("run_id", "Run ID", "Remote run id", VariableType::String),
        PinSpec::new("status", "Status", "Final run status", VariableType::String),
    ];

    (inputs, outputs)
}

fn rest_desired(node: &Node, meta: &EventMeta) -> (Vec<PinSpec>, Vec<PinSpec>) {
    let mut route_values: Vec<String> = meta
        .rest_routes
        .iter()
        .map(|route| route_label(&route.method, &route.path))
        .collect();
    route_values.extend(
        meta.rest_files
            .iter()
            .map(|file| route_label("GET", &file.path)),
    );

    let mut inputs = vec![
        PinSpec::new(
            "route",
            "Route",
            "Route of the remote API to call",
            VariableType::String,
        )
        .dropdown(route_values),
        PinSpec::new(
            "query",
            "Query",
            "Query parameters as an object",
            VariableType::Generic,
        )
        .default(json!({})),
        PinSpec::new("body", "Body", "Request body (JSON)", VariableType::Generic)
            .default(Value::Null),
        PinSpec::new(
            "headers",
            "Headers",
            "Additional request headers as an object",
            VariableType::Generic,
        )
        .default(json!({})),
        timeout_spec(),
    ];

    let selection = pin_string(node, "route");
    if let Some((_method, path)) = selected_route(meta, &selection) {
        for param in &template_params(&path) {
            inputs.push(PinSpec::new(
                &format!("param_{}", param),
                param,
                "Path parameter",
                VariableType::String,
            ));
        }
    }

    let outputs = vec![
        PinSpec::new(
            "status",
            "Status Code",
            "HTTP status code of the response",
            VariableType::Integer,
        ),
        PinSpec::new(
            "response_headers",
            "Response Headers",
            "Response headers as an object",
            VariableType::Generic,
        ),
        PinSpec::new(
            "response",
            "Response",
            "Response body (JSON when parseable, else text)",
            VariableType::Generic,
        ),
        PinSpec::new(
            "file",
            "File",
            "Response body as a downloaded file when it is binary",
            VariableType::Struct,
        )
        .schema(flow_path_schema()),
    ];
    (inputs, outputs)
}

fn mcp_desired(node: &Node, meta: &EventMeta) -> (Vec<PinSpec>, Vec<PinSpec>) {
    let mut inputs = vec![
        PinSpec::new(
            "mode",
            "Mode",
            "Whether to call a tool or read a resource",
            VariableType::String,
        )
        .dropdown(vec![
            MCP_MODE_CALL_TOOL.to_string(),
            MCP_MODE_READ_RESOURCE.to_string(),
        ])
        .default(json!(MCP_MODE_CALL_TOOL)),
        PinSpec::new(
            "headers",
            "Auth Headers",
            "Static registration authentication headers (for example Authorization or x-api-key). HMAC auth is not supported for MCP because every request needs a fresh signature.",
            VariableType::Struct,
        )
        .schema(schema_string::<HashMap<String, String>>())
        .default(json!({})),
        timeout_spec(),
    ];

    let mode = {
        let value = pin_string(node, "mode");
        if value.is_empty() {
            MCP_MODE_CALL_TOOL.to_string()
        } else {
            value
        }
    };

    let mut outputs = Vec::new();

    if mode == MCP_MODE_READ_RESOURCE {
        inputs.push(
            PinSpec::new(
                "resource",
                "Resource",
                "Resource to read",
                VariableType::String,
            )
            .dropdown(meta.mcp_resources.iter().map(|r| r.uri.clone()).collect()),
        );
        outputs.push(
            PinSpec::new(
                "file",
                "File",
                "Resource contents as a file",
                VariableType::Struct,
            )
            .schema(flow_path_schema()),
        );
        outputs.push(PinSpec::new(
            "text",
            "Text",
            "Resource contents when textual",
            VariableType::String,
        ));
    } else {
        inputs.push(
            PinSpec::new("tool", "Tool", "Tool to call", VariableType::String)
                .dropdown(meta.mcp_tools.iter().map(|t| t.name.clone()).collect()),
        );

        let selected_tool = pin_string(node, "tool");
        if let Some(tool) = meta.mcp_tools.iter().find(|t| t.name == selected_tool)
            && let Some(properties) = tool
                .input_schema
                .as_ref()
                .and_then(|schema| schema.get("properties"))
                .and_then(|props| props.as_object())
        {
            for (name, prop_schema) in properties {
                let description = prop_schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("Tool argument");
                inputs.push(PinSpec::new(
                    &format!("arg_{}", name),
                    name,
                    description,
                    json_schema_type(prop_schema),
                ));
            }
        }

        outputs.push(PinSpec::new(
            "result",
            "Result",
            "Tool call result",
            VariableType::Generic,
        ));
        outputs.push(PinSpec::new(
            "result_text",
            "Result Text",
            "Text content of the tool result",
            VariableType::String,
        ));
    }

    (inputs, outputs)
}

fn fallback_desired() -> (Vec<PinSpec>, Vec<PinSpec>) {
    let inputs = vec![
        PinSpec::new(
            "payload",
            "Payload",
            "Input payload passed to the remote event",
            VariableType::Generic,
        ),
        PinSpec::new(
            "wait_for_result",
            "Wait For Result",
            "Wait for the remote run to finish and return its result",
            VariableType::Boolean,
        )
        .default(json!(true)),
        timeout_spec(),
    ];
    let outputs = vec![
        PinSpec::new("run_id", "Run ID", "Remote run id", VariableType::String),
        PinSpec::new("status", "Status", "Final run status", VariableType::String),
        PinSpec::new(
            "result",
            "Result",
            "Result payload of the remote run",
            VariableType::Generic,
        ),
    ];
    (inputs, outputs)
}

fn template_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .map(|s| s.to_string())
        })
        .collect()
}

fn schema_string<T: schemars::JsonSchema>() -> String {
    let schema = schemars::schema_for!(T);
    flow_like_types::json::to_value(&schema)
        .ok()
        .and_then(|value| flow_like_types::json::to_string(&value).ok())
        .unwrap_or_default()
}

fn flow_path_schema() -> String {
    schema_string::<FlowPath>()
}

fn add_remote_event_selector_pins(node: &mut Node, event_description: &str) {
    node.add_input_pin(
        PIN_REMOTE_APP_ID,
        "Project",
        "Connected project to invoke the event in",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        PIN_REMOTE_EVENT,
        "Event",
        event_description,
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        PIN_REMOTE_EVENT_META,
        "Event Details",
        "Auto-filled by the editor when an event is selected. Drives the typed pins.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

#[crate::register_node]
#[derive(Default)]
pub struct CallRemoteEventNode {}

impl CallRemoteEventNode {
    pub fn new() -> Self {
        CallRemoteEventNode {}
    }
}

#[async_trait]
impl NodeLogic for CallRemoteEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "call_remote_event",
            "Call Remote Event",
            "Invoke a chat, API or MCP event of a connected project. Pins adapt to the selected event. The project must have granted this app a role that allows executing events.",
            "Events/Remote",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_version(4);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        add_remote_event_selector_pins(&mut node, "Event of the selected project to invoke");

        node.add_output_pin(
            "exec_out",
            "Done",
            "The remote event was invoked",
            VariableType::Execution,
        );

        // Start with the generic fallback pins; on_update refines them.
        let (inputs, outputs) = fallback_desired();
        reconcile_inputs(&mut node, &inputs);
        reconcile_outputs(&mut node, &outputs);

        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let (inputs, outputs) = match parse_meta(node) {
            Some(meta) => match meta.event_type.as_str() {
                "simple_chat" => chat_desired(node),
                "rest" => rest_desired(node, &meta),
                "mcp" => mcp_desired(node, &meta),
                _ => fallback_desired(),
            },
            None => fallback_desired(),
        };
        reconcile_inputs(node, &inputs);
        reconcile_outputs(node, &outputs);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let remote_app_id: String = context.evaluate_pin(PIN_REMOTE_APP_ID).await?;
        let event_id: String = context.evaluate_pin(PIN_REMOTE_EVENT).await?;
        let remote_app_id = validate_path_id(&remote_app_id, "remote project")?;
        let event_id = validate_path_id(&event_id, "remote event")?;

        let meta_raw: String = context
            .evaluate_pin(PIN_REMOTE_EVENT_META)
            .await
            .unwrap_or_default();
        let meta: EventMeta = flow_like_types::json::from_str(&meta_raw).unwrap_or_default();

        let session = remote_app_session(context, &remote_app_id).await?;

        match meta.event_type.as_str() {
            "simple_chat" => self.run_chat(context, &session, &event_id).await,
            "rest" => self.run_rest(context, &session, &event_id, &meta).await,
            "mcp" => self.run_mcp(context, &session, &event_id, &meta).await,
            _ => self.run_generic(context, &session, &event_id).await,
        }
    }
}

impl CallRemoteEventNode {
    async fn timeout_secs(&self, context: &mut ExecutionContext) -> u64 {
        let seconds: i64 = context.evaluate_pin("timeout_seconds").await.unwrap_or(120);
        seconds.clamp(1, 600) as u64
    }

    async fn run_generic(
        &self,
        context: &mut ExecutionContext,
        session: &RemoteAppSession,
        event_id: &str,
    ) -> flow_like_types::Result<()> {
        let payload: Value = context.evaluate_pin("payload").await.unwrap_or(Value::Null);
        let wait_for_result: bool = context
            .evaluate_pin("wait_for_result")
            .await
            .unwrap_or(true);
        let timeout = self.timeout_secs(context).await;
        let body = json!({ "payload": payload });

        if !wait_for_result {
            let url = session.url(&format!("events/{}/invoke/async", event_id));
            let queued: Value = flow_like_types::tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                async {
                    let response = post_json(session, &url, &body).await?;
                    let queued: Value = response.json().await?;
                    Ok::<_, flow_like_types::Error>(queued)
                },
            )
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "Remote event request did not finish within {} seconds",
                    timeout
                )
            })??;
            context
                .set_pin_value(
                    "run_id",
                    queued.get("run_id").cloned().unwrap_or(Value::Null),
                )
                .await?;
            context
                .set_pin_value(
                    "status",
                    queued.get("status").cloned().unwrap_or(json!("pending")),
                )
                .await?;
            context.set_pin_value("result", Value::Null).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let url = session.url(&format!("events/{}/invoke", event_id));
        let outcome = invoke_and_collect(session, &url, &body, timeout).await?;
        outcome.ensure_ok()?;

        context
            .set_pin_value("run_id", json!(outcome.run_id.clone().unwrap_or_default()))
            .await?;
        context
            .set_pin_value("status", json!(outcome.status_str()))
            .await?;
        context
            .set_pin_value(
                "result",
                outcome.generic_result.clone().unwrap_or(Value::Null),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn run_chat(
        &self,
        context: &mut ExecutionContext,
        session: &RemoteAppSession,
        event_id: &str,
    ) -> flow_like_types::Result<()> {
        let message: String = context.evaluate_pin("message").await.unwrap_or_default();
        let history: Value = context.evaluate_pin("history").await.unwrap_or(Value::Null);
        let local_session: Value = context
            .evaluate_pin("local_session")
            .await
            .unwrap_or(Value::Null);
        let global_session: Value = context
            .evaluate_pin("global_session")
            .await
            .unwrap_or(Value::Null);
        let tools: Vec<String> = context.evaluate_pin("tools").await.unwrap_or_default();
        let attachments: Value = context
            .evaluate_pin("attachments")
            .await
            .unwrap_or(Value::Null);
        let timeout = self.timeout_secs(context).await;

        let mut messages: Vec<Value> = match &history {
            Value::Array(items) => items.clone(),
            Value::Object(obj) => {
                if let Some(items) = obj.get("messages").and_then(|m| m.as_array()) {
                    items.clone()
                } else if obj.contains_key("role") {
                    vec![history.clone()]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        if !message.trim().is_empty() {
            messages.push(json!({ "role": "user", "content": message }));
        }

        let mut chat = json!({ "messages": messages });
        if !local_session.is_null() {
            chat["local_session"] = local_session;
        }
        if !global_session.is_null() {
            chat["global_session"] = global_session;
        }
        if !tools.is_empty() {
            chat["tools"] = json!(tools);
        }
        if !attachments.is_null() {
            chat["attachments"] = attachments;
        }

        let url = session.url(&format!("events/{}/invoke", event_id));
        let outcome =
            invoke_and_collect(session, &url, &json!({ "payload": chat }), timeout).await?;
        outcome.ensure_ok()?;

        let response = outcome.chat_result().unwrap_or(Value::Null);
        let response_text = response
            .get("response")
            .map(extract_response_text)
            .unwrap_or_else(|| extract_response_text(&response));

        context.set_pin_value("response", response.clone()).await?;
        context
            .set_pin_value("response_text", json!(response_text))
            .await?;
        context
            .set_pin_value("run_id", json!(outcome.run_id.clone().unwrap_or_default()))
            .await?;
        context
            .set_pin_value("status", json!(outcome.status_str()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn run_rest(
        &self,
        context: &mut ExecutionContext,
        session: &RemoteAppSession,
        event_id: &str,
        meta: &EventMeta,
    ) -> flow_like_types::Result<()> {
        let selection: String = context.evaluate_pin("route").await.unwrap_or_default();
        let (method, mut path) = selected_route(meta, &selection)
            .ok_or_else(|| flow_like_types::anyhow!("No route selected"))?;
        let is_file = is_file_route(meta, &method, &path);

        for param in template_params(&path) {
            let value: String = context
                .evaluate_pin(&format!("param_{}", param))
                .await
                .unwrap_or_default();
            let value = encode_path_parameter(&value)?;
            path = path.replace(&format!("{{{}}}", param), &value);
        }

        let query: Value = context.evaluate_pin("query").await.unwrap_or(Value::Null);
        let body: Value = context.evaluate_pin("body").await.unwrap_or(Value::Null);
        let headers: Value = context.evaluate_pin("headers").await.unwrap_or(Value::Null);
        let timeout = self.timeout_secs(context).await;

        let url = session.url(&format!(
            "events/{}/rest{}",
            event_id,
            ensure_leading_slash(&path)
        ));
        let http_method = flow_like_types::reqwest::Method::from_bytes(method.as_bytes())
            .unwrap_or(flow_like_types::reqwest::Method::GET);
        let mut request = http_client_no_redirect()
            .request(http_method, &url)
            .bearer_auth(&session.token);

        if let Some(query_obj) = query.as_object() {
            let pairs: Vec<(String, String)> = query_obj
                .iter()
                .map(|(k, v)| (k.clone(), value_to_query(v)))
                .collect();
            request = request.query(&pairs);
        }
        request = with_event_registration_headers(request, &headers);
        if !body.is_null() {
            request = request.json(&body);
        }

        let (status, header_map, content_type, bytes) =
            flow_like_types::tokio::time::timeout(std::time::Duration::from_secs(timeout), async {
                let mut response = request
                    .send()
                    .await
                    .map_err(|err| flow_like_types::anyhow!("Remote REST call failed: {}", err))?;

                // Static file registrations redirect to a short-lived object-store URL.
                // Follow it with a new request that carries neither the app token nor
                // registration credentials. Custom headers are otherwise retained by
                // reqwest across cross-origin redirects.
                if is_file && response.status().is_redirection() {
                    response = follow_get_redirect_without_credentials(response).await?;
                }

                // Non-file redirects are returned to the flow as status + Location
                // rather than followed with credentials or treated as API failures.
                let response = if response.status().is_redirection() {
                    response
                } else {
                    error_for_status(response, "Remote REST call").await?
                };

                let status = response.status().as_u16() as i64;
                let header_map: flow_like_types::json::Map<String, Value> = response
                    .headers()
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.to_string(), json!(value)))
                    })
                    .collect();
                let content_type = response
                    .headers()
                    .get(flow_like_types::reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let bytes = response.bytes().await.map_err(|err| {
                    flow_like_types::anyhow!("Failed to read REST response: {}", err)
                })?;
                Ok::<_, flow_like_types::Error>((status, header_map, content_type, bytes))
            })
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "Remote REST call did not finish within {} seconds",
                    timeout
                )
            })??;

        context.set_pin_value("status", json!(status)).await?;
        context
            .set_pin_value("response_headers", json!(header_map))
            .await?;

        if is_file || is_binary_content(&content_type) {
            let file =
                write_to_cache_file(context, &file_name(&path, &content_type), bytes.to_vec())
                    .await?;
            context.set_pin_value("file", json!(file)).await?;
            context.set_pin_value("response", Value::Null).await?;
        } else {
            let value = flow_like_types::json::from_slice::<Value>(&bytes)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
            context.set_pin_value("response", value).await?;
            context.set_pin_value("file", Value::Null).await?;
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn run_mcp(
        &self,
        context: &mut ExecutionContext,
        session: &RemoteAppSession,
        event_id: &str,
        meta: &EventMeta,
    ) -> flow_like_types::Result<()> {
        let mode: String = context.evaluate_pin("mode").await.unwrap_or_default();
        let headers: Value = context.evaluate_pin("headers").await.unwrap_or(Value::Null);
        let timeout = self.timeout_secs(context).await;

        if mode == MCP_MODE_READ_RESOURCE {
            let uri: String = context.evaluate_pin("resource").await.unwrap_or_default();
            if uri.trim().is_empty() {
                return Err(flow_like_types::anyhow!("No resource selected"));
            }
            let result = flow_like_types::tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                session.mcp_request(event_id, "resources/read", json!({ "uri": uri }), &headers),
            )
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "Remote MCP request did not finish within {} seconds",
                    timeout
                )
            })??;
            let contents = result
                .get("contents")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .cloned()
                .unwrap_or(Value::Null);

            let mime = meta
                .mcp_resources
                .iter()
                .find(|r| r.uri == uri)
                .and_then(|r| r.mime_type.clone())
                .or_else(|| {
                    contents
                        .get("mimeType")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                });

            let (bytes, text) = if let Some(text) = contents.get("text").and_then(|t| t.as_str()) {
                (text.as_bytes().to_vec(), Some(text.to_string()))
            } else if let Some(blob) = contents.get("blob").and_then(|b| b.as_str()) {
                let decoded = BASE64_STANDARD
                    .decode(blob)
                    .map_err(|err| flow_like_types::anyhow!("Invalid resource blob: {}", err))?;
                (decoded, None)
            } else {
                (Vec::new(), None)
            };

            let file =
                write_to_cache_file(context, &resource_file_name(&uri, mime.as_deref()), bytes)
                    .await?;
            context.set_pin_value("file", json!(file)).await?;
            context
                .set_pin_value("text", json!(text.unwrap_or_default()))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let tool: String = context.evaluate_pin("tool").await.unwrap_or_default();
        if tool.trim().is_empty() {
            return Err(flow_like_types::anyhow!("No tool selected"));
        }

        let mut arguments = flow_like_types::json::Map::new();
        if let Some(tool_meta) = meta.mcp_tools.iter().find(|t| t.name == tool)
            && let Some(properties) = tool_meta
                .input_schema
                .as_ref()
                .and_then(|schema| schema.get("properties"))
                .and_then(|props| props.as_object())
        {
            for name in properties.keys() {
                let value: Value = context
                    .evaluate_pin(&format!("arg_{}", name))
                    .await
                    .unwrap_or(Value::Null);
                if !value.is_null() {
                    arguments.insert(name.clone(), value);
                }
            }
        }

        let result = flow_like_types::tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            session.mcp_request(
                event_id,
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
                &headers,
            ),
        )
        .await
        .map_err(|_| {
            flow_like_types::anyhow!(
                "Remote MCP request did not finish within {} seconds",
                timeout
            )
        })??;

        if result
            .get("isError")
            .and_then(|e| e.as_bool())
            .unwrap_or(false)
        {
            return Err(flow_like_types::anyhow!(
                "Remote tool '{}' returned an error: {}",
                tool,
                extract_text(&result)
            ));
        }

        let result_text = extract_mcp_text(&result);
        context.set_pin_value("result", result.clone()).await?;
        context
            .set_pin_value("result_text", json!(result_text))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dedicated remote API node
// ---------------------------------------------------------------------------

#[crate::register_node]
#[derive(Default)]
pub struct CallRemoteApiNode {}

impl CallRemoteApiNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CallRemoteApiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "call_remote_api",
            "Call Remote API",
            "Call an internal REST API exposed by a connected project and return its status, headers and response body.",
            "Events/Remote",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        add_remote_event_selector_pins(&mut node, "REST API event of the selected project");
        node.add_output_pin(
            "exec_out",
            "Done",
            "The remote API request completed",
            VariableType::Execution,
        );

        let (inputs, outputs) = rest_desired(&node, &EventMeta::default());
        reconcile_inputs(&mut node, &inputs);
        reconcile_outputs(&mut node, &outputs);
        node.set_long_running(true);
        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let meta = parse_meta(node).unwrap_or_default();
        let (inputs, outputs) = rest_desired(node, &meta);
        reconcile_inputs(node, &inputs);
        reconcile_outputs(node, &outputs);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let remote_app_id: String = context.evaluate_pin(PIN_REMOTE_APP_ID).await?;
        let event_id: String = context.evaluate_pin(PIN_REMOTE_EVENT).await?;
        let remote_app_id = validate_path_id(&remote_app_id, "remote project")?;
        let event_id = validate_path_id(&event_id, "remote API event")?;
        let meta_raw: String = context
            .evaluate_pin(PIN_REMOTE_EVENT_META)
            .await
            .unwrap_or_default();
        let meta: EventMeta = flow_like_types::json::from_str(&meta_raw).map_err(|_| {
            flow_like_types::anyhow!(
                "Remote API details are missing; select the remote API event again"
            )
        })?;
        if meta.event_type != "rest" {
            return Err(flow_like_types::anyhow!(
                "The selected remote event is type '{}', but Call Remote API requires a REST event",
                meta.event_type
            ));
        }

        let session = remote_app_session(context, &remote_app_id).await?;
        CallRemoteEventNode::new()
            .run_rest(context, &session, &event_id, &meta)
            .await
    }
}

// ---------------------------------------------------------------------------
// Dedicated remote chat node
// ---------------------------------------------------------------------------

#[crate::register_node]
#[derive(Default)]
pub struct CallRemoteChatNode {}

impl CallRemoteChatNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CallRemoteChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "call_remote_chat",
            "Call Remote Chat",
            "Call a chat event in a connected project. Chunks, complete responses, widgets, attachments and session state are exposed while the remote chat streams.",
            "Events/Remote",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        add_remote_event_selector_pins(&mut node, "Chat event of the selected project");
        node.add_output_pin(
            "on_stream",
            "On Stream",
            "Fires for every output event produced by the remote chat",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Done",
            "The remote chat completed",
            VariableType::Execution,
        );

        let (inputs, outputs) = remote_chat_desired();
        reconcile_inputs(&mut node, &inputs);
        reconcile_outputs(&mut node, &outputs);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("on_stream").await?;

        let remote_app_id: String = context.evaluate_pin(PIN_REMOTE_APP_ID).await?;
        let event_id: String = context.evaluate_pin(PIN_REMOTE_EVENT).await?;
        let remote_app_id = validate_path_id(&remote_app_id, "remote project")?;
        let event_id = validate_path_id(&event_id, "remote chat event")?;
        let meta_raw: String = context
            .evaluate_pin(PIN_REMOTE_EVENT_META)
            .await
            .unwrap_or_default();
        let meta: EventMeta = flow_like_types::json::from_str(&meta_raw).map_err(|_| {
            flow_like_types::anyhow!(
                "Remote chat details are missing; select the remote chat event again"
            )
        })?;
        if meta.event_type != "simple_chat" {
            return Err(flow_like_types::anyhow!(
                "The selected remote event is type '{}', but Call Remote Chat requires a chat event",
                meta.event_type
            ));
        }

        reset_remote_chat_outputs(context).await?;

        let message: String = context.evaluate_pin("message").await.unwrap_or_default();
        let history: Value = context.evaluate_pin("history").await.unwrap_or(Value::Null);
        let local_session: Value = context
            .evaluate_pin("local_session")
            .await
            .unwrap_or(Value::Null);
        let global_session: Value = context
            .evaluate_pin("global_session")
            .await
            .unwrap_or(Value::Null);
        let tools: Vec<String> = context.evaluate_pin("tools").await.unwrap_or_default();
        let actions: Value = context.evaluate_pin("actions").await.unwrap_or(Value::Null);
        let attachments: Value = context
            .evaluate_pin("attachments")
            .await
            .unwrap_or(Value::Null);
        let user: Value = context.evaluate_pin("user").await.unwrap_or(Value::Null);
        let timeout = CallRemoteEventNode::new().timeout_secs(context).await;

        let mut messages: Vec<Value> = match &history {
            Value::Array(items) => items.clone(),
            Value::Object(obj) => obj
                .get("messages")
                .and_then(|messages| messages.as_array())
                .cloned()
                .or_else(|| obj.contains_key("role").then(|| vec![history.clone()]))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !message.trim().is_empty() {
            messages.push(json!({ "role": "user", "content": message }));
        }

        let mut chat = json!({ "messages": messages });
        insert_non_null(&mut chat, "local_session", local_session);
        insert_non_null(&mut chat, "global_session", global_session);
        if !tools.is_empty() {
            chat["tools"] = json!(tools);
        }
        insert_non_empty_array(&mut chat, "actions", actions);
        insert_non_empty_array(&mut chat, "attachments", attachments);
        insert_non_null(&mut chat, "user", user);

        let session = remote_app_session(context, &remote_app_id).await?;
        let url = session.url(&format!("events/{}/invoke", event_id));
        let body = json!({ "payload": chat });
        let stream_state = RemoteChatStreamState::new(context).await?;
        let mut handler = RemoteChatEventOutput::new(context, stream_state);
        let outcome_result =
            invoke_and_collect_with_handler(&session, &url, &body, timeout, &mut handler).await;
        let finalize_result = handler.finalize().await;
        drop(handler);

        if let Err(error) = finalize_result {
            if outcome_result.is_ok() {
                return Err(error);
            }
            context.log_message(
                &format!("Failed to finalize remote chat stream outputs: {error}"),
                LogLevel::Warn,
            );
        }

        let outcome = outcome_result?;
        outcome.ensure_ok()?;
        context
            .set_pin_value("run_id", json!(outcome.run_id.clone().unwrap_or_default()))
            .await?;
        context
            .set_pin_value("status", json!(outcome.status_str()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

struct RemoteChatStreamConsumer {
    node_id: String,
    context: ExecutionContext,
}

struct RemoteChatStreamState {
    parent_node_id: String,
    consumers: Vec<RemoteChatStreamConsumer>,
}

impl RemoteChatStreamState {
    async fn new(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        let on_stream = context.get_pin_by_name("on_stream").await?;
        context.activate_exec_pin_ref(&on_stream).await?;
        let parent_node_id = context.node.node.lock().await.id.clone();
        let mut consumers = Vec::new();
        for node in on_stream.get_connected_nodes() {
            let node_id = node.node.lock().await.id.clone();
            consumers.push(RemoteChatStreamConsumer {
                node_id,
                context: context.create_sub_context(&node).await,
            });
        }
        Ok(Self {
            parent_node_id,
            consumers,
        })
    }

    async fn emit(&mut self, context: &mut ExecutionContext) {
        let mut recursion_guard = AHashSet::new();
        recursion_guard.insert(self.parent_node_id.clone());
        for consumer in &mut self.consumers {
            let mut guard = Some(recursion_guard.clone());
            let result = InternalNode::trigger(&mut consumer.context, &mut guard, true).await;
            consumer.context.end_trace();
            if let Err(error) = result {
                context.log_message(
                    &format!(
                        "Remote chat stream-connected node {} failed: {error:?}",
                        consumer.node_id
                    ),
                    LogLevel::Error,
                );
            }
        }
    }

    async fn finalize(&mut self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_stream").await?;
        for consumer in &mut self.consumers {
            consumer.context.end_trace();
            context.push_sub_context(&mut consumer.context);
        }
        Ok(())
    }
}

struct RemoteChatEventOutput<'a> {
    context: &'a mut ExecutionContext,
    stream: RemoteChatStreamState,
}

impl<'a> RemoteChatEventOutput<'a> {
    fn new(context: &'a mut ExecutionContext, stream: RemoteChatStreamState) -> Self {
        Self { context, stream }
    }

    async fn finalize(&mut self) -> flow_like_types::Result<()> {
        self.stream.finalize(self.context).await
    }
}

#[async_trait]
impl RemoteSseEventHandler for RemoteChatEventOutput<'_> {
    async fn on_event(&mut self, event: &RemoteSseEvent) -> flow_like_types::Result<()> {
        if let Some(run_id) = &event.run_id {
            self.context.set_pin_value("run_id", json!(run_id)).await?;
        }
        if !is_stream_output_event(&event.event_type) {
            return Ok(());
        }

        for (pin, value) in remote_chat_pin_updates(event) {
            self.context.set_pin_value(pin, value).await?;
        }
        self.stream.emit(self.context).await;
        Ok(())
    }
}

fn is_stream_output_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "chat_stream_partial"
            | "chat_stream"
            | "chat_out"
            | "chat_local_session"
            | "chat_global_session"
            | "chat_usage_stat"
            | "a2ui"
            | "interaction_request"
    )
}

fn remote_chat_pin_updates(event: &RemoteSseEvent) -> Vec<(&'static str, Value)> {
    let mut updates = vec![
        ("event_type", json!(event.event_type)),
        ("event_payload", event.payload.clone()),
    ];
    if let Some(run_id) = &event.run_id {
        updates.push(("run_id", json!(run_id)));
    }

    match event.event_type.as_str() {
        "chat_stream_partial" => {
            push_payload_field(&mut updates, &event.payload, "chunk", "chunk");
            push_payload_field(&mut updates, &event.payload, "widgets", "widgets");
            push_payload_field(
                &mut updates,
                &event.payload,
                "attachments",
                "attachments_out",
            );
            push_payload_field(&mut updates, &event.payload, "actions", "actions_out");
            push_payload_field(&mut updates, &event.payload, "plan", "plan");
        }
        "chat_stream" | "chat_out" => {
            if let Some(response) = event.payload.get("response") {
                updates.push(("response", response.clone()));
                updates.push(("response_text", json!(extract_response_text(response))));
            }
            push_payload_field(&mut updates, &event.payload, "widgets", "widgets");
            push_payload_field(
                &mut updates,
                &event.payload,
                "attachments",
                "attachments_out",
            );
            push_payload_field(&mut updates, &event.payload, "actions", "actions_out");
            push_payload_field(&mut updates, &event.payload, "model_id", "model_id");
        }
        "chat_local_session" => updates.push(("local_session_out", event.payload.clone())),
        "chat_global_session" => updates.push(("global_session_out", event.payload.clone())),
        "chat_usage_stat" => updates.push(("usage_stat", event.payload.clone())),
        _ => {}
    }

    updates
}

fn push_payload_field(
    updates: &mut Vec<(&'static str, Value)>,
    payload: &Value,
    field: &str,
    pin: &'static str,
) {
    if let Some(value) = payload.get(field) {
        updates.push((pin, value.clone()));
    }
}

fn insert_non_null(target: &mut Value, field: &str, value: Value) {
    if !value.is_null() {
        target[field] = value;
    }
}

fn insert_non_empty_array(target: &mut Value, field: &str, value: Value) {
    if value.as_array().is_some_and(|values| !values.is_empty()) {
        target[field] = value;
    }
}

async fn reset_remote_chat_outputs(context: &mut ExecutionContext) -> flow_like_types::Result<()> {
    for pin in ["widgets", "attachments_out", "actions_out"] {
        context.set_pin_value(pin, json!([])).await?;
    }
    for pin in [
        "chunk",
        "response",
        "plan",
        "local_session_out",
        "global_session_out",
        "usage_stat",
        "event_payload",
    ] {
        context.set_pin_value(pin, Value::Null).await?;
    }
    for pin in [
        "response_text",
        "model_id",
        "event_type",
        "run_id",
        "status",
    ] {
        context.set_pin_value(pin, json!("")).await?;
    }
    Ok(())
}

fn extract_response_text(response: &Value) -> String {
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| {
            choices.iter().rev().find_map(|choice| {
                choice
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_str)
            })
        })
        .map(str::to_string)
        .unwrap_or_else(|| extract_text(response))
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

fn ensure_leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

fn encode_path_parameter(value: &str) -> flow_like_types::Result<String> {
    // URL parsers normalize literal and percent-encoded dot-only segments
    // before sending the request. Reject them explicitly so a dynamic route
    // value can never escape the trusted event REST proxy path.
    if value == "." || value == ".." {
        return Err(flow_like_types::anyhow!(
            "Remote REST path parameters cannot be '.' or '..'"
        ));
    }
    Ok(urlencoding::encode(value).into_owned())
}

fn value_to_query(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_binary_content(content_type: &str) -> bool {
    let ct = content_type.split(';').next().unwrap_or("").trim();
    !(ct.is_empty()
        || ct.starts_with("text/")
        || ct.contains("json")
        || ct.contains("xml")
        || ct.contains("yaml")
        || ct.contains("javascript")
        || ct.contains("csv"))
}

fn file_name(path: &str, content_type: &str) -> String {
    let base: String = path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("download")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if base.contains('.') {
        return base;
    }
    let ext = extension_for(content_type);
    format!("{}{}", base, ext)
}

fn resource_file_name(uri: &str, mime: Option<&str>) -> String {
    let base = uri
        .rsplit(['/', ':'])
        .find(|segment| !segment.is_empty())
        .unwrap_or("resource")
        .to_string();
    let base: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if base.contains('.') {
        return base;
    }
    format!("{}{}", base, extension_for(mime.unwrap_or("")))
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "application/json" => ".json",
        "application/pdf" => ".pdf",
        "text/plain" => ".txt",
        "text/html" => ".html",
        "text/csv" => ".csv",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/svg+xml" => ".svg",
        _ => ".bin",
    }
}

async fn write_to_cache_file(
    context: &mut ExecutionContext,
    file_name: &str,
    bytes: Vec<u8>,
) -> flow_like_types::Result<FlowPath> {
    let dir = FlowPath::from_cache_dir(context, true, false).await?;
    let path = format!(
        "{}/{}-{}",
        dir.path.trim_end_matches('/'),
        flow_like_types::create_id(),
        file_name
    );
    let file = FlowPath::new(path, dir.store_ref.clone(), dir.cache_store_ref.clone());
    file.put(context, bytes, false).await?;
    Ok(file)
}

/// Best-effort text extraction from an arbitrary response value.
fn extract_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    for key in ["response_text", "text", "content", "message", "response"] {
        if let Some(inner) = value.get(key) {
            if let Some(text) = inner.as_str() {
                return text.to_string();
            }
            let nested = extract_text(inner);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    String::new()
}

/// Concatenates the text parts of an MCP tool result (`content: [{type:"text", text}]`).
fn extract_mcp_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use flow_like::flow::{node::NodeLogic, pin::PinType};
    use flow_like_types::json::json;

    use super::{
        CallRemoteApiNode, CallRemoteChatNode, RemoteSseEvent, encode_path_parameter,
        is_stream_output_event, remote_chat_pin_updates,
    };

    #[test]
    fn remote_rest_path_parameters_stay_in_one_segment() {
        assert_eq!(
            encode_path_parameter("folder/name?x=1%done").unwrap(),
            "folder%2Fname%3Fx%3D1%25done"
        );
        assert_eq!(
            encode_path_parameter(" already-safe ").unwrap(),
            "%20already-safe%20"
        );
    }

    #[test]
    fn remote_rest_path_parameters_reject_dot_segments() {
        assert!(encode_path_parameter(".").is_err());
        assert_eq!(encode_path_parameter(" .. ").unwrap(), "%20..%20");
        assert_eq!(encode_path_parameter("...").unwrap(), "...");
        assert_eq!(encode_path_parameter("%2e%2e").unwrap(), "%252e%252e");
    }

    #[test]
    fn dedicated_remote_nodes_expose_stable_output_contracts() {
        let api = CallRemoteApiNode::new().get_node();
        assert_eq!(api.name, "call_remote_api");
        assert!(api.pins.values().any(|pin| {
            pin.pin_type == PinType::Output && pin.name == "file" && pin.schema.is_some()
        }));

        let chat = CallRemoteChatNode::new().get_node();
        assert_eq!(chat.name, "call_remote_chat");
        for name in [
            "on_stream",
            "exec_out",
            "chunk",
            "response",
            "widgets",
            "attachments_out",
            "local_session_out",
            "global_session_out",
            "event_payload",
        ] {
            assert!(
                chat.pins
                    .values()
                    .any(|pin| pin.pin_type == PinType::Output && pin.name == name),
                "missing remote chat output {name}"
            );
        }

        let names = chat
            .pins
            .values()
            .map(|pin| pin.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names.len(),
            names.iter().copied().collect::<HashSet<_>>().len(),
            "pin names must be unambiguous across inputs and outputs"
        );
        for pin in chat.pins.values().filter(|pin| {
            pin.pin_type == PinType::Input
                && pin.data_type != flow_like::flow::variable::VariableType::Execution
        }) {
            assert!(
                pin.default_value.is_some(),
                "optional chat input {} needs a default so the node can execute unwired",
                pin.name
            );
        }
    }

    #[test]
    fn remote_chat_updates_surface_typed_partial_and_response_fields() {
        let partial = RemoteSseEvent {
            event_type: "chat_stream_partial".to_string(),
            payload: json!({
                "chunk": { "id": "chunk-1", "choices": [] },
                "widgets": [{ "instance_id": "one" }],
                "attachments": ["https://example.com/file"],
                "actions": [],
                "plan": { "current_step": 0 }
            }),
            run_id: Some("run-1".to_string()),
        };
        let updates = remote_chat_pin_updates(&partial)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(updates["chunk"]["id"], "chunk-1");
        assert_eq!(updates["widgets"][0]["instance_id"], "one");
        assert_eq!(updates["attachments_out"][0], "https://example.com/file");
        assert_eq!(updates["run_id"], "run-1");

        let response = RemoteSseEvent {
            event_type: "chat_out".to_string(),
            payload: json!({
                "response": {
                    "choices": [{ "message": { "content": "final answer" } }]
                },
                "local_session": {},
                "global_session": {}
            }),
            run_id: None,
        };
        let updates = remote_chat_pin_updates(&response)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(updates["response_text"], "final answer");
        assert!(!updates.contains_key("local_session_out"));
        assert!(!updates.contains_key("global_session_out"));
    }

    #[test]
    fn dedicated_session_events_are_not_lost_or_overwritten() {
        let local = RemoteSseEvent {
            event_type: "chat_local_session".to_string(),
            payload: json!({ "turn": 3 }),
            run_id: None,
        };
        let updates = remote_chat_pin_updates(&local)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(updates["local_session_out"]["turn"], 3);

        assert!(is_stream_output_event("chat_stream_partial"));
        assert!(is_stream_output_event("a2ui"));
        assert!(is_stream_output_event("interaction_request"));
        assert!(!is_stream_output_event("progress"));
        assert!(!is_stream_output_event("completed"));
    }
}
