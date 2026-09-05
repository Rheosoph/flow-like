use super::element_utils::extract_element_id;
use flow_like::a2ui::components::PlotlyChartProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the layout configuration for a Plotly chart element.
#[crate::register_node]
#[derive(Default)]
pub struct SetChartLayout;

impl SetChartLayout {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChartLayout {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_chart_layout",
            "Set Chart Layout",
            "Sets the layout configuration for a Plotly chart",
            "UI/Elements/Charts",
        );
        node.set_flowscript_name("ui", "setChartLayout");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Chart",
            "Reference to the chart element (ID or element object)",
            VariableType::Struct,
        )
        .set_schema::<PlotlyChartProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "layout",
            "Layout",
            "Chart layout object (Plotly layout format)",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let layout: Value = context.evaluate_pin("layout").await?;

        let update_value = json!({
            "type": "setChartLayout",
            "layout": layout
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
