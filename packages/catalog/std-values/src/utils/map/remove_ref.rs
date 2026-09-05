use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RemoveMapRefNode {}

impl RemoveMapRefNode {
    pub fn new() -> Self {
        RemoveMapRefNode {}
    }
}

#[async_trait]
impl NodeLogic for RemoveMapRefNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_remove_ref",
            "Remove Key (By Ref)",
            "Remove a key directly from a variable map without copying. Much faster for large maps.",
            "Utils/Map/By Reference",
        );
        node.set_flowscript_name("map", "removeRef");
        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "Reference to the map variable to modify",
            VariableType::String,
        );

        node.add_input_pin("key", "Key", "Key to remove", VariableType::String);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "value",
            "Value",
            "The removed value (null if key not found)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "was_present",
            "Was Present",
            "Was the key in the map?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let key: String = context.evaluate_pin("key").await?;

        let variable = context.get_variable(&var_ref).await?;
        let value_ref = variable.get_value();
        let mut guard = value_ref.lock().await;

        let removed = match &mut *guard {
            Value::Object(map) => map.remove(&key),
            Value::Null => None,
            _ => {
                bail!("Variable is not a map");
            }
        };

        let was_present = removed.is_some();
        context
            .set_pin_value("value", removed.unwrap_or(Value::Null))
            .await?;
        context
            .set_pin_value("was_present", json!(was_present))
            .await?;
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
                node.error = Some("No map variable selected".to_string());
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

        if variable.value_type != ValueType::HashMap {
            node.error = Some(format!(
                "Variable '{}' is not a map (type: {:?})",
                variable.name, variable.value_type
            ));
        }
    }
}
