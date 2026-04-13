use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::{Variable, VariableType},
};
use flow_like_types::{Value, async_trait};
use std::{collections::HashMap, sync::Arc};

#[crate::register_node]
#[derive(Default)]
pub struct GetVariable {}

impl GetVariable {
    pub fn new() -> Self {
        GetVariable {}
    }

    pub fn push_registry(registry: &mut HashMap<&'static str, Arc<dyn NodeLogic>>) {
        let node = GetVariable::new();
        let node = Arc::new(node);
        registry.insert("variable_get", node);
    }
}

#[async_trait]
impl NodeLogic for GetVariable {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "variable_get",
            "Get Variable",
            "Get Variable Value",
            "Variable",
        );

        node.add_icon("/flow/icons/variable.svg");

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "The reference to the variable",
            VariableType::String,
        );

        node.add_output_pin(
            "value_ref",
            "Value",
            "The value of the variable",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let variable: Variable = context.get_variable(&var_ref).await?;

        let value_pin = context.get_pin_by_name("value_ref").await?;
        let value = variable.get_value();
        let value_cloned = value.lock().await.clone();

        if variable.secret {
            context.log_message("Accessed secret variable value", LogLevel::Debug);
        } else {
            context.log_message(
                &format!("Accessed variable value: {:?}", value_cloned),
                LogLevel::Debug,
            );
        }

        value_pin.set_value(value_cloned).await;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let var_ref = match read_only_node.get_pin_by_name("var_ref") {
            Some(pin) => pin,
            None => {
                node.error = Some("Variable not found!".to_string());
                return;
            }
        };

        let var_ref_value = match var_ref.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).unwrap();
            parsed.as_str().map(String::from)
        }) {
            Some(val) => val,
            None => {
                node.error = Some("Variable reference not found!".to_string());
                return;
            }
        };

        let var_ref_variable = match board.get_any_variable(&var_ref_value) {
            Some(var) => var,
            None => {
                node.error = Some("Variable not found!".to_string());
                return;
            }
        };

        let expected_name = format!("Get {}", &var_ref_variable.name);

        // Check if anything changed using read_only_node to avoid borrow issues
        let value_pin = read_only_node.get_pin_by_name("value_ref");
        let type_changed = value_pin.is_some_and(|pin| {
            pin.data_type != var_ref_variable.data_type
                || pin.value_type != var_ref_variable.value_type
                || pin.schema != var_ref_variable.schema
        });
        let name_changed = read_only_node.friendly_name != expected_name;

        if !type_changed && !name_changed {
            return;
        }

        node.friendly_name = expected_name;

        let mut_value = match node.get_pin_mut_by_name("value_ref") {
            Some(val) => val,
            None => {
                node.error = Some("Value pin not found!".to_string());
                return;
            }
        };
        let immutable_value = mut_value.clone();

        mut_value.data_type = var_ref_variable.data_type.clone();
        mut_value.value_type = var_ref_variable.value_type.clone();
        mut_value.schema = var_ref_variable.schema.clone();

        if immutable_value.connected_to.is_empty() {
            return;
        }

        let mut connected = immutable_value.connected_to.clone();

        connected.retain(|conn| {
            board.get_pin_by_id(conn).is_some_and(|pin| {
                // Check type and value_type match
                if pin.data_type != mut_value.data_type || pin.value_type != mut_value.value_type {
                    return false;
                }
                // If both have schemas, they must match
                if mut_value.schema.is_some() && pin.schema.is_some() {
                    return mut_value.schema == pin.schema;
                }
                true
            })
        });

        mut_value.connected_to = connected;
    }
}
