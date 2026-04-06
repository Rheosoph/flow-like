use super::element_utils::extract_element_id;
use super::update_schemas::{
    BarChartStyle, GenericChartStyle, LineChartStyle, PieChartStyle, RadarChartStyle,
};
use flow_like::a2ui::components::NivoChartProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

/// Unified Nivo chart styling node.
///
/// Configure any Nivo chart type's appearance with a single node.
/// The input pins change dynamically based on the selected chart type.
///
/// **Supported Chart Types:**
/// - Bar: layout, groupMode, padding, borderRadius, enableLabel, enableGrid
/// - Line: curve, lineWidth, enableArea, enablePoints, pointSize
/// - Pie: innerRadius, padAngle, cornerRadius, startAngle, endAngle
/// - Radar: gridShape, gridLevels, fillOpacity, borderWidth, enableDots
/// - Other: Generic style object for heatmap, calendar, sankey, etc.
#[crate::register_node]
#[derive(Default)]
pub struct SetChartStyle;

impl SetChartStyle {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetChartStyle {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_chart_style",
            "Set Chart Style",
            "Configure Nivo chart appearance",
            "UI/Elements/Charts",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Chart",
            "Reference to the NivoChart element",
            VariableType::Struct,
        )
        .set_schema::<NivoChartProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "chart_type",
            "Chart Type",
            "Type of chart to style",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Bar".to_string(),
                    "Line".to_string(),
                    "Pie".to_string(),
                    "Radar".to_string(),
                    "Other".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Bar")));

        // Default to Bar style input
        node.add_input_pin(
            "bar_style",
            "Bar Style",
            "Bar chart styling options",
            VariableType::Struct,
        )
        .set_schema::<BarChartStyle>();

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let chart_type: String = context.evaluate_pin("chart_type").await?;

        let (style_key, style_value) = match chart_type.as_str() {
            "Bar" => {
                let style: BarChartStyle = context.evaluate_pin("bar_style").await?;
                ("barStyle", json!(style))
            }
            "Line" => {
                let style: LineChartStyle = context.evaluate_pin("line_style").await?;
                ("lineStyle", json!(style))
            }
            "Pie" => {
                let style: PieChartStyle = context.evaluate_pin("pie_style").await?;
                ("pieStyle", json!(style))
            }
            "Radar" => {
                let style: RadarChartStyle = context.evaluate_pin("radar_style").await?;
                ("radarStyle", json!(style))
            }
            "Other" => {
                let style_type: String = context.evaluate_pin("style_type").await?;
                let style: GenericChartStyle = context.evaluate_pin("generic_style").await?;
                let key = format!("{}Style", style_type.to_lowercase());
                // Return early with dynamic key
                let update = json!({ key: style });
                context.upsert_element(&element_id, update).await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unknown chart type: {}",
                    chart_type
                ));
            }
        };

        let update = json!({ style_key: style_value });
        context.upsert_element(&element_id, update).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let chart_type = node
            .get_pin_by_name("chart_type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Bar".to_string());

        let bar_pin = node.get_pin_by_name("bar_style").cloned();
        let line_pin = node.get_pin_by_name("line_style").cloned();
        let pie_pin = node.get_pin_by_name("pie_style").cloned();
        let radar_pin = node.get_pin_by_name("radar_style").cloned();
        let style_type_pin = node.get_pin_by_name("style_type").cloned();
        let generic_pin = node.get_pin_by_name("generic_style").cloned();

        // Helper to remove all style pins
        let remove_all = |node: &mut Node,
                          bar: Option<_>,
                          line: Option<_>,
                          pie: Option<_>,
                          radar: Option<_>,
                          style_type: Option<_>,
                          generic: Option<_>| {
            remove_pin(node, bar);
            remove_pin(node, line);
            remove_pin(node, pie);
            remove_pin(node, radar);
            remove_pin(node, style_type);
            remove_pin(node, generic);
        };

        match chart_type.as_str() {
            "Bar" => {
                remove_all(
                    node,
                    None,
                    line_pin,
                    pie_pin,
                    radar_pin,
                    style_type_pin,
                    generic_pin,
                );
                if bar_pin.is_none() {
                    node.add_input_pin(
                        "bar_style",
                        "Bar Style",
                        "Bar chart styling options",
                        VariableType::Struct,
                    )
                    .set_schema::<BarChartStyle>();
                }
            }
            "Line" => {
                remove_all(
                    node,
                    bar_pin,
                    None,
                    pie_pin,
                    radar_pin,
                    style_type_pin,
                    generic_pin,
                );
                if line_pin.is_none() {
                    node.add_input_pin(
                        "line_style",
                        "Line Style",
                        "Line chart styling options",
                        VariableType::Struct,
                    )
                    .set_schema::<LineChartStyle>();
                }
            }
            "Pie" => {
                remove_all(
                    node,
                    bar_pin,
                    line_pin,
                    None,
                    radar_pin,
                    style_type_pin,
                    generic_pin,
                );
                if pie_pin.is_none() {
                    node.add_input_pin(
                        "pie_style",
                        "Pie Style",
                        "Pie/donut chart styling options",
                        VariableType::Struct,
                    )
                    .set_schema::<PieChartStyle>();
                }
            }
            "Radar" => {
                remove_all(
                    node,
                    bar_pin,
                    line_pin,
                    pie_pin,
                    None,
                    style_type_pin,
                    generic_pin,
                );
                if radar_pin.is_none() {
                    node.add_input_pin(
                        "radar_style",
                        "Radar Style",
                        "Radar chart styling options",
                        VariableType::Struct,
                    )
                    .set_schema::<RadarChartStyle>();
                }
            }
            "Other" => {
                remove_all(node, bar_pin, line_pin, pie_pin, radar_pin, None, None);
                if style_type_pin.is_none() {
                    node.add_input_pin(
                        "style_type",
                        "Style Type",
                        "Chart type name (heatmap, calendar, sankey, etc.)",
                        VariableType::String,
                    )
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec![
                                "heatmap".to_string(),
                                "calendar".to_string(),
                                "sankey".to_string(),
                                "scatter".to_string(),
                                "treemap".to_string(),
                                "funnel".to_string(),
                                "chord".to_string(),
                            ])
                            .build(),
                    )
                    .set_default_value(Some(json!("heatmap")));
                }
                if generic_pin.is_none() {
                    node.add_input_pin(
                        "generic_style",
                        "Style",
                        "Chart style configuration",
                        VariableType::Struct,
                    )
                    .set_schema::<GenericChartStyle>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }
            _ => {}
        }
    }
}
