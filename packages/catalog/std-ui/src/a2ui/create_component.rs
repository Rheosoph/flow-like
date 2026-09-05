use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CreateComponent;

impl CreateComponent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateComponent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_create_component",
            "Create Component",
            "Creates an A2UI component with ID, style, and component data",
            "UI/Component",
        );
        node.set_flowscript_name("ui", "createComponent");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "component_id",
            "Component ID",
            "Unique identifier for the component",
            VariableType::String,
        );

        node.add_input_pin(
            "component_type",
            "Type",
            "Component type (row, column, text, button, etc.)",
            VariableType::String,
        )
        .set_default_value(Some(json!("text")));

        node.add_input_pin(
            "props",
            "Props",
            "Component properties as JSON",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_input_pin(
            "style",
            "Style",
            "Optional style for the component",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::Style>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "component",
            "Component",
            "The created component",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::SurfaceComponent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let component_id: String = context.evaluate_pin("component_id").await?;
        let component_type: String = context.evaluate_pin("component_type").await?;
        let props: Value = context
            .evaluate_pin::<Value>("props")
            .await
            .unwrap_or(json!({}));
        let style: Option<flow_like::a2ui::Style> = context
            .evaluate_pin::<flow_like::a2ui::Style>("style")
            .await
            .ok();

        let mut component_data = json!({
            "type": component_type
        });

        if let Value::Object(prop_map) = props
            && let Value::Object(ref mut data_map) = component_data
        {
            for (key, value) in prop_map {
                data_map.insert(key, value);
            }
        }

        let mut component = flow_like::a2ui::SurfaceComponent::new(&component_id, component_data);
        if let Some(s) = style {
            component = component.with_style(s);
        }

        context.set_pin_value("component", json!(component)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
