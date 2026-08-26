use super::element_utils::extract_element_id;
use flow_like::a2ui::components::NivoChartProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Sets the configuration for a Nivo chart element.
///
/// Allows full customization of chart appearance including colors,
/// margins, axes, legends, and chart-type-specific options.
#[crate::register_node]
#[derive(Default)]
pub struct SetNivoConfig;

impl SetNivoConfig {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetNivoConfig {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_nivo_config",
            "Set Nivo Chart Config",
            "Sets configuration options for a Nivo chart",
            "UI/Elements/Charts",
        );
        node.set_flowscript_name("ui", "setNivoConfig");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Chart",
            "Reference to the Nivo chart element",
            VariableType::Struct,
        )
        .set_schema::<NivoChartProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "config",
            "Config",
            "Full Nivo configuration object (merged with defaults)",
            VariableType::Generic,
        );

        node.add_input_pin(
            "chart_type",
            "Chart Type",
            "Chart type (bar, line, pie, radar, etc.)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "colors",
            "Colors",
            "Color scheme name or array of colors",
            VariableType::Generic,
        )
        .set_default_value(Some(json!(null)));

        node.add_input_pin(
            "height",
            "Height",
            "Chart height (e.g., '400px')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let config: Value = context.evaluate_pin("config").await.unwrap_or(Value::Null);
        let chart_type: String = context.evaluate_pin("chart_type").await.unwrap_or_default();
        let colors: Value = context.evaluate_pin("colors").await.unwrap_or(Value::Null);
        let height: String = context.evaluate_pin("height").await.unwrap_or_default();

        let mut update = json!({
            "type": "setNivoConfig"
        });

        if !config.is_null() {
            update["config"] = config;
        }
        if !chart_type.is_empty() {
            update["chartType"] = json!(chart_type);
        }
        if !colors.is_null() {
            update["colors"] = colors;
        }
        if !height.is_empty() {
            update["height"] = json!(height);
        }

        context.upsert_element(&element_id, update).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
