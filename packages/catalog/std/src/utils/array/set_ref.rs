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
pub struct SetIndexArrayRefNode {}

impl SetIndexArrayRefNode {
    pub fn new() -> Self {
        SetIndexArrayRefNode {}
    }
}

#[async_trait]
impl NodeLogic for SetIndexArrayRefNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_set_index_ref",
            "Set Index (By Ref)",
            "Set an element at a specific index directly in a variable array without copying. Much faster for large arrays.",
            "Utils/Array/By Reference",
        );
        node.set_flowscript_name("array", "setIndexRef");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "Reference to the array variable to modify",
            VariableType::String,
        );

        node.add_input_pin("index", "Index", "Index to set", VariableType::Integer);

        node.add_input_pin(
            "value",
            "Value",
            "Value to set at the index",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let index: usize = context.evaluate_pin("index").await?;
        let value: Value = context.evaluate_pin("value").await?;

        let variable = context.get_variable(&var_ref).await?;
        let value_ref = variable.get_value();
        let mut guard = value_ref.lock().await;

        match &mut *guard {
            Value::Array(arr) => {
                if index >= arr.len() {
                    bail!(
                        "Index {} out of bounds (array length: {})",
                        index,
                        arr.len()
                    );
                }
                arr[index] = value;
            }
            _ => {
                bail!("Variable is not an array");
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
                node.error = Some("No array variable selected".to_string());
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

        if variable.value_type != ValueType::Array {
            node.error = Some(format!(
                "Variable '{}' is not an array (type: {:?})",
                variable.name, variable.value_type
            ));
        }
    }
}
