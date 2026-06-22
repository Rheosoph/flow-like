use crate::history::{Content, ContentType, History, HistoryMessage, MessageContent, Role};
use flow_like_types::Result;
use rig::OneOrMany;
use rig::completion::Message as RigMessage;
use rig::message::{AssistantContent, Text, UserContent};

pub fn history_to_rig_messages(history: &History) -> Result<Vec<RigMessage>> {
    let mut messages = Vec::new();

    for msg in &history.messages {
        let rig_msg = history_message_to_rig(msg)?;
        messages.push(rig_msg);
    }

    Ok(messages)
}

pub fn history_message_to_rig(msg: &HistoryMessage) -> Result<RigMessage> {
    match msg.role {
        Role::User => {
            let content = extract_text_content(&msg.content);
            Ok(RigMessage::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: content,
                    additional_params: None,
                })),
            })
        }
        Role::Assistant => {
            let content = extract_text_content(&msg.content);
            Ok(RigMessage::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(Text {
                    text: content,
                    additional_params: None,
                })),
            })
        }
        Role::System | Role::Function | Role::Tool => {
            let content = extract_text_content(&msg.content);
            Ok(RigMessage::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: content,
                    additional_params: None,
                })),
            })
        }
    }
}

fn extract_text_content(content: &MessageContent) -> String {
    match content {
        MessageContent::String(s) => s.clone(),
        MessageContent::Contents(contents) => {
            let text_parts: Vec<String> = contents
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text, .. } => Some(text.clone()),
                    Content::Image { .. } => None,
                    Content::Audio { .. } => None,
                    Content::Video { .. } => None,
                    Content::Document { .. } => None,
                })
                .collect();
            text_parts.join("\n")
        }
    }
}

pub fn rig_message_to_history(msg: &RigMessage) -> Result<HistoryMessage> {
    match msg {
        RigMessage::System { content } => Ok(HistoryMessage {
            role: Role::System,
            content: MessageContent::String(content.clone()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        }),
        RigMessage::User { content } => {
            let text = extract_rig_user_text(content);
            Ok(HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text,
                }]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            })
        }
        RigMessage::Assistant { id: _, content } => {
            let text = extract_rig_assistant_text(content);
            Ok(HistoryMessage {
                role: Role::Assistant,
                content: MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text,
                }]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            })
        }
    }
}

fn extract_rig_user_text(content: &OneOrMany<UserContent>) -> String {
    let first = content.first();
    let rest = content.rest();

    let mut texts = Vec::new();
    if let UserContent::Text(t) = &first {
        texts.push(t.text.clone());
    }

    for c in rest {
        if let UserContent::Text(t) = c {
            texts.push(t.text.clone());
        }
    }

    texts.join("\n")
}

fn extract_rig_assistant_text(content: &OneOrMany<AssistantContent>) -> String {
    let first = content.first();
    let rest = content.rest();

    let mut texts = Vec::new();
    if let AssistantContent::Text(t) = &first {
        texts.push(t.text.clone());
    }

    for c in rest {
        if let AssistantContent::Text(t) = c {
            texts.push(t.text.clone());
        }
    }

    texts.join("\n")
}

pub fn rig_messages_to_history(messages: Vec<RigMessage>, model: String) -> Result<History> {
    let history_messages: Result<Vec<HistoryMessage>> =
        messages.iter().map(rig_message_to_history).collect();

    Ok(History::new(model, history_messages?))
}
