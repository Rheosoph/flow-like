use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::element_utils::extract_element_id;

/// Creates a new element and adds it to a parent container.
///
/// The element is created dynamically and inserted into the specified parent.
#[crate::register_node]
#[derive(Default)]
pub struct CreateElement;

impl CreateElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_create_element",
            "Create Element",
            "Creates a new element and adds it to a parent container",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "createElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "The surface to create the element in",
            VariableType::String,
        );

        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Parent element ID string or element object from Get Element",
            VariableType::Generic,
        );

        node.add_input_pin(
            "element_id",
            "Element ID",
            "Unique ID for the new element",
            VariableType::String,
        );

        node.add_input_pin(
            "component_type",
            "Type",
            "The component type (e.g., 'Text', 'Button', 'Container')",
            VariableType::String,
        );

        node.add_input_pin(
            "props",
            "Props",
            "Component properties as JSON object",
            VariableType::Generic,
        );

        node.add_input_pin(
            "index",
            "Index",
            "Optional index to insert at (default: append at end)",
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "created_id",
            "Created ID",
            "The ID of the created element",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let parent_value: Value = context.evaluate_pin("parent_id").await?;
        let parent_id = extract_element_id(&parent_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid parent reference - expected string ID or element object"
            )
        })?;
        let element_id: String = context.evaluate_pin("element_id").await?;
        let component_type: String = context.evaluate_pin("component_type").await?;
        let props: Value = context
            .evaluate_pin("props")
            .await
            .unwrap_or(Value::Object(Default::default()));
        let index: Option<i64> = context.evaluate_pin("index").await.ok();

        // Build the component Value
        let component_value = if let Some(obj) = props.as_object() {
            let mut map = obj.clone();
            map.insert("type".to_string(), Value::String(component_type));
            Value::Object(map)
        } else {
            let mut map = flow_like_types::json::Map::new();
            map.insert("type".to_string(), Value::String(component_type));
            Value::Object(map)
        };

        // Create the SurfaceComponent
        let surface_component =
            flow_like::a2ui::SurfaceComponent::new(element_id.clone(), component_value);

        context
            .create_element(
                &surface_id,
                &parent_id,
                surface_component,
                index.map(|i| i as usize),
            )
            .await?;

        context
            .set_pin_value("created_id", Value::String(element_id))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
