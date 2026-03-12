use flow_like::{
    a2ui::CanvasSettings,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetSurfaceCustomCss;

impl SetSurfaceCustomCss {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetSurfaceCustomCss {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_surface_custom_css",
            "Set Surface Custom CSS",
            "Sets or clears scoped custom CSS for a custom UI surface at runtime",
            "UI/Surface",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "ID of the custom UI surface to update",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));

        node.add_input_pin(
            "custom_css",
            "Custom CSS",
            "CSS to apply to the surface. Leave empty to clear it.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let custom_css = context
            .evaluate_pin::<String>("custom_css")
            .await
            .ok()
            .and_then(|css| {
                let trimmed = css.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            });

        context
            .stream_a2ui_set_canvas_settings(
                &surface_id,
                CanvasSettings {
                    custom_css: custom_css,
                    ..Default::default()
                },
            )
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
