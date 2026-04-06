use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::sync::Arc;

/// Gets a value from page-local state by key.
///
/// Page state is scoped to the current page and cleared on navigation.
/// The state is passed in the workflow payload as `_page_state`.
#[crate::register_node]
#[derive(Default)]
pub struct GetPageState;

impl GetPageState {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetPageState {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_page_state",
            "Get Page State",
            "Gets a value from page-local state by key",
            "UI/State",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "key",
            "Key",
            "The key to retrieve from page state",
            VariableType::String,
        );

        node.add_output_pin(
            "value",
            "Value",
            "The value stored at the key",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the key exists in page state",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let key: String = context.evaluate_pin("key").await?;

        let payload = context.get_payload().await?;

        let page_state = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_page_state"))
            .and_then(|s| s.as_object());

        let value = page_state.and_then(|s| s.get(&key));

        if let Some(v) = value {
            context
                .get_pin_by_name("value")
                .await?
                .set_value(v.clone())
                .await;
            context
                .get_pin_by_name("exists")
                .await?
                .set_value(Value::Bool(true))
                .await;
            context.log_message(&format!("Got page state: {}", key), LogLevel::Debug);
        } else {
            context
                .get_pin_by_name("value")
                .await?
                .set_value(Value::Null)
                .await;
            context
                .get_pin_by_name("exists")
                .await?
                .set_value(Value::Bool(false))
                .await;
            context.log_message(
                &format!("Page state key not found: {}", key),
                LogLevel::Debug,
            );
        }

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
            node.friendly_name = format!("Get Page '{}'", k);
        } else {
            node.friendly_name = "Get Page State".to_string();
        }
    }
}
