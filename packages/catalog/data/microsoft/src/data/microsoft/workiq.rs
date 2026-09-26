use super::copilot::{
    CopilotChatEndpoint, add_copilot_chat_pins, normalize_timezone_pin, run_copilot_chat,
};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_data_support::work_iq::{
    WORK_IQ_BASE_URL, WORK_IQ_PROVIDER_ID, WORK_IQ_SCOPE,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

const CATEGORY: &str = "Data/Microsoft/Work IQ";
const OPERATIONS: [&str; 6] = ["Fetch", "Create", "Update", "Delete", "Action", "Function"];

/// Delegated Work IQ credentials. Work IQ tokens are issued for `api://workiq.svc.cloud.microsoft`,
/// so they are not interchangeable with a Microsoft Graph provider.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct WorkIqProvider {
    pub provider_id: String,
    pub access_token: String,
}

fn add_provider_output_pin(node: &mut Node) {
    node.add_output_pin(
        "provider",
        "Provider",
        "Microsoft Work IQ provider with delegated authentication",
        VariableType::Struct,
    )
    .set_schema::<WorkIqProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_provider_input_pin(node: &mut Node) {
    node.add_input_pin(
        "provider",
        "Provider",
        "Microsoft Work IQ provider",
        VariableType::Struct,
    )
    .set_schema::<WorkIqProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn work_iq_scores(performance: u8) -> NodeScores {
    NodeScores::new()
        .set_privacy(6)
        .set_security(8)
        .set_performance(performance)
        .set_governance(8)
        .set_reliability(7)
        .set_cost(3)
        .build()
}

// =============================================================================
// Providers
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct WorkIqOAuthProviderNode {}

#[async_trait]
impl NodeLogic for WorkIqOAuthProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_workiq_provider_oauth",
            "Work IQ (OAuth)",
            "Connect to Microsoft Work IQ with delegated OAuth. A tenant admin must enable Work IQ and consent to WorkIQAgent.Ask. Work IQ usage is billed in Copilot Credits.",
            CATEGORY,
        );
        node.set_flowscript_name("microsoft.workiq", "providerOauth");
        node.add_icon("/flow/icons/microsoft.svg");

        add_provider_output_pin(&mut node);

        node.add_oauth_provider(WORK_IQ_PROVIDER_ID);
        node.add_required_oauth_scopes(WORK_IQ_PROVIDER_ID, vec![WORK_IQ_SCOPE]);
        node.set_scores(work_iq_scores(8));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token = context
            .get_oauth_token(WORK_IQ_PROVIDER_ID)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Microsoft Work IQ is not authenticated or its token expired. Authorize access when prompted."
                )
            })?
            .access_token
            .clone();

        let provider = WorkIqProvider {
            provider_id: WORK_IQ_PROVIDER_ID.to_string(),
            access_token: token,
        };
        context.set_pin_value("provider", json!(provider)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct WorkIqTokenProviderNode {}

#[async_trait]
impl NodeLogic for WorkIqTokenProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_workiq_provider_token",
            "Work IQ (Token)",
            "Connect to Microsoft Work IQ with a delegated access token for api://workiq.svc.cloud.microsoft (for example from an on-behalf-of exchange). App-only tokens are rejected by Work IQ.",
            CATEGORY,
        );
        node.set_flowscript_name("microsoft.workiq", "providerToken");
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin(
            "token",
            "Access Token",
            "Delegated Work IQ access token (audience api://workiq.svc.cloud.microsoft)",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        add_provider_output_pin(&mut node);

        node.set_scores(work_iq_scores(9));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token: String = context.evaluate_pin("token").await?;
        if token.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "Work IQ access token is empty; provide a delegated token for api://workiq.svc.cloud.microsoft"
            ));
        }

        let provider = WorkIqProvider {
            provider_id: WORK_IQ_PROVIDER_ID.to_string(),
            access_token: token,
        };
        context.set_pin_value("provider", json!(provider)).await?;
        Ok(())
    }
}

// =============================================================================
// Chat (REST)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct WorkIqChatNode {}

