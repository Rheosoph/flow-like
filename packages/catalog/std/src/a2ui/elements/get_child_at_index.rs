use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct GetChildAtIndex;

impl GetChildAtIndex {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetChildAtIndex {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_child_at_index",
            "Get Child At Index",
            "Gets a child element at a specific index from a container",
            "UI/Elements/Containers",
        );
        node.set_flowscript_name("ui", "getChildAtIndex");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "container_ref",
            "Container",
            "Reference to the container element",
            VariableType::String,
        );

        node.add_input_pin(
            "index",
            "Index",
            "The index of the child to get (0-based)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "child",
            "Child",
            "The child element at the specified index",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "child_id",
            "Child ID",
            "The ID of the child element",
            VariableType::String,
        );

        node.add_output_pin(
            "found",
            "Found",
            "Whether a child was found at the index",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let container_id: String = context.evaluate_pin("container_ref").await?;
        let index: i64 = context.evaluate_pin("index").await?;

        let elements = context.get_frontend_elements().await?;

        let Some(elements_map) = elements else {
            context.log_message("No elements in payload", LogLevel::Warn);
            context
                .get_pin_by_name("child")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("child_id")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("found")
                .await?
                .set_value(Value::Bool(false))
                .await;
            return Ok(());
        };

        let Some(container) = elements_map.get(&container_id) else {
            context.log_message(
                &format!("Container not found: {}", container_id),
                LogLevel::Warn,
            );
            context
                .get_pin_by_name("child")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("child_id")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("found")
                .await?
                .set_value(Value::Bool(false))
                .await;
            return Ok(());
        };

        let child_ids: Vec<String> = container
            .get("component")
            .and_then(|c| c.get("children"))
            .and_then(|ch| ch.get("explicitList"))
            .and_then(|list| list.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let idx = index as usize;
        if idx >= child_ids.len() {
            context.log_message(
                &format!(
                    "Index {} out of bounds (length: {})",
                    index,
                    child_ids.len()
                ),
                LogLevel::Warn,
            );
            context
                .get_pin_by_name("child")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("child_id")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("found")
                .await?
                .set_value(Value::Bool(false))
                .await;
            return Ok(());
        }

        let child_id = &child_ids[idx];
        let child_element = elements_map.get(child_id).cloned();

        if let Some(mut child) = child_element {
            if let Some(obj) = child.as_object_mut() {
                obj.insert("_id".to_string(), Value::String(child_id.clone()));
            }
            context
                .get_pin_by_name("child")
                .await?
                .set_value(child)
                .await;
            context
                .get_pin_by_name("child_id")
                .await?
                .set_value(Value::String(child_id.clone()))
                .await;
            context
                .get_pin_by_name("found")
                .await?
                .set_value(Value::Bool(true))
                .await;
        } else {
            context
                .get_pin_by_name("child")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("child_id")
                .await?
                .set_value(Value::String(child_id.clone()))
                .await;
            context
                .get_pin_by_name("found")
                .await?
                .set_value(Value::Bool(false))
                .await;
        }

        Ok(())
    }
}
