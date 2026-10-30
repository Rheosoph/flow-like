use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatReactionOperation {
    Add,
    Remove,
}

/// A workflow's reaction. An omitted ID targets the user message that started the run.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct ChatReaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub emoji: String,
    pub operation: ChatReactionOperation,
}

impl ChatReaction {
    fn new(
        message_id: String,
        emoji: String,
        operation: ChatReactionOperation,
    ) -> flow_like_types::Result<Self> {
        let emoji = emoji.trim();
        if emoji.is_empty() {
            return Err(anyhow!("Choose an emoji for the reaction"));
        }
        if emoji.encode_utf16().count() > 64 {
            return Err(anyhow!("The reaction exceeds the emoji length limit"));
        }
        let message_id = message_id.trim();
        Ok(Self {
            message_id: (!message_id.is_empty()).then(|| message_id.to_string()),
            emoji: emoji.to_string(),
            operation,
        })
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ReactNode {}

impl ReactNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReactNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_react",
            "React to Message",
            "Adds or removes a workflow emoji reaction on a chat message without starting another turn.",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "react");
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "emoji",
            "Emoji",
            "Emoji to add or remove",
            VariableType::String,
        );
        node.add_input_pin(
            "message_id",
            "Message ID",
            "Leave empty to react to the user message that started this run",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "operation",
            "Operation",
            "Add or remove the workflow's reaction",
            VariableType::String,
        )
        .set_default_value(Some(json!("add")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["add".to_string(), "remove".to_string()])
                .build(),
        );
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
        let emoji: String = context.evaluate_pin("emoji").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;
        let operation: ChatReactionOperation = context.evaluate_pin("operation").await?;
        let reaction = ChatReaction::new(message_id, emoji, operation)?;

        context.stream_response("chat_reaction", reaction).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_target_is_omitted_and_multi_codepoint_emojis_are_preserved() {
        let reaction = ChatReaction::new(
            " ".to_string(),
            " 👩🏽‍💻 ".to_string(),
            ChatReactionOperation::Add,
        )
        .unwrap();
        assert_eq!(
            flow_like_types::json::to_value(reaction).unwrap(),
            json!({"emoji": "👩🏽‍💻", "operation": "add"})
        );
    }

    #[test]
    fn removal_keeps_the_explicit_target_and_rejects_empty_reactions() {
        let reaction = ChatReaction::new(
            " user-message ".to_string(),
            "❤️".to_string(),
            ChatReactionOperation::Remove,
        )
        .unwrap();
        let encoded = json!({"message_id": "user-message", "emoji": "❤️", "operation": "remove"});
        assert_eq!(flow_like_types::json::to_value(&reaction).unwrap(), encoded);
        assert_eq!(
            flow_like_types::json::from_value::<ChatReaction>(encoded).unwrap(),
            reaction
        );
        assert!(
            ChatReaction::new(String::new(), "  ".to_string(), ChatReactionOperation::Add).is_err()
        );
        assert!(
            ChatReaction::new(String::new(), "👍".repeat(33), ChatReactionOperation::Add).is_err()
        );
        assert!(
            flow_like_types::json::from_value::<ChatReactionOperation>(json!("toggle")).is_err()
        );
    }
}
