use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail};

#[crate::register_node]
#[derive(Default)]
pub struct ClearSetRefNode {}

impl ClearSetRefNode {
    pub fn new() -> Self {
        ClearSetRefNode {}
    }
}

#[async_trait]
impl NodeLogic for ClearSetRefNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_clear_ref",
            "Clear (By Ref)",
            "Clear all elements directly from a variable set without copying.",
            "Utils/Set/By Reference",
        );
        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "Reference to the set variable to clear",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let var_ref: String = context.evaluate_pin("var_ref").await?;

        let variable = context.get_variable(&var_ref).await?;
        let value_ref = variable.get_value();
        let mut guard = value_ref.lock().await;

        match &mut *guard {
            Value::Array(arr) => {
                arr.clear();
            }
            Value::Null => {}
            _ => {
                bail!("Variable is not a set");
            }
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let var_ref = match node.get_pin_by_name("var_ref") {
            Some(pin) => pin,
            None => return,
        };

        let var_ref_value = match var_ref.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        }) {
            Some(val) if !val.is_empty() => val,
            _ => {
                node.error = Some("No set variable selected".to_string());
                return;
            }
        };

        let variable = match board.get_any_variable(&var_ref_value) {
            Some(var) => var,
            None => {
                node.error = Some(format!("Variable '{}' not found", var_ref_value));
                return;
            }
        };

        if variable.value_type != ValueType::HashSet {
            node.error = Some(format!(
                "Variable '{}' is not a set (type: {:?})",
                variable.name, variable.value_type
            ));
        }
    }
}
