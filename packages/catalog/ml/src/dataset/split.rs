use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::arrow_utils::record_batch_to_value;
#[cfg(feature = "execute")]
use flow_like_storage::lancedb::query::ExecutableQuery;
#[cfg(feature = "execute")]
use flow_like_types::rand::{self, Rng};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;

#[crate::register_node]
#[derive(Default)]
pub struct SplitDatasetNode {}

impl SplitDatasetNode {
    pub fn new() -> Self {
        SplitDatasetNode {}
    }
}

#[async_trait]
impl NodeLogic for SplitDatasetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_dataset_split",
            "Split Dataset",
            "Split a dataset into training and testing subsets",
            "AI/ML/Dataset",
        );
        node.set_flowscript_name("ml", "splitDataset");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts the split",
            VariableType::Execution,
        );

        node.add_input_pin(
            "split",
            "Split",
            "Ratio used for assigning rows to the training set (rest goes to test)",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "source",
            "Data Source",
            "Data Source (DB or CSV)",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "train",
            "Training Database",
            "Destination database connection that receives the training rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "test",
            "Test Database",
            "Destination database connection that receives the testing rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the split has finished",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let source: NodeDBConnection = context.evaluate_pin("source").await?;
        let test: NodeDBConnection = context.evaluate_pin("test").await?;
        let train: NodeDBConnection = context.evaluate_pin("train").await?;
        let probability: f64 = context.evaluate_pin("split").await?;

        let source = source.load(context).await?;
        let test = test.load(context).await?;
        let train = train.load(context).await?;

        source.ensure_flushed().await?;
        let source_guard = source.db.read().await;
        let source_table = source_guard.inner().raw().await?;
        let query = source_table.query();
        let mut item_stream = query.execute().await?;

        while let Ok(Some(items)) = item_stream.try_next().await {
            let items = record_batch_to_value(&items)?;

            let mut train_items = Vec::with_capacity(items.len());
            let mut test_items = Vec::with_capacity(items.len());

            {
                let mut rng = rand::rng();

                for item in items {
                    let random_bool = rng.random_bool(probability);

                    if random_bool {
                        train_items.push(item);
                    } else {
                        test_items.push(item);
                    }
                }
            }

            if !train_items.is_empty() {
                train.insert_from(context, train_items).await?;
            }
            if !test_items.is_empty() {
                test.insert_from(context, test_items).await?;
            }
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}
