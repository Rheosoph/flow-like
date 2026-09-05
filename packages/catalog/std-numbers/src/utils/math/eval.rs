#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[crate::register_node]
#[derive(Default)]
pub struct EvalNode {}

impl EvalNode {
    pub fn new() -> Self {
        EvalNode {}
    }
}

#[async_trait]
impl NodeLogic for EvalNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "eval",
            "Evaluate Expression",
            "Evaluates a mathematical expression",
            "Math",
        );
        node.set_flowscript_name("math", "eval");
        node.add_icon("/flow/icons/calculator.svg");

        node.add_input_pin(
            "expression",
            "Expression",
            "Mathematical expression",
            VariableType::String,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Result of the expression",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let expression: String = context.evaluate_pin("expression").await?;
        let mut ns = fasteval::EmptyNamespace;
        let result = match fasteval::ez_eval(expression.as_str(), &mut ns) {
            Ok(result) => result,
            Err(e) => {
                let error: &str = &format!("Error evaluating expression: {}", e);
                context.log_message(error, LogLevel::Error);
                0.0 // Or another appropriate default value
            }
        };

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This feature requires the 'execute' feature"
        ))
    }
}
