use flow_like::{
    a2ui::{DataModel, Surface, SurfaceComponent},
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct BeginRendering;

impl BeginRendering {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for BeginRendering {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_begin_rendering",
            "Begin Rendering",
            "Sends a surface to the frontend to begin rendering",
            "UI/Surface",
        );
        node.set_flowscript_name("ui", "beginRendering");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface",
            "Surface",
            "The surface to render",
            VariableType::Struct,
        )
        .set_schema::<Surface>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "components",
            "Components",
            "Array of components to include",
            VariableType::Struct,
        )
        .set_schema::<SurfaceComponent>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "data_model",
            "Data Model",
            "Initial data model for bindings",
            VariableType::Struct,
        )
        .set_schema::<DataModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface: Surface = context.evaluate_pin("surface").await?;
        let components: Vec<SurfaceComponent> = context.evaluate_pin("components").await?;
        let data_model: DataModel = context
            .evaluate_pin::<DataModel>("data_model")
            .await
            .unwrap_or_default();

        let mut full_surface = surface;
        for component in components {
            full_surface.add_component(component);
        }

        context
            .stream_a2ui_begin_rendering(&full_surface, &data_model)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
