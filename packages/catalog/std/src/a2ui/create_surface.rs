use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CreateSurface;

impl CreateSurface {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateSurface {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_create_surface",
            "Create Surface",
            "Creates a new A2UI surface with an ID and root component",
            "UI/Surface",
        );
        node.set_flowscript_name("ui", "createSurface");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "Unique identifier for the surface",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));

        node.add_input_pin(
            "root_component_id",
            "Root Component ID",
            "ID of the root component in the surface",
            VariableType::String,
        )
        .set_default_value(Some(json!("root")));

        node.add_input_pin(
            "catalog_id",
            "Catalog ID",
            "Optional custom component catalog",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Normal);

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "surface",
            "Surface",
            "The created surface for adding components",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::Surface>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let root_component_id: String = context.evaluate_pin("root_component_id").await?;
        let catalog_id: Option<String> = context.evaluate_pin::<String>("catalog_id").await.ok();

        let mut surface = flow_like::a2ui::Surface::new(&surface_id, &root_component_id);
        if let Some(catalog) = catalog_id {
            surface = surface.with_catalog(catalog);
        }

        context.set_pin_value("surface", json!(surface)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
