use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CellToParentNode {}

impl CellToParentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellToParentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cell_to_parent",
            "H3 Cell Parent",
            "Returns the parent cell at a coarser resolution. The parent contains the given cell.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "cellToParent");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin("cell", "Cell", "H3 cell index", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "parent_resolution",
            "Parent Resolution",
            "Target resolution for the parent (must be lower than cell's resolution)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_output_pin(
            "parent",
            "Parent",
            "Parent H3 cell index at the specified resolution",
            VariableType::String,
        );

        node.add_output_pin(
            "original_resolution",
            "Original Resolution",
            "Resolution of the input cell",
            VariableType::Integer,
        );

        node.set_long_running(false);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
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
        let parent_res: i64 = context.evaluate_pin("parent_resolution").await?;
        let parent_res = parent_res.clamp(0, 15) as u8;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let original_res = cell.resolution();

        let target_res = Resolution::try_from(parent_res)
            .map_err(|e| flow_like_types::anyhow!("Invalid resolution: {}", e))?;

        let parent = cell.parent(target_res).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Cannot get parent: target resolution {} must be lower than cell resolution {}",
                parent_res,
                u8::from(original_res)
            )
        })?;

        context
            .set_pin_value("parent", json!(parent.to_string()))
            .await?;
        context
            .set_pin_value("original_resolution", json!(u8::from(original_res) as i64))
            .await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
