use super::chat_event::Attachment;
use crate::data::path::FlowPath;
use crate::remote_util::{RemoteAppSession, error_for_status, http_client, validate_path_id};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flow_like_model_provider::history::History;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, tokio};
use futures::StreamExt;
use serde::Deserialize;
use std::time::Duration;

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
        if let Some(pin) = node.pins.values_mut().find(|pin| pin.name == spec.name) {
            pin.data_type = spec.data_type.clone();
            pin.value_type = spec.value_type.clone();
            pin.schema = spec.schema.clone();
            pin.options = build_options(spec);
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
        if node.pins.values().any(|pin| pin.name == spec.name) {
            continue;
        }
        let pin = node.add_output_pin(
            &spec.name,
            &spec.friendly,
            &spec.description,
            spec.data_type.clone(),
        );
        pin.value_type = spec.value_type.clone();
        pin.schema = spec.schema.clone();
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
        "Maximum time to wait for the remote run to finish",
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
        ),
        PinSpec::new(
            "history",
            "History",
            "Prior conversation history",
            VariableType::Struct,
        )
        .schema(schema_string::<History>())
        .enforce(),
        PinSpec::new(
            "local_session",
            "Local Session",
            "Local session state",
            VariableType::Struct,
        ),
        PinSpec::new(
            "global_session",
            "Global Session",
            "Global session state",
            VariableType::Struct,
        ),
        PinSpec::new(
            "tools",
            "Tools",
            "Tool ids the assistant may use",
            VariableType::String,
        )
        .array(),
        PinSpec::new(
            "attachments",
            "Attachments",
            "Attachments to include",
            VariableType::Struct,
        )
        .array()
        .schema(schema_string::<Attachment>())
        .enforce(),
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
        ),
        PinSpec::new("body", "Body", "Request body (JSON)", VariableType::Generic),
        PinSpec::new(
            "headers",
            "Headers",
            "Additional request headers as an object",
            VariableType::Generic,
        ),
        timeout_spec(),
    ];

    let selection = pin_string(node, "route");
    let mut is_file = false;
    if let Some((method, path)) = selected_route(meta, &selection) {
        is_file = is_file_route(meta, &method, &path);
        for param in &template_params(&path) {
            inputs.push(PinSpec::new(
                &format!("param_{}", param),
                param,
                "Path parameter",
                VariableType::String,
            ));
        }
    }

    let mut outputs = vec![
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
    ];
    if is_file {
        outputs.push(
            PinSpec::new("file", "File", "Downloaded file", VariableType::Struct)
                .schema(flow_path_schema()),
        );
    }
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
        node.set_version(3);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
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
            "Event of the selected project to invoke",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            PIN_REMOTE_EVENT_META,
            "Event Details",
            "Auto-filled by the editor when an event is selected. Drives the input and output pins.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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

        let session = RemoteAppSession::open(context, &remote_app_id).await?;

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
            let response = post_json(session, &url, &body).await?;
            let queued: Value = response.json().await?;
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
        let response_text = extract_text(&response);

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
            path = path.replace(&format!("{{{}}}", param), value.trim());
        }

        let query: Value = context.evaluate_pin("query").await.unwrap_or(Value::Null);
        let body: Value = context.evaluate_pin("body").await.unwrap_or(Value::Null);
        let headers: Value = context.evaluate_pin("headers").await.unwrap_or(Value::Null);

        let url = session.url(&format!(
            "events/{}/rest{}",
            event_id,
            ensure_leading_slash(&path)
        ));
        let http_method = flow_like_types::reqwest::Method::from_bytes(method.as_bytes())
            .unwrap_or(flow_like_types::reqwest::Method::GET);
        let mut request = http_client()
            .request(http_method, &url)
            .bearer_auth(&session.token);

        if let Some(query_obj) = query.as_object() {
            let pairs: Vec<(String, String)> = query_obj
                .iter()
                .map(|(k, v)| (k.clone(), value_to_query(v)))
                .collect();
            request = request.query(&pairs);
        }
        if let Some(header_obj) = headers.as_object() {
            for (name, value) in header_obj {
                if let Some(value) = value.as_str() {
                    request = request.header(name, value);
                }
            }
        }
        if !body.is_null() {
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .map_err(|err| flow_like_types::anyhow!("Remote REST call failed: {}", err))?;
        let response = error_for_status(response, "Remote REST call").await?;

        let status = response.status().as_u16() as i64;
        let header_map: flow_like_types::json::Map<String, Value> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.to_string(), json!(v))))
            .collect();
        let content_type = response
            .headers()
            .get(flow_like_types::reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        context.set_pin_value("status", json!(status)).await?;
        context
            .set_pin_value("response_headers", json!(header_map))
            .await?;

        let bytes = response
            .bytes()
            .await
            .map_err(|err| flow_like_types::anyhow!("Failed to read REST response: {}", err))?;

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

        if mode == MCP_MODE_READ_RESOURCE {
            let uri: String = context.evaluate_pin("resource").await.unwrap_or_default();
            if uri.trim().is_empty() {
                return Err(flow_like_types::anyhow!("No resource selected"));
            }
            let result = session
                .mcp_request(event_id, "resources/read", json!({ "uri": uri }))
                .await?;
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

        let result = session
            .mcp_request(
                event_id,
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await?;

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
// Invocation helpers (SSE collection)
// ---------------------------------------------------------------------------

struct SseOutcome {
    run_id: Option<String>,
    status: Option<String>,
    error_message: Option<String>,
    generic_result: Option<Value>,
    chat_out: Option<Value>,
    chat_stream: Option<Value>,
}

impl SseOutcome {
    fn status_str(&self) -> String {
        self.status
            .clone()
            .unwrap_or_else(|| "Completed".to_string())
    }

    fn ensure_ok(&self) -> flow_like_types::Result<()> {
        if matches!(
            self.status.as_deref(),
            Some("Failed") | Some("Cancelled") | Some("Timeout")
        ) {
            return Err(flow_like_types::anyhow!(
                "Remote run {} ended with status {}: {}",
                self.run_id.clone().unwrap_or_default(),
                self.status_str(),
                self.error_message.clone().unwrap_or_default()
            ));
        }
        Ok(())
    }

    fn chat_result(&self) -> Option<Value> {
        self.chat_out
            .clone()
            .or_else(|| self.chat_stream.clone())
            .or_else(|| self.generic_result.clone())
    }
}

async fn post_json(
    session: &RemoteAppSession,
    url: &str,
    body: &Value,
) -> flow_like_types::Result<flow_like_types::reqwest::Response> {
    let response = http_client()
        .post(url)
        .bearer_auth(&session.token)
        .json(body)
        .send()
        .await
        .map_err(|err| flow_like_types::anyhow!("Failed to invoke remote event: {}", err))?;
    error_for_status(response, "Remote event invocation").await
}

async fn invoke_and_collect(
    session: &RemoteAppSession,
    url: &str,
    body: &Value,
    timeout: u64,
) -> flow_like_types::Result<SseOutcome> {
    let response = post_json(session, url, body).await?;
    tokio::time::timeout(Duration::from_secs(timeout), collect_sse_outcome(response))
        .await
        .map_err(|_| {
            flow_like_types::anyhow!("Remote event did not finish within {} seconds", timeout)
        })?
}

async fn collect_sse_outcome(
    response: flow_like_types::reqwest::Response,
) -> flow_like_types::Result<SseOutcome> {
    let mut outcome = SseOutcome {
        run_id: None,
        status: None,
        error_message: None,
        generic_result: None,
        chat_out: None,
        chat_stream: None,
    };
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    'outer: while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|err| flow_like_types::anyhow!("Failed to read event stream: {}", err))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer.drain(..pos + 2);

            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let Ok(parsed) = flow_like_types::json::from_str::<Value>(data.trim()) else {
                    continue;
                };

                if outcome.run_id.is_none()
                    && let Some(run_id) = parsed.get("run_id").and_then(|v| v.as_str())
                {
                    outcome.run_id = Some(run_id.to_string());
                }

                let event_type = parsed
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                match event_type {
                    "generic_result" if outcome.generic_result.is_none() => {
                        outcome.generic_result = parsed.get("payload").cloned();
                    }
                    "chat_out" => {
                        outcome.chat_out = parsed.get("payload").cloned();
                    }
                    "chat_stream" => {
                        outcome.chat_stream = parsed.get("payload").cloned();
                    }
                    "completed" => {
                        outcome.status = parsed
                            .get("payload")
                            .and_then(|p| p.get("status"))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        outcome.error_message = parsed
                            .get("payload")
                            .and_then(|p| p.get("error_message"))
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        break 'outer;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(outcome)
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
    let base = path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("download")
        .to_string();
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
