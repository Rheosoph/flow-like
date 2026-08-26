use flow_like::{
    a2ui::SurfaceComponent,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SurfaceUpdate;

impl SurfaceUpdate {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SurfaceUpdate {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_surface_update",
            "Surface Update",
            "Updates components in an existing surface",
            "UI/Surface",
        );
        node.set_flowscript_name("ui", "surfaceUpdate");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "ID of the surface to update",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));

        node.add_input_pin(
            "components",
            "Components",
            "Components to add or update",
            VariableType::Struct,
        )
        .set_schema::<SurfaceComponent>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let components: Vec<SurfaceComponent> = context.evaluate_pin("components").await?;

        context
            .stream_a2ui_surface_update(&surface_id, components)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
