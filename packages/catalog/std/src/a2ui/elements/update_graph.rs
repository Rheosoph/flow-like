use super::element_utils::extract_element_id;
use super::update_schemas::{GraphEdgeInput, GraphLabelStyle, GraphNodeInput};
use flow_like::a2ui::components::GraphProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified Graph update node.
///
/// Replaces the data of a graph element. The input pins change dynamically
/// based on the selected property.
///
/// **Properties:**
/// - Nodes: Array of nodes with id, label, caption, props
/// - Edges: Array of edges with id, source, target, label, props
/// - Label Styles: Per-label color, icon and size
#[crate::register_node]
#[derive(Default)]
pub struct UpdateGraph;

impl UpdateGraph {
    pub fn new() -> Self {
        Self
    }
}

fn add_nodes_pin(node: &mut Node) {
    node.add_input_pin(
        "nodes",
        "Nodes",
        "Array of graph nodes",
        VariableType::Struct,
    )
    .set_value_type(flow_like::flow::pin::ValueType::Array)
    .set_schema::<GraphNodeInput>()
    .set_options(PinOptions::new().set_enforce_schema(false).build());
}

fn add_edges_pin(node: &mut Node) {
    node.add_input_pin(
        "edges",
        "Edges",
        "Array of graph edges",
        VariableType::Struct,
    )
    .set_value_type(flow_like::flow::pin::ValueType::Array)
    .set_schema::<GraphEdgeInput>()
    .set_options(PinOptions::new().set_enforce_schema(false).build());
}

fn add_label_styles_pin(node: &mut Node) {
    node.add_input_pin(
        "label_styles",
        "Label Styles",
        "Per-label color, icon and size",
        VariableType::Struct,
    )
    .set_value_type(flow_like::flow::pin::ValueType::Array)
    .set_schema::<GraphLabelStyle>()
    .set_options(PinOptions::new().set_enforce_schema(false).build());
}

#[async_trait]
impl NodeLogic for UpdateGraph {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_graph",
            "Update Graph",
            "Update the nodes, edges or label styles of a graph",
            "UI/Elements/Graph",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Graph",
            "Reference to the graph element",
            VariableType::Struct,
        )
        .set_schema::<GraphProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "property",
            "Property",
            "Which property to update",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Nodes".to_string(),
                    "Edges".to_string(),
                    "Label Styles".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Nodes")));

        add_nodes_pin(&mut node);

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let property: String = context.evaluate_pin("property").await?;

        let update = match property.as_str() {
            "Nodes" => {
                let nodes: Value = context.evaluate_pin("nodes").await?;
                json!({
                    "type": "setGraphNodes",
                    "nodes": { "literalJson": flow_like_types::json::to_string(&nodes)? }
                })
            }
            "Edges" => {
                let edges: Value = context.evaluate_pin("edges").await?;
                json!({
                    "type": "setGraphEdges",
                    "edges": { "literalJson": flow_like_types::json::to_string(&edges)? }
                })
            }
            "Label Styles" => {
                let styles: Value = context.evaluate_pin("label_styles").await?;
                json!({
                    "type": "setGraphLabelStyles",
                    "labelStyles": { "literalJson": flow_like_types::json::to_string(&styles)? }
                })
            }
            _ => return Err(flow_like_types::anyhow!("Unknown property: {}", property)),
        };

        context.upsert_element(&element_id, update).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let property = node
            .get_pin_by_name("property")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Nodes".to_string());

        let nodes_pin = node.get_pin_by_name("nodes").cloned();
        let edges_pin = node.get_pin_by_name("edges").cloned();
        let styles_pin = node.get_pin_by_name("label_styles").cloned();

        match property.as_str() {
            "Nodes" => {
                remove_pin(node, edges_pin);
                remove_pin(node, styles_pin);
                if nodes_pin.is_none() {
                    add_nodes_pin(node);
                } else if let Some(pin) = node.get_pin_mut_by_name("nodes") {
                    pin.set_schema::<GraphNodeInput>();
                }
            }
            "Edges" => {
                remove_pin(node, nodes_pin);
                remove_pin(node, styles_pin);
                if edges_pin.is_none() {
                    add_edges_pin(node);
                } else if let Some(pin) = node.get_pin_mut_by_name("edges") {
                    pin.set_schema::<GraphEdgeInput>();
                }
            }
            "Label Styles" => {
                remove_pin(node, nodes_pin);
                remove_pin(node, edges_pin);
                if styles_pin.is_none() {
                    add_label_styles_pin(node);
                } else if let Some(pin) = node.get_pin_mut_by_name("label_styles") {
                    pin.set_schema::<GraphLabelStyle>();
                }
            }
            _ => {}
        }
    }
}
