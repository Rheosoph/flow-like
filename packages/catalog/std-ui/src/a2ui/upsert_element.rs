use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct UpsertElement;

impl UpsertElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpsertElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_upsert_element",
            "Upsert Element",
            "Updates or inserts an element value in the frontend",
            "UI/Data",
        );
        node.set_flowscript_name("ui", "upsertElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_id",
            "Element ID",
            "ID of the element to update (e.g., 'main/status-text')",
            VariableType::String,
        );

        node.add_input_pin(
            "value",
            "Value",
            "New value for the element",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_id: String = context.evaluate_pin("element_id").await?;
        let value: Value = context.evaluate_pin("value").await?;
        ensure_component_payload(&element_id, &value)?;

        context.upsert_element(&element_id, value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

/// A `createComponent` without a typed `component` would leave the page an
/// element it cannot render, so it fails here where the author can see why.
fn ensure_component_payload(element_id: &str, value: &Value) -> flow_like_types::Result<()> {
    let Some(update) = value.as_object() else {
        return Ok(());
    };
    if update.get("type").and_then(Value::as_str) != Some("createComponent") {
        return Ok(());
    }
    let typed = update
        .get("component")
        .and_then(Value::as_object)
        .and_then(|component| component.get("type"))
        .is_some_and(Value::is_string);
    if typed {
        return Ok(());
    }
    Err(flow_like_types::anyhow!(
        "Upsert Element '{}': createComponent needs a `component` object with a `type`; to change props of the existing element send {{\"type\":\"setProps\",\"props\":{{...}}}} instead",
        element_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn create_component_requires_a_typed_component() {
        let partial = json!({ "type": "createComponent", "props": { "src": "/home/u/x.pdf" } });
        let untyped = json!({ "type": "createComponent", "component": { "src": "x" } });
        assert!(ensure_component_payload("page/preview", &partial).is_err());
        assert!(ensure_component_payload("page/preview", &untyped).is_err());
    }

    #[test]
    fn other_updates_pass_through() {
        for value in [
            json!({ "type": "createComponent", "component": { "type": "filePreview" } }),
            json!({ "type": "setProps", "props": { "src": "/home/u/x.pdf" } }),
            json!({ "mode": "setProps", "props": { "src": "/home/u/x.pdf" } }),
            json!("text"),
        ] {
            assert!(ensure_component_payload("page/preview", &value).is_ok());
        }
    }
}
