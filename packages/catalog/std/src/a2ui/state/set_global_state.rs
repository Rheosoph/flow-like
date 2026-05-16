use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Sets a value in global state by key.
///
/// Global state is shared across all pages and persists during the session.
/// Streams a `setGlobalState` message to the frontend.
#[crate::register_node]
#[derive(Default)]
pub struct SetGlobalState;

impl SetGlobalState {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetGlobalState {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_global_state",
            "Set Global State",
            "Sets a value in global state by key",
            "UI/State",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );

        node.add_input_pin(
            "key",
            "Key",
            "The key to store the value at",
            VariableType::String,
        );

        node.add_input_pin(
            "value",
            "Value",
            "The value to store",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.activate_exec_pin("exec_out").await?;

        let key: String = context.evaluate_pin("key").await?;
        let value: Value = context.evaluate_pin("value").await?;

        context.set_global_state(&key, value.clone()).await?;

        context.log_message(
            &format!("Set global state '{}' = {:?}", key, value),
            LogLevel::Debug,
        );

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let key_pin = match read_only_node.get_pin_by_name("key") {
            Some(pin) => pin,
            None => return,
        };

        let key = key_pin.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        });

        if let Some(k) = key {
            node.friendly_name = format!("Set Global '{}'", k);
        } else {
            node.friendly_name = "Set Global State".to_string();
        }
    }
}
