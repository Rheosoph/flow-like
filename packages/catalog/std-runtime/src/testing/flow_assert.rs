use flow_like::flow::pin::ValueType;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct FlowAssertNode {}

impl FlowAssertNode {
    pub fn new() -> Self {
        FlowAssertNode {}
    }
}

#[async_trait]
impl NodeLogic for FlowAssertNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "flow_assert",
            "Assert",
            "Asserts a condition inside a flow. On pass it logs `ASSERT_OK {label}` (Info) and continues; on fail it logs `ASSERT_FAIL {label} {details}` (Error) and halts the run with an error. Test runners grep these stable marker prefixes. Name test events with a `test` prefix so they are discoverable by test tooling.",
            "Utils/Testing",
        );
        node.set_flowscript_name("test", "assert");
        node.add_icon("/flow/icons/shield.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "condition",
            "Condition",
            "The condition that must hold.",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(false)));

        node.add_input_pin(
            "label",
            "Label",
            "Stable name for this assertion, echoed in the ASSERT_OK/ASSERT_FAIL log markers.",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("assertion")));

        node.add_input_pin(
            "details",
            "Details",
            "Optional context logged when the assertion fails.",
            VariableType::Generic,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_output_pin(
            "exec_out",
            "Pass",
            "Continues only when the assertion holds.",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&output).await?;

        let condition = context.evaluate_pin::<bool>("condition").await?;
        let label = context.evaluate_pin::<String>("label").await?;
        let details = context.evaluate_pin::<Value>("details").await?;

        if condition {
            context.log_message(&format!("ASSERT_OK {label}"), LogLevel::Info);
            context.activate_exec_pin_ref(&output).await?;
            return Ok(());
        }

        let details_string = match details {
            Value::String(s) => s,
            other => flow_like_types::json::to_string(&other)
                .unwrap_or_else(|_| "<unserializable value>".to_string()),
        };

        context.log_message(
            &format!("ASSERT_FAIL {label} {details_string}"),
            LogLevel::Error,
        );
        Err(flow_like_types::anyhow!(
            "ASSERT_FAIL {label}: {details_string}"
        ))
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("details", board, None, Some(ValueType::Normal));
    }
}
