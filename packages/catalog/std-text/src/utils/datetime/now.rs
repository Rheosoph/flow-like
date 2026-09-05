use chrono::Utc;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateTimeNowNode {}

impl DateTimeNowNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DateTimeNowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_now",
            "Now",
            "Returns the current date and time in UTC",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "now");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);
        node.add_output_pin(
            "date",
            "Date",
            "Current UTC date and time",
            VariableType::Date,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let now = Utc::now();
        context.deactivate_exec_pin("exec_out").await?;
        context.set_pin_value("date", json!(now)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
