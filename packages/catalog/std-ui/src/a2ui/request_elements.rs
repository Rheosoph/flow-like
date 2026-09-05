use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::time::Duration;

/// A page that has gone away must not hold an executor for longer than this.
const MAX_TIMEOUT_MS: i64 = 5 * 60 * 1000;

#[crate::register_node]
#[derive(Default)]
pub struct RequestElements;

impl RequestElements {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RequestElements {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_request_elements",
            "Request Elements",
            "Fetches elements from the live page in one round-trip so later reads hit the cache",
            "UI/Data",
        );
        node.set_flowscript_name("ui", "requestElements");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ids",
            "Selectors",
            "Element selectors, e.g. ['main/input-field', 'type:switch', 'glob:feed-row-*/subscribed', 'children:main/list', 'host:main/feed-row-1']",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "How long to wait for the page to answer",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(15000)));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let selectors: Vec<String> = context.evaluate_pin("element_ids").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await.unwrap_or(15_000);

        if !context.elements.read().await.on_demand() {
            context.log_message(
                "The client did not declare `_elements_mode: \"demand\"`; elements cannot be requested mid-run",
                LogLevel::Warn,
            );
        } else if !selectors.is_empty() {
            let fetched = context
                .request_elements(
                    selectors,
                    Duration::from_millis(timeout_ms.clamp(1, MAX_TIMEOUT_MS) as u64),
                )
                .await?;
            context.log_message(
                &format!("Fetched {} element(s) from the page", fetched.len()),
                LogLevel::Debug,
            );
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
