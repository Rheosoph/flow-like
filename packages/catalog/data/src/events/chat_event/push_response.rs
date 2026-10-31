use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{history::Content, response::Response};
use flow_like_types::{async_trait, json::json};

use super::CachedChatResponse;

#[crate::register_node]
#[derive(Default)]
pub struct PushResponseNode {}

impl PushResponseNode {
    pub fn new() -> Self {
        PushResponseNode {}
    }
}

#[async_trait]
impl NodeLogic for PushResponseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_response",
            "Push Response",
            "Pushes a response to the chat",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "pushResponse");
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "response",
            "Response",
            "Chat Response",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "new_message",
            "New Message",
            "Start a new message. Later chat pushes belong to this message.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let response: Response = context.evaluate_pin("response").await?;
        // Older saved nodes do not have this pin until their catalog migration runs.
        let new_message = match context.get_pin_by_name("new_message").await {
            Ok(pin) => context.evaluate_pin_ref::<bool>(pin).await?,
            Err(_) => false,
        };
        let (summary, level) = describe_response(&response);
        context.log_message(&summary, level);

        let cached_response = CachedChatResponse::load(context).await?;
        let _message_guard = cached_response.emission.lock().await;
        let current = cached_response
            .replace_response(response, new_message)
            .await;

        context.stream_response("chat_stream", current).await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}

/// Sizes only, never the text: tells "the model sent nothing" apart from
/// "the chat lost it" without copying the answer into the run log.
fn describe_response(response: &Response) -> (String, LogLevel) {
    let Some(choice) = response
        .choices
        .iter()
        .rev()
        .find(|choice| choice.message.role == "assistant")
    else {
        let roles: Vec<&str> = response
            .choices
            .iter()
            .map(|choice| choice.message.role.as_str())
            .collect();
        return (
            format!(
                "Pushed a response without an assistant message ({} choices, roles {roles:?}); the chat shows no answer.",
                response.choices.len()
            ),
            LogLevel::Warn,
        );
    };

    let message = &choice.message;
    let text: String = message
        .ordered_content_parts()
        .iter()
        .filter_map(|part| match part {
            Content::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text_chars = text.chars().count();
    let visible_chars = strip_think_blocks(&text).trim().chars().count();
    let details = format!(
        "text: {text_chars} chars, outside <think>: {visible_chars} chars, reasoning: {} chars, tool calls: {}, finish_reason: {:?}",
        message
            .reasoning
            .as_deref()
            .map_or(0, |r| r.chars().count()),
        message.tool_calls.len(),
        choice.finish_reason,
    );

    if visible_chars == 0 {
        (
            format!("Pushed a response with no visible answer text ({details})."),
            LogLevel::Warn,
        )
    } else {
        (format!("Pushed a response ({details})."), LogLevel::Info)
    }
}

/// Mirrors the chat UI: `<think>` blocks render as reasoning, and an
/// unterminated `<think>` hides everything after it.
fn strip_think_blocks(text: &str) -> String {
    let mut visible = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        visible.push_str(&rest[..start]);
        let after_open = &rest[start + "<think>".len()..];
        let Some(end) = after_open.find("</think>") else {
            return visible;
        };
        rest = &after_open[end + "</think>".len()..];
    }
    visible.push_str(rest);
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_message_defaults_to_false_and_migrates_existing_nodes() {
        let node = PushResponseNode::new().get_node();
        assert_eq!(node.version, Some(1));
        let pin = node.get_pin_by_name("new_message").unwrap();
        assert_eq!(pin.data_type, VariableType::Boolean);
        assert_eq!(pin.effective_default(&Default::default()), json!(false));
    }

    #[test]
    fn unterminated_think_hides_the_rest() {
        assert_eq!(strip_think_blocks("a<think>b</think>c<think>d"), "ac");
        assert_eq!(strip_think_blocks("plain"), "plain");
    }

    #[test]
    fn empty_or_think_only_answers_warn() {
        assert!(matches!(
            describe_response(&Response::new()).1,
            LogLevel::Warn
        ));
        assert!(matches!(
            describe_response(&Response::from_text("<think>plan", "m")).1,
            LogLevel::Warn
        ));
        assert!(matches!(
            describe_response(&Response::from_text("<think>plan</think>Hi", "m")).1,
            LogLevel::Info
        ));
    }
}
