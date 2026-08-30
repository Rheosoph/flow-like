use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CellToChildrenNode {}

impl CellToChildrenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellToChildrenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cell_to_children",
            "H3 Cell Children",
            "Returns all child cells at a finer resolution that fit within the given cell.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "cellToChildren");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin("cell", "Cell", "H3 cell index", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "child_resolution",
            "Child Resolution",
            "Target resolution for children (must be higher than cell's resolution)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "children",
            "Children",
            "Array of child H3 cell indices",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of child cells",
            VariableType::Integer,
        );

        node.set_long_running(false);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use h3o::{CellIndex, Resolution};
        use std::str::FromStr;

        let cell_str: String = context.evaluate_pin("cell").await?;
        let child_res: i64 = context.evaluate_pin("child_resolution").await?;
        let child_res = child_res.clamp(0, 15) as u8;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let target_res = Resolution::try_from(child_res)
            .map_err(|e| flow_like_types::anyhow!("Invalid resolution: {}", e))?;

        let children: Vec<String> = cell.children(target_res).map(|c| c.to_string()).collect();

        let count = children.len() as i64;

        context.set_pin_value("children", json!(children)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