#[async_trait]
impl NodeLogic for WorkIqChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_workiq_chat",
            "Work IQ Chat",
            "Chat with Microsoft 365 Copilot through the Work IQ REST API, grounded in the user's Microsoft 365 data and optionally the web. Streams the reply and returns citations. Billed per use in Copilot Credits.",
            CATEGORY,
        );
        node.set_flowscript_name("microsoft.workiq", "chat");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        add_copilot_chat_pins(&mut node);

        node.add_required_oauth_scopes(WORK_IQ_PROVIDER_ID, vec![WORK_IQ_SCOPE]);
        node.set_long_running(true);
        node.set_scores(work_iq_scores(6));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("done").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: WorkIqProvider = context.evaluate_pin("provider").await?;
        let endpoint = CopilotChatEndpoint {
            conversations_url: format!("{WORK_IQ_BASE_URL}/rest/conversations"),
            access_token: provider.access_token,
            model: "microsoft-work-iq",
            log_label: "Invoking Microsoft Work IQ Chat",
        };
        run_copilot_chat(context, endpoint).await
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        normalize_timezone_pin(node);
    }
}

// =============================================================================
// Request (remote MCP entity tools)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct WorkIqRequestNode {}

#[async_trait]
impl NodeLogic for WorkIqRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_workiq_request",
            "Work IQ Request",
            "Read or change Microsoft 365 data through the Work IQ MCP entity tools, addressed by a relative Microsoft Graph path. Collections return at most 100 items and cannot be paged. Create, Update, Delete and Action are blocked until a tenant admin allows them in Work IQ policy.",
            CATEGORY,
        );
        node.set_flowscript_name("microsoft.workiq", "request");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        node.add_input_pin(
            "operation",
            "Operation",
            "Fetch reads, Create posts to a collection, Update patches an entity, Delete removes it, Action runs a side-effect action (e.g. /me/sendMail), Function calls a Graph function (e.g. /me/calendarView?...)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Fetch")))
        .set_options(
            PinOptions::new()
                .set_valid_values(OPERATIONS.iter().map(|op| op.to_string()).collect())
                .build(),
        );
        node.add_input_pin(
            "path",
            "Path",
            "Relative Microsoft Graph path such as /me/messages?$top=10 or /me/events/{id}. Allowed prefixes depend on tenant policy (by default /me/, /users/, /sites/).",
            VariableType::String,
        );
        node.add_input_pin(
            "body",
            "Body",
            "JSON body for Create, Update and Action",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Normal)
        .set_default_value(Some(json!(null)))
        .set_open_schema();

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "status",
            "Status",
            "HTTP status code Microsoft Graph returned through Work IQ",
            VariableType::Integer,
        );
        node.add_output_pin(
            "response",
            "Response",
            "JSON returned by Microsoft Graph",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin(
            "values",
            "Values",
            "Collection items when the response has a value array",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(WORK_IQ_PROVIDER_ID, vec![WORK_IQ_SCOPE]);
        node.set_long_running(true);
        node.set_scores(work_iq_scores(7));
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: WorkIqProvider = context.evaluate_pin("provider").await?;
        let operation: String = context.evaluate_pin("operation").await?;
        let path: String = context.evaluate_pin("path").await?;
        let body: flow_like_types::Value = context
            .evaluate_pin("body")
            .await
            .unwrap_or(flow_like_types::Value::Null);
        let path = super::graph::normalize_graph_path(&path);

        let failure = match mcp::request(&provider, &operation, &path, &body).await {
            Ok(response) => {
                let failure = (response.status >= 400).then(|| {
                    format!(
                        "Work IQ {operation} {path} returned HTTP {}",
                        response.status
                    )
                });
                write_entity_response(context, response).await?;
                failure
            }
            Err(error) => Some(error.to_string()),
        };

        match failure {
            Some(message) => {
                context
                    .set_pin_value("error_message", json!(message))
                    .await?;
                context.activate_exec_pin("error").await
            }
            None => context.activate_exec_pin("exec_out").await,
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Work IQ Request requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn write_entity_response(
    context: &mut ExecutionContext,
    response: mcp::EntityResponse,
) -> flow_like_types::Result<()> {
    let values = response.data["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    context
        .set_pin_value("status", json!(response.status))
        .await?;
    context.set_pin_value("values", json!(values)).await?;
    context.set_pin_value("response", response.data).await
}

#[cfg(feature = "execute")]
mod mcp {
    use super::{OPERATIONS, WorkIqProvider};
    use flow_like_catalog_data_support::work_iq::WORK_IQ_MCP_URL;
    use flow_like_types::{Value, json::json};
    use rmcp::{
        ServiceExt,
        model::{
            CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, Implementation,
            JsonObject,
        },
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    use std::time::Duration;

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
    const CALL_TIMEOUT: Duration = Duration::from_secs(120);

    #[derive(Debug, PartialEq)]
    pub(super) struct EntityResponse {
        pub status: i64,
        pub data: Value,
    }

    pub(super) async fn request(
        provider: &WorkIqProvider,
        operation: &str,
        path: &str,
        body: &Value,
    ) -> flow_like_types::Result<EntityResponse> {
        let (tool, arguments) = tool_call(operation, path, body)?;
        let result = call_tool(&provider.access_token, tool, arguments).await?;
        if result.is_error == Some(true) {
            return Err(flow_like_types::anyhow!(
                "Work IQ tool '{tool}' failed for {path}: {}",
                result_text(&result)
            ));
        }
        Ok(entity_response(tool_payload(&result)))
    }

    pub(super) fn tool_call(
        operation: &str,
        path: &str,
        body: &Value,
    ) -> flow_like_types::Result<(&'static str, JsonObject)> {
        let json_body = || -> Option<String> {
            match body {
                Value::Null => None,
                Value::String(encoded) => Some(encoded.clone()),
                other => Some(other.to_string()),
            }
        };
        let required_body = || {
            json_body().ok_or_else(|| {
                flow_like_types::anyhow!("Work IQ {operation} on {path} requires a JSON body")
            })
        };

        let (tool, arguments) = match operation {
            "Fetch" => ("fetch", json!({ "entityUrls": [path] })),
            "Create" => (
                "create_entity",
                json!({ "parentUrl": path, "jsonBody": required_body()? }),
            ),
            "Update" => (
                "update_entity",
                json!({ "entityUrl": path, "jsonBody": required_body()? }),
            ),
            "Delete" => ("delete_entity", json!({ "entityUrl": path })),
            "Action" => {
                let mut arguments = json!({ "actionUrl": path });
                if let Some(encoded) = json_body() {
                    arguments["jsonBody"] = json!(encoded);
                }
                ("do_action", arguments)
            }
            "Function" => ("call_function", json!({ "functionUrl": path })),
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown Work IQ operation '{other}'; expected one of {}",
                    OPERATIONS.join(", ")
                ));
            }
        };

        Ok((tool, arguments.as_object().cloned().unwrap_or_default()))
    }

    async fn call_tool(
        access_token: &str,
        tool: &'static str,
        arguments: JsonObject,
    ) -> flow_like_types::Result<CallToolResult> {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(WORK_IQ_MCP_URL)
                .auth_header(access_token.to_string()),
        );
        let mut implementation = Implementation::new("Flow-Like", env!("CARGO_PKG_VERSION"));
        implementation.website_url = Some("https://flow-like.com".to_string());
        let client_info = ClientInfo::new(ClientCapabilities::default(), implementation);

        let client = tokio::time::timeout(CONNECT_TIMEOUT, client_info.serve(transport))
            .await
            .map_err(|_| {
                flow_like_types::anyhow!(
                    "Timed out connecting to Work IQ MCP at {WORK_IQ_MCP_URL} after {CONNECT_TIMEOUT:?}"
                )
            })?
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "Failed to connect to Work IQ MCP at {WORK_IQ_MCP_URL}: {error}"
                )
            })?;

        let mut params = CallToolRequestParams::new(tool);
        params.arguments = Some(arguments);
        let result = tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params)).await;
        let _ = client.cancel().await;

        result
            .map_err(|_| {
                flow_like_types::anyhow!("Work IQ tool '{tool}' timed out after {CALL_TIMEOUT:?}")
            })?
            .map_err(|error| flow_like_types::anyhow!("Work IQ tool '{tool}' failed: {error}"))
    }

    fn result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
            .collect::<Vec<_>>()
            .join("")
    }

    pub(super) fn tool_payload(result: &CallToolResult) -> Value {
        if let Some(structured) = &result.structured_content {
            return structured.clone();
        }
        let text = result_text(result);
        flow_like_types::json::from_str(&text).unwrap_or(Value::String(text))
    }

    /// `fetch` wraps each path in `results[]`; the other entity tools return `{ statusCode, data }`.
    pub(super) fn entity_response(payload: Value) -> EntityResponse {
        let entry = payload
            .get("results")
            .and_then(|results| results.get(0))
            .cloned()
            .unwrap_or(payload);
        match entry.get("statusCode").and_then(Value::as_i64) {
            Some(status) => EntityResponse {
                status,
                data: entry.get("data").cloned().unwrap_or(Value::Null),
            },
            None => EntityResponse {
                status: 200,
                data: entry,
            },
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rmcp::model::Content;

        #[test]
        fn operations_map_to_work_iq_tool_arguments() {
            let body = json!({ "subject": "Hi" });

            let (tool, args) = tool_call("Fetch", "/me/messages", &Value::Null).unwrap();
            assert_eq!(tool, "fetch");
            assert_eq!(
                Value::Object(args),
                json!({ "entityUrls": ["/me/messages"] })
            );

            let (tool, args) = tool_call("Create", "/me/messages", &body).unwrap();
            assert_eq!(tool, "create_entity");
            assert_eq!(args["parentUrl"], "/me/messages");
            assert_eq!(args["jsonBody"], json!(body.to_string()));

            let (tool, args) = tool_call("Update", "/me/messages/1", &body).unwrap();
            assert_eq!(tool, "update_entity");
            assert_eq!(args["entityUrl"], "/me/messages/1");

            let (tool, args) = tool_call("Delete", "/me/messages/1", &Value::Null).unwrap();
            assert_eq!(tool, "delete_entity");
            assert_eq!(
                Value::Object(args),
                json!({ "entityUrl": "/me/messages/1" })
            );

            let (tool, args) = tool_call("Action", "/me/messages/1/send", &Value::Null).unwrap();
            assert_eq!(tool, "do_action");
            assert!(args.get("jsonBody").is_none());

            let (tool, args) = tool_call("Function", "/me/calendarView", &Value::Null).unwrap();
            assert_eq!(tool, "call_function");
            assert_eq!(args["functionUrl"], "/me/calendarView");
        }

        #[test]
        fn pre_encoded_string_body_is_not_double_encoded() {
            let (_, args) =
                tool_call("Create", "/me/events", &json!("{\"subject\":\"Hi\"}")).unwrap();
            assert_eq!(args["jsonBody"], "{\"subject\":\"Hi\"}");
        }

        #[test]
        fn writes_without_body_and_unknown_operations_are_rejected() {
            let error = tool_call("Create", "/me/events", &Value::Null).unwrap_err();
            assert!(error.to_string().contains("requires a JSON body"));

            let error = tool_call("Patch", "/me", &Value::Null).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Unknown Work IQ operation 'Patch'")
            );
        }

        #[test]
        fn fetch_payload_unwraps_the_first_result() {
            let payload = json!({
                "results": [{ "data": { "value": [{ "id": "1" }] }, "statusCode": 200 }]
            });
            assert_eq!(
                entity_response(payload),
                EntityResponse {
                    status: 200,
                    data: json!({ "value": [{ "id": "1" }] }),
                }
            );
        }

        #[test]
        fn entity_payload_keeps_graph_status_and_data() {
            assert_eq!(
                entity_response(json!({ "statusCode": 204 })),
                EntityResponse {
                    status: 204,
                    data: Value::Null,
                }
            );
            assert_eq!(
                entity_response(json!({ "statusCode": 403, "data": { "error": "denied" } })),
                EntityResponse {
                    status: 403,
                    data: json!({ "error": "denied" }),
                }
            );
        }

        #[test]
        fn payload_falls_back_to_json_text_content() {
            let result = CallToolResult::success(vec![Content::text(
                "{\"statusCode\":201,\"data\":{\"id\":\"42\"}}",
            )]);
            assert_eq!(
                entity_response(tool_payload(&result)),
                EntityResponse {
                    status: 201,
                    data: json!({ "id": "42" }),
                }
            );
        }
    }
}
