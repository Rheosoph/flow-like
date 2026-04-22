use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets all children of a container element.
///
/// Looks up the element's children.explicitList and returns the full child elements.
#[crate::register_node]
#[derive(Default)]
pub struct QueryChildren;

impl QueryChildren {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for QueryChildren {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_query_children",
            "Query Children",
            "Gets all child elements of a container",
            "UI/Elements/Query",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Container",
            "Reference to the container element",
            VariableType::String,
        );

        node.add_output_pin(
            "children",
            "Children",
            "Array of child elements",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "child_ids",
            "Child IDs",
            "Array of child element IDs",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of children",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_id: String = context.evaluate_pin("element_ref").await?;

        let elements = context.get_frontend_elements().await?;

        let Some(elements_map) = elements else {
            context.log_message("No elements in payload", LogLevel::Warn);
            context
                .get_pin_by_name("children")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("child_ids")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("count")
                .await?
                .set_value(Value::Number(0.into()))
                .await;
            return Ok(());
        };

        let Some(element) = elements_map.get(&element_id) else {
            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Warn,
            );
            context
                .get_pin_by_name("children")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("child_ids")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("count")
                .await?
                .set_value(Value::Number(0.into()))
                .await;
            return Ok(());
        };

        // Get children.explicitList from component
        let child_ids: Vec<String> = element
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

        // Fetch full element data for each child
        let mut children: Vec<Value> = Vec::new();
        for child_id in &child_ids {
            if let Some(child_element) = elements_map.get(child_id) {
                let mut child_with_id = child_element.clone();
                if let Some(obj) = child_with_id.as_object_mut() {
                    obj.insert("_id".to_string(), Value::String(child_id.clone()));
                }
                children.push(child_with_id);
            }
        }

        let count = children.len() as i64;

        context.log_message(
            &format!("Found {} children for element '{}'", count, element_id),
            LogLevel::Debug,
        );

        context
            .get_pin_by_name("children")
            .await?
            .set_value(Value::Array(children))
            .await;
        context
            .get_pin_by_name("child_ids")
            .await?
            .set_value(Value::Array(
                child_ids.into_iter().map(Value::String).collect(),
            ))
            .await;
        context
            .get_pin_by_name("count")
            .await?
            .set_value(Value::Number(count.into()))
            .await;

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
            node.friendly_name = format!("Children of {}", id);
        } else {
            node.friendly_name = "Query Children".to_string();
        }
    }
}
