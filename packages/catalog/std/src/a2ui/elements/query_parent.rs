use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::element_utils::extract_element_id_from_pin;

/// Gets the parent element of a given element.
///
/// Searches through all elements to find which container has this element as a child.
#[crate::register_node]
#[derive(Default)]
pub struct QueryParent;

impl QueryParent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for QueryParent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_query_parent",
            "Query Parent",
            "Gets the parent element of an element",
            "UI/Elements/Query",
        );
        node.set_flowscript_name("ui", "queryParent");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the element to find parent of",
            VariableType::String,
        );

        node.add_output_pin(
            "parent",
            "Parent",
            "The parent element data",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "parent_id",
            "Parent ID",
            "ID of the parent element",
            VariableType::String,
        );

        node.add_output_pin(
            "has_parent",
            "Has Parent",
            "Whether a parent was found",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let elements = context.get_frontend_elements().await?;

        let Some(elements_map) = elements else {
            context.log_message("No elements in payload", LogLevel::Warn);
            context
                .get_pin_by_name("parent")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("parent_id")
                .await?
                .set_value(Value::String(String::new()))
                .await;
            context
                .get_pin_by_name("has_parent")
                .await?
                .set_value(Value::Bool(false))
                .await;
            return Ok(());
        };

        // Search for parent - find element that has this element in its children.explicitList
        let mut parent_id: Option<String> = None;
        let mut parent_element: Option<Value> = None;

        for (id, element) in elements_map {
            let child_ids: Vec<&str> = element
                .get("component")
                .and_then(|c| c.get("children"))
                .and_then(|ch| ch.get("explicitList"))
                .and_then(|list| list.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            if child_ids.contains(&element_id.as_str()) {
                parent_id = Some(id.clone());
                let mut parent_with_id = element.clone();
                if let Some(obj) = parent_with_id.as_object_mut() {
                    obj.insert("_id".to_string(), Value::String(id.clone()));
                }
                parent_element = Some(parent_with_id);
                break;
            }
        }

        if let (Some(pid), Some(parent)) = (parent_id, parent_element) {
            context.log_message(
                &format!("Found parent '{}' for element '{}'", pid, element_id),
                LogLevel::Debug,
            );
            context
                .get_pin_by_name("parent")
                .await?
                .set_value(parent)
                .await;
            context
                .get_pin_by_name("parent_id")
                .await?
                .set_value(Value::String(pid))
                .await;
            context
                .get_pin_by_name("has_parent")
                .await?
                .set_value(Value::Bool(true))
                .await;
        } else {
            context.log_message(
                &format!("No parent found for element '{}'", element_id),
                LogLevel::Debug,
            );
            context
                .get_pin_by_name("parent")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("parent_id")
                .await?
                .set_value(Value::String(String::new()))
                .await;
            context
                .get_pin_by_name("has_parent")
                .await?
                .set_value(Value::Bool(false))
                .await;
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let element_ref = match read_only_node.get_pin_by_name("element_ref") {
            Some(pin) => pin,
            None => return,
        };

        let element_id = element_ref.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        });

        if let Some(id) = element_id {
            node.friendly_name = format!("Parent of {}", id);
        } else {
            node.friendly_name = "Query Parent".to_string();
        }
    }
}
