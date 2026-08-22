use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BoolToggleNode {}

impl BoolToggleNode {
    pub fn new() -> Self {
        BoolToggleNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolToggleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bool_toggle",
            "Toggle (By Ref)",
            "Flips a boolean variable in place",
            "Utils/Bool",
        );
        node.add_icon("/flow/icons/bool.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);
        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "Reference to the boolean variable to flip",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        node.add_output_pin(
            "new_value",
            "New Value",
            "The value the variable holds after flipping",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let variable = context.get_variable(&var_ref).await?;
        let value_ref = variable.get_value();
        let mut guard = value_ref.lock().await;

        let new_value = match &*guard {
            Value::Bool(current) => !current,
            Value::Null => true,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Variable {var_ref} holds {other}, which is not a boolean"
                ));
            }
        };
        *guard = Value::Bool(new_value);
        drop(guard);

        context.set_pin_value("new_value", json!(new_value)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
