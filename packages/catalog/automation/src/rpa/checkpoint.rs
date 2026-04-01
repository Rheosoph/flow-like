use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SaveCheckpointNode {}

impl SaveCheckpointNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SaveCheckpointNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_save_checkpoint",
            "Save Checkpoint",
            "Creates checkpoint data for potential recovery",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "checkpoint_name",
            "Checkpoint Name",
            "Name to identify this checkpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("checkpoint_1")));

        node.add_input_pin(
            "data",
            "Data",
            "JSON string of data to save at checkpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("{}")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "checkpoint_data",
            "Checkpoint Data",
            "Complete checkpoint data as JSON",
            VariableType::String,
        );

        node.add_output_pin(
            "checkpoint_id",
            "Checkpoint ID",
            "Unique ID for this checkpoint",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use chrono::Utc;

        context.deactivate_exec_pin("exec_out").await?;

        let checkpoint_name: String = context.evaluate_pin("checkpoint_name").await?;
        let data: String = context.evaluate_pin("data").await?;

        let checkpoint_id = format!("{}_{}", checkpoint_name, flow_like_types::create_id());

        let checkpoint = flow_like_types::json::json!({
            "id": checkpoint_id,
            "name": checkpoint_name,
            "timestamp": Utc::now().to_rfc3339(),
            "data": data
        });

        context
            .set_pin_value("checkpoint_data", json!(checkpoint.to_string()))
            .await?;
        context
            .set_pin_value("checkpoint_id", json!(checkpoint_id))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ParseCheckpointNode {}

impl ParseCheckpointNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ParseCheckpointNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_parse_checkpoint",
            "Parse Checkpoint",
            "Parses checkpoint data from a saved JSON string",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "checkpoint_data",
            "Checkpoint Data",
            "Checkpoint JSON string to parse",
            VariableType::String,
        )
        .set_default_value(Some(json!("{}")));

        node.add_output_pin(
            "exec_valid",
            "Valid",
            "Checkpoint was valid",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_invalid",
            "Invalid",
            "Checkpoint was invalid",
            VariableType::Execution,
        );

        node.add_output_pin(
            "data",
            "Data",
            "Extracted data from checkpoint",
            VariableType::String,
        );
        node.add_output_pin("name", "Name", "Checkpoint name", VariableType::String);
        node.add_output_pin(
            "timestamp",
            "Timestamp",
            "When checkpoint was saved",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_valid").await?;
        context.deactivate_exec_pin("exec_invalid").await?;

        let checkpoint_data: String = context.evaluate_pin("checkpoint_data").await?;

        match flow_like_types::json::from_str::<flow_like_types::Value>(&checkpoint_data) {
            Ok(checkpoint) => {
                let data = checkpoint["data"].as_str().unwrap_or("{}").to_string();
                let name = checkpoint["name"].as_str().unwrap_or("").to_string();
                let timestamp = checkpoint["timestamp"].as_str().unwrap_or("").to_string();

                context.set_pin_value("data", json!(data)).await?;
                context.set_pin_value("name", json!(name)).await?;
                context.set_pin_value("timestamp", json!(timestamp)).await?;
                context.activate_exec_pin("exec_valid").await?;
            }
            Err(_) => {
                context.set_pin_value("data", json!("{}")).await?;
                context.set_pin_value("name", json!("")).await?;
                context.set_pin_value("timestamp", json!("")).await?;
                context.activate_exec_pin("exec_invalid").await?;
            }
        }

        Ok(())
    }
}
