use super::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::Map, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetElementAction;

impl SetElementAction {
    pub fn new() -> Self {
        Self
    }
}

fn build_action_update(event_name: &str, action: Option<Value>) -> Value {
    let event_name = event_name.trim();
    if event_name.is_empty() {
        return json!({
            "type": "setAction",
            "action": action
        });
    }

    json!({
        "type": "setEventActions",
        "eventName": event_name,
        "actions": action.into_iter().collect::<Vec<_>>()
    })
}

#[async_trait]
impl NodeLogic for SetElementAction {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_element_action",
            "Set Element Action",
            "Dynamically sets the legacy default action or a named event action of an interactive element",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "setElementAction");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the element (ID string or element object from Get Element)",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.add_input_pin(
            "event_name",
            "Event",
            "Optional named component event (for example click, change, open, or delete). Leave empty to update the legacy default action.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "action_type",
            "Action Type",
            "Type of action: navigate_page, external_link, workflow_event, or clear to remove action",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "navigate_page".to_string(),
                    "external_link".to_string(),
                    "workflow_event".to_string(),
                    "clear".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("navigate_page")));

        node.add_input_pin(
            "route",
            "Route",
            "For navigate_page: the route path (e.g., /about, /products/123)",
            VariableType::String,
        );

        node.add_input_pin(
            "query_params",
            "Query Params",
            "For navigate_page: optional JSON object of query parameters",
            VariableType::String,
        );

        node.add_input_pin(
            "url",
            "URL",
            "For external_link: the external URL to open",
            VariableType::String,
        );

        node.add_input_pin(
            "node_id",
            "Node ID",
            "For workflow_event: the ID of the workflow node to trigger",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);
        node.set_version(1);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;

        if element_value.is_null() {
            return Err(flow_like_types::anyhow!(
                "Element reference is null - the element was not found"
            ));
        }

        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object with __element_id"
            )
        })?;

        let action_type: String = context
            .evaluate_pin("action_type")
            .await
            .unwrap_or_default();
        let event_name: String = context.evaluate_pin("event_name").await.unwrap_or_default();

        let action = if action_type == "clear" {
            None
        } else {
            let mut action_context = Map::new();

            match action_type.as_str() {
                "navigate_page" => {
                    let route: String = context.evaluate_pin("route").await.unwrap_or_default();
                    let query_params: String = context
                        .evaluate_pin("query_params")
                        .await
                        .unwrap_or_default();

                    if !route.is_empty() {
                        action_context.insert("route".to_string(), json!(route));
                    }
                    if !query_params.is_empty() {
                        action_context.insert("queryParams".to_string(), json!(query_params));
                    }
                }
                "external_link" => {
                    let url: String = context.evaluate_pin("url").await.unwrap_or_default();
                    if !url.is_empty() {
                        action_context.insert("url".to_string(), json!(url));
                    }
                }
                "workflow_event" => {
                    let node_id: String = context.evaluate_pin("node_id").await.unwrap_or_default();
                    if !node_id.is_empty() {
                        action_context.insert("nodeId".to_string(), json!(node_id));
                    }
                }
                _ => {}
            }

            Some(json!({
                "name": action_type,
                "context": action_context
            }))
        };

        // Empty event_name retains the exact legacy payload and behavior. A
        // named clear writes an explicit empty list so the event does not fall
        // back to the legacy default action.
        let update_value = build_action_update(&event_name, action);

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_event_name_preserves_legacy_set_action_payload_bytes() {
        let action = Some(json!({
            "name": "navigate_page",
            "context": {
                "route": "/orders",
                "queryParams": "{\"status\":\"open\"}"
            }
        }));

        let actual = flow_like_types::json::to_vec(&build_action_update("", action.clone()))
            .expect("serialize emitted update");
        let legacy = flow_like_types::json::to_vec(&json!({
            "type": "setAction",
            "action": action
        }))
        .expect("serialize legacy update");

        assert_eq!(actual, legacy);
    }

    #[test]
    fn named_event_emits_one_item_ordered_action_list() {
        let action = json!({
            "name": "workflow_event",
            "context": { "nodeId": "handle-open" }
        });

        let update = build_action_update(" open ", Some(action.clone()));

        assert_eq!(
            update,
            json!({
                "type": "setEventActions",
                "eventName": "open",
                "actions": [action]
            })
        );
        assert_eq!(
            update
                .get("actions")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn named_clear_emits_explicit_empty_action_list() {
        assert_eq!(
            build_action_update("delete", None),
            json!({
                "type": "setEventActions",
                "eventName": "delete",
                "actions": []
            })
        );
    }
}
