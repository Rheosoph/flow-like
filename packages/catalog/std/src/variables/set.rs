use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::schemas_are_compatible,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::{collections::HashMap, sync::Arc};

#[crate::register_node]
#[derive(Default)]
pub struct SetVariable {}

impl SetVariable {
    pub fn new() -> Self {
        SetVariable {}
    }

    pub fn push_registry(registry: &mut HashMap<&'static str, Arc<dyn NodeLogic>>) {
        let node = SetVariable::new();
        let node = Arc::new(node);
        registry.insert("variable_set", node);
    }
}

#[async_trait]
impl NodeLogic for SetVariable {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "variable_set",
            "Set Variable",
            "Set Variable Value",
            "Variable",
        );
        node.set_flowscript_name("variable", "set");

        node.add_icon("/flow/icons/variable.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "The reference to the variable",
            VariableType::String,
        );

        node.add_input_pin(
            "value_in",
            "Value",
            "The value of the variable",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Triggering once the variable value was set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "value_ref",
            "New Value",
            "The newly set value",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.deactivate_exec_pin_ref(&exec_out).await?;

        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let value = context.evaluate_pin_to_ref("value_in").await?;

        context.set_variable_value(&var_ref, value.clone()).await?;
        context.set_pin_value("value_ref", value).await?;
        context.activate_exec_pin_ref(&exec_out).await?;
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

        let expected_name = format!("Set {}", var_ref_variable.name);

        // Check if value_in pin needs updating
        let value_in = read_only_node.get_pin_by_name("value_in");
        let value_ref = read_only_node.get_pin_by_name("value_ref");

        let value_in_changed = value_in.is_some_and(|pin| {
            pin.data_type != var_ref_variable.data_type
                || pin.value_type != var_ref_variable.value_type
                || pin.schema != var_ref_variable.schema
        });

        let value_ref_changed = value_ref.is_some_and(|pin| {
            pin.data_type != var_ref_variable.data_type
                || pin.value_type != var_ref_variable.value_type
                || pin.schema != var_ref_variable.schema
        });

        let name_changed = node.friendly_name != expected_name;

        if !value_in_changed && !value_ref_changed && !name_changed {
            return;
        }

        node.friendly_name = expected_name;

        let mut_value = match node.get_pin_mut_by_name("value_in") {
            Some(val) => val,
            None => {
                node.error = Some("Value pin not found!".to_string());
                return;
            }
        };
        mut_value.data_type = var_ref_variable.data_type.clone();
        mut_value.value_type = var_ref_variable.value_type.clone();
        mut_value.schema = var_ref_variable.schema.clone();
        if !mut_value.depends_on.is_empty() {
            let mut dependencies = mut_value.depends_on.clone();
            dependencies.retain(|deps| {
                board.get_pin_by_id(deps).is_some_and(|pin| {
                    if pin.data_type != mut_value.data_type
                        || pin.value_type != mut_value.value_type
                    {
                        return false;
                    }
                    schemas_are_compatible(mut_value.schema.as_deref(), pin.schema.as_deref())
                })
            });

            mut_value.depends_on = dependencies;
        }

        let mut_new_value = match node.get_pin_mut_by_name("value_ref") {
            Some(val) => val,
            None => {
                node.error = Some("New Value pin not found!".to_string());
                return;
            }
        };

        mut_new_value.data_type = var_ref_variable.data_type.clone();
        mut_new_value.value_type = var_ref_variable.value_type.clone();
        mut_new_value.schema = var_ref_variable.schema.clone();

        if !mut_new_value.connected_to.is_empty() {
            let mut connected = mut_new_value.connected_to.clone();

            connected.retain(|conn| {
                board.get_pin_by_id(conn).is_some_and(|pin| {
                    if pin.data_type != mut_new_value.data_type
                        || pin.value_type != mut_new_value.value_type
                    {
                        return false;
                    }
                    schemas_are_compatible(mut_new_value.schema.as_deref(), pin.schema.as_deref())
                })
            });

            mut_new_value.connected_to = connected;
        }
    }
}
