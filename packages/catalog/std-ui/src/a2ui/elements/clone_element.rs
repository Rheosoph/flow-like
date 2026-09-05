use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Clones an existing element and adds it to a container.
#[crate::register_node]
#[derive(Default)]
pub struct CloneElement;

impl CloneElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CloneElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_clone_element",
            "Clone Element",
            "Clones an existing element and adds it to a container",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "cloneElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "source_element",
            "Source Element",
            "The element to clone (format: surfaceId/elementId)",
            VariableType::String,
        );

        node.add_input_pin(
            "new_element_id",
            "New Element ID",
            "ID for the cloned element",
            VariableType::String,
        );

        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Container to add the cloned element to (optional, uses source parent if empty)",
            VariableType::String,
        );

        node.add_input_pin(
            "index",
            "Index",
            "Position in parent container (-1 for end)",
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "cloned_element_ref",
            "Cloned Element",
            "Reference to the cloned element",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source_ref: String = context.evaluate_pin("source_element").await?;
        let new_element_id: String = context.evaluate_pin("new_element_id").await?;
        let parent_id: String = context.evaluate_pin("parent_id").await.unwrap_or_default();
        let index: Option<i64> = context.evaluate_pin("index").await.ok();

        // Parse source reference (surfaceId/elementId)
        let parts: Vec<&str> = source_ref.split('/').collect();
        if parts.len() != 2 {
            return Err(flow_like_types::anyhow!(
                "Invalid source element reference: {}. Expected format: surfaceId/elementId",
                source_ref
            ));
        }
        let surface_id = parts[0];

        // Get source element from frontend elements
        let Some((_, source)) = context.read_element(&source_ref).await? else {
            return Err(flow_like_types::anyhow!(
                "Source element not found: {}",
                source_ref
            ));
        };

        // Extract component data
        let component_type = source
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let component_props = source
            .get("component")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        // Determine parent - use provided or source's parent
        let effective_parent = if parent_id.is_empty() {
            source
                .get("parentId")
                .and_then(|p| p.as_str())
                .map(String::from)
                .unwrap_or_default()
        } else {
            parent_id
        };

        // Build component value with type
        let component_value = if let Some(obj) = component_props.as_object() {
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
            flow_like::a2ui::SurfaceComponent::new(new_element_id.clone(), component_value);

        context
            .create_element(
                surface_id,
                &effective_parent,
                surface_component,
                index.map(|i| i as usize),
            )
            .await?;

        // Create new element reference
        let new_element_ref = format!("{}/{}", surface_id, new_element_id);

        context
            .set_pin_value("cloned_element_ref", Value::String(new_element_ref))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
