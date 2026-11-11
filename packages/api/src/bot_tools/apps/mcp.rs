use flow_like::flow::event::Event;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::{AppTools, ToolResult, text};
use crate::{entity::event as event_entity, routes::inbound::dispatch_bot_mcp_operation};

const MISSING_TOOL: &str = "This is an MCP server interface. Read describe_app_interface, then set mcp_tool to an exact tool name and payload to its arguments ({} for no arguments).";
const INVOCATION_INSTRUCTIONS: &str = "Set mcp_tool to an exact tools[].name and payload to the arguments described by that tool's inputSchema. Omit payload or pass {} for a tool with no arguments. The runtime fills the internal payload and _client pins automatically.";

impl AppTools {
    pub(super) async fn mcp_tools(&self, app_id: &str, event: &Event) -> Result<Value, String> {
        let result = self
            .mcp_operation(app_id, event, "tools/list", json!({}))
            .await?;
        inventory(app_id, &event.id, result)
    }

    pub(super) async fn call_mcp_tool(
        &self,
        app_id: &str,
        event: &Event,
        args: &Value,
    ) -> ToolResult {
        let (name, arguments) = tool_call(args)?;
        let result = self
            .mcp_operation(
                app_id,
                event,
                "tools/call",
                json!({"name": name, "arguments": arguments}),
            )
            .await?;
        Ok(call_outcome(app_id, &event.id, name, result))
    }

    async fn mcp_operation(
        &self,
        app_id: &str,
        event: &Event,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let row = event_entity::Entity::find_by_id(&event.id)
            .filter(event_entity::Column::AppId.eq(app_id))
            .one(&self.state.db)
            .await
            .map_err(|error| format!("MCP event '{}' could not be read: {error}", event.id))?
            .ok_or_else(|| format!("Event '{}' not found in app '{app_id}'.", event.id))?;
        dispatch_bot_mcp_operation(&self.state, &row, method, params, self.principal.clone())
            .await
            .map_err(|error| format!("MCP {method} on event '{}' failed: {error}", event.id))
    }
}

fn tool_call(args: &Value) -> Result<(&str, Value), String> {
    let name = text(args, "mcp_tool").ok_or_else(|| MISSING_TOOL.to_string())?;
    match args.get("payload") {
        None | Some(Value::Null) => Ok((name, json!({}))),
        Some(payload @ Value::Object(_)) => Ok((name, payload.clone())),
        Some(_) => Err("MCP tool arguments must be an object.".to_string()),
    }
}

fn inventory(app_id: &str, event_id: &str, result: Value) -> Result<Value, String> {
    let tools = result
        .get("tools")
        .filter(|tools| tools.is_array())
        .ok_or_else(|| {
            format!("MCP tools/list on event '{event_id}' did not return a tool inventory.")
        })?;
    Ok(json!({
        "tools": tools,
        "invocation": {
            "consumer_tool": "call_app_event",
            "app_id": app_id,
            "event_id": event_id,
            "instructions": INVOCATION_INSTRUCTIONS,
        },
    }))
}

fn call_outcome(app_id: &str, event_id: &str, tool: &str, result: Value) -> Value {
    let failed = result.get("isError").and_then(Value::as_bool) == Some(true);
    json!({
        "status": if failed { "error" } else { "ok" },
        "app_id": app_id,
        "event_id": event_id,
        "event_type": "mcp",
        "mcp_tool": tool,
        "result": result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_call_needs_a_tool_name() {
        for args in [json!({}), json!({"mcp_tool": "  "}), json!({"mcp_tool": 3})] {
            assert_eq!(tool_call(&args).unwrap_err(), MISSING_TOOL);
        }
    }

    #[test]
    fn tool_arguments_default_to_an_empty_object_and_must_be_objects() {
        for args in [
            json!({"mcp_tool": "t"}),
            json!({"mcp_tool": "t", "payload": null}),
        ] {
            assert_eq!(tool_call(&args).unwrap(), ("t", json!({})));
        }
        let args = json!({"mcp_tool": " t ", "payload": {"a": 1}});
        assert_eq!(tool_call(&args).unwrap(), ("t", json!({"a": 1})));
        for payload in [json!([1]), json!("x"), json!(1)] {
            let args = json!({"mcp_tool": "t", "payload": payload});
            assert_eq!(
                tool_call(&args).unwrap_err(),
                "MCP tool arguments must be an object."
            );
        }
    }

    #[test]
    fn the_inventory_carries_the_tools_and_how_to_call_them() {
        let listed = inventory("a1", "e1", json!({"tools": [{"name": "t"}]})).unwrap();
        assert_eq!(listed["tools"], json!([{"name": "t"}]));
        assert_eq!(listed["invocation"]["consumer_tool"], "call_app_event");
        assert_eq!(listed["invocation"]["app_id"], "a1");
        assert_eq!(listed["invocation"]["event_id"], "e1");
        for result in [json!({}), json!({"tools": "x"})] {
            assert!(
                inventory("a1", "e1", result)
                    .unwrap_err()
                    .contains("did not return a tool inventory")
            );
        }
    }

    #[test]
    fn a_tool_result_is_an_error_only_when_the_server_says_so() {
        let ok = call_outcome("a1", "e1", "t", json!({"content": [], "isError": false}));
        assert_eq!(ok["status"], "ok");
        let missing = call_outcome("a1", "e1", "t", json!({"content": []}));
        assert_eq!(missing["status"], "ok");
        let failed = call_outcome("a1", "e1", "t", json!({"isError": true}));
        assert_eq!(failed["status"], "error");
        assert_eq!(failed["event_type"], "mcp");
        assert_eq!(failed["mcp_tool"], "t");
        assert_eq!(failed["result"], json!({"isError": true}));
    }
}
