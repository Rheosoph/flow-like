use flow_like::flow::pin::ValueType;
use flow_like::{
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    state::ToastLevel,
};
use flow_like_types::{Value, async_trait};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct InfoNode {}

impl InfoNode {
    pub fn new() -> Self {
        InfoNode {}
    }
}

#[async_trait]
impl NodeLogic for InfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "log_info",
            "Print Info",
            "Print Debugging Information",
            "Logging",
        );
        node.add_icon("/flow/icons/log-info.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "message",
            "Message",
            "The message to log",
            VariableType::Generic,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            "toast",
            "On Screen?",
            "Should the user see a toast popping up?",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "The flow to follow if the condition is true",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&output).await?;

        let should_toast = context.evaluate_pin::<bool>("toast").await?;
        let message = context.evaluate_pin::<Value>("message").await?;

        let string_message = match message {
            Value::String(s) => s,
            other => flow_like_types::json::to_string(&other)
                .unwrap_or_else(|_| "<unserializable value>".to_string()),
        };

        if should_toast {
            context
                .toast_message(&string_message, ToastLevel::Info)
                .await?;
        }

        context.log_message(&string_message, LogLevel::Info);
        context.activate_exec_pin_ref(&output).await?;

        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("message", board, None, Some(ValueType::Normal));
    }
}
