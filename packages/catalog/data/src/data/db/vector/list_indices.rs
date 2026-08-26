use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::json::json;
use flow_like_types::{
    JsonSchema, async_trait,
    json::{Deserialize, Serialize},
};

use super::NodeDBConnection;

/// Stable metadata shape for Lance index descriptions. The execution result
/// uses the storage DTO with the same serialized fields, while metadata builds
/// avoid compiling the LanceDB implementation merely to describe this pin.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
struct IndexConfigMetadata {
    name: String,
    index_type: String,
    columns: Vec<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct ListIndicesNode {}

impl ListIndicesNode {
    pub fn new() -> Self {
        ListIndicesNode {}
    }
}

#[async_trait]
impl NodeLogic for ListIndicesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "list_indices_db",
            "List Indices",
            "Lists all indices on a database table",
            "Data/Database/Meta",
        );
        node.set_flowscript_name("db", "listIndices");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done listing indices",
            VariableType::Execution,
        );

        node.add_output_pin(
            "indices",
            "Indices",
            "List of indices on the table",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<IndexConfigMetadata>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        let indices = database.inner().list_indices().await?;

        context.set_pin_value("indices", json!(indices)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}
