use super::computer_use::AgentAction;
use crate::llm::ModelImage;
use flow_like_types::{Value, json};
use rig::OneOrMany;
use rig::message::{
    AssistantContent, ImageMediaType, Message, MimeType, ToolResult, ToolResultContent, UserContent,
};

pub(crate) const OMITTED_SCREENSHOT: &str = "[earlier screenshot omitted]";
pub(crate) const OMITTED_ELEMENTS: &str = "[earlier element list omitted]";
pub(crate) const ELEMENTS_HEADER: &str = "Numbered elements";

/// Observations with images that trigger pruning, and how many of the newest survive it.
/// Pruning in batches keeps the prefix stable between prunes, so provider prompt caches hit.
pub(crate) const PRUNE_TRIGGER: usize = 5;
pub(crate) const KEEP_OBSERVATIONS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolCallRequest {
    pub id: String,
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

pub(crate) fn text(text: impl Into<String>) -> UserContent {
    UserContent::text(text)
}

pub(crate) fn image(image: &ModelImage) -> UserContent {
    UserContent::image_base64(
        image.base64.clone(),
        ImageMediaType::from_mime_type(&image.media_type),
        None,
    )
}

pub(crate) fn user(first: UserContent, rest: Vec<UserContent>) -> Message {
    let mut content = OneOrMany::one(first);
    for part in rest {
        content.push(part);
    }
    Message::User { content }
}

/// Tool calls of a model reply in order; string-encoded arguments are decoded.
pub(crate) fn tool_calls(choice: &OneOrMany<AssistantContent>) -> Vec<ToolCallRequest> {
    choice
        .iter()
        .filter_map(|content| match content {
            AssistantContent::ToolCall(call) => Some(ToolCallRequest {
                id: call.id.clone(),
                call_id: call.call_id.clone(),
                name: call.function.name.clone(),
                arguments: match &call.function.arguments {
                    Value::String(raw) => {
                        json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
                    }
                    other => other.clone(),
                },
            }),
            _ => None,
        })
        .collect()
}

pub(crate) fn assistant_text(choice: &OneOrMany<AssistantContent>) -> String {
    choice
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn result_text(action: &AgentAction) -> String {
    match (action.skipped, action.ok) {
        (true, _) => format!("Not executed: {}", action.message),
        (false, true) => action.message.clone(),
        (false, false) => format!("Error: {}", action.message),
    }
}

/// One user message answering every tool call of the previous assistant turn, in order.
pub(crate) fn tool_results(calls: &[ToolCallRequest], actions: &[AgentAction]) -> Option<Message> {
    let mut parts = calls.iter().zip(actions).map(|(call, action)| {
        UserContent::ToolResult(ToolResult {
            id: call.id.clone(),
            call_id: call.call_id.clone(),
            content: OneOrMany::one(ToolResultContent::text(result_text(action))),
        })
    });
    let first = parts.next()?;
    Some(user(first, parts.collect()))
}

fn has_image(message: &Message) -> bool {
    matches!(message, Message::User { content } if content.iter().any(|part| matches!(part, UserContent::Image(_))))
}

/// Replaces the screenshots and element lists of all but the newest `keep` observations once
/// more than `trigger` observations carry images. Returns how many observations were pruned.
pub(crate) fn prune_observations(messages: &mut [Message], trigger: usize, keep: usize) -> usize {
    let observations: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| has_image(message))
        .map(|(index, _)| index)
        .collect();
    if observations.len() <= trigger {
        return 0;
    }
    let stale = &observations[..observations.len().saturating_sub(keep)];
    for &index in stale {
        if let Message::User { content } = &mut messages[index] {
            for part in content.iter_mut() {
                let replacement = match part {
                    UserContent::Image(_) => OMITTED_SCREENSHOT,
                    UserContent::Text(t) if t.text.starts_with(ELEMENTS_HEADER) => OMITTED_ELEMENTS,
                    _ => continue,
                };
                *part = text(replacement);
            }
        }
    }
    stale.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;
    use rig::message::{ToolCall, ToolFunction};

    fn model_image() -> ModelImage {
        ModelImage {
            media_type: "image/png".into(),
            base64: "iVBORw0KGgo=".into(),
        }
    }

    fn observation(label: &str) -> Message {
        user(
            text(format!("Screenshot {label}")),
            vec![
                text(format!("{ELEMENTS_HEADER}: [1] button \"{label}\"")),
                image(&model_image()),
            ],
        )
    }

    fn action(skipped: bool, ok: bool, message: &str) -> AgentAction {
        AgentAction {
            tool: "click".into(),
            args: json!({}),
            input_points: vec![],
            skipped,
            ok,
            message: message.into(),
        }
    }

    fn parts(message: &Message) -> Vec<UserContent> {
        match message {
            Message::User { content } => content.iter().cloned().collect(),
            other => panic!("expected a user message, got {other:?}"),
        }
    }

    #[test]
    fn tool_calls_keep_order_ids_and_decode_string_arguments() {
        let choice = OneOrMany::many(vec![
            AssistantContent::text("I'll open the menu."),
            AssistantContent::ToolCall(
                ToolCall::new(
                    "a".into(),
                    ToolFunction::new("click".into(), json!({"x": 1, "y": 2})),
                )
                .with_call_id("call_a".into()),
            ),
            AssistantContent::ToolCall(ToolCall::new(
                "b".into(),
                ToolFunction::new("type".into(), Value::String(r#"{"text":"hi"}"#.into())),
            )),
        ])
        .unwrap();
        let calls = tool_calls(&choice);
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (calls[0].id.as_str(), calls[0].call_id.as_deref()),
            ("a", Some("call_a"))
        );
        assert_eq!(calls[1].arguments, json!({"text": "hi"}));
        assert_eq!(assistant_text(&choice), "I'll open the menu.");
    }

    #[test]
    fn every_call_gets_one_result_in_a_single_message() {
        let calls = vec![
            ToolCallRequest {
                id: "a".into(),
                call_id: Some("call_a".into()),
                name: "click".into(),
                arguments: json!({}),
            },
            ToolCallRequest {
                id: "b".into(),
                call_id: None,
                name: "type".into(),
                arguments: json!({}),
            },
        ];
        let actions = vec![
            action(
                false,
                false,
                "Point (2000, 5) lies outside the 1456x816 screenshot",
            ),
            action(true, false, "an earlier action in this turn failed"),
        ];
        let message = tool_results(&calls, &actions).unwrap();
        let results: Vec<(String, Option<String>, String)> = parts(&message)
            .into_iter()
            .map(|part| match part {
                UserContent::ToolResult(result) => {
                    let ToolResultContent::Text(text) = result.content.first() else {
                        panic!("tool results are text");
                    };
                    (result.id, result.call_id, text.text)
                }
                other => panic!("expected a tool result, got {other:?}"),
            })
            .collect();
        assert_eq!(results[0].0, "a");
        assert_eq!(results[0].1.as_deref(), Some("call_a"));
        assert!(results[0].2.starts_with("Error: Point (2000, 5)"));
        assert_eq!(
            results[1].2,
            "Not executed: an earlier action in this turn failed"
        );
        assert!(tool_results(&[], &[]).is_none());
        assert_eq!(result_text(&action(false, true, "Clicked")), "Clicked");
    }

    #[test]
    fn pruning_waits_for_the_trigger_then_keeps_the_newest_observations() {
        let mut messages = vec![user(
            text("Task: rename the file"),
            vec![
                text(format!("{ELEMENTS_HEADER}: [1] button \"first\"")),
                image(&model_image()),
            ],
        )];
        for step in 1..=4 {
            messages.push(Message::assistant(format!("step {step}")));
            messages.push(observation(&step.to_string()));
        }
        assert_eq!(prune_observations(&mut messages, 5, 3), 0);
        messages.push(Message::assistant("step 5"));
        messages.push(observation("5"));
        assert_eq!(prune_observations(&mut messages, 5, 3), 3);

        let images = messages.iter().filter(|m| has_image(m)).count();
        assert_eq!(images, 3);
        let first = parts(&messages[0]);
        assert!(matches!(&first[0], UserContent::Text(t) if t.text == "Task: rename the file"));
        assert!(matches!(&first[1], UserContent::Text(t) if t.text == OMITTED_ELEMENTS));
        assert!(matches!(&first[2], UserContent::Text(t) if t.text == OMITTED_SCREENSHOT));
        let newest = parts(messages.last().unwrap());
        assert!(matches!(&newest[2], UserContent::Image(_)));
        assert!(matches!(&newest[1], UserContent::Text(t) if t.text.starts_with(ELEMENTS_HEADER)));

        assert_eq!(prune_observations(&mut messages, 5, 3), 0);
        messages.push(observation("6"));
        messages.push(observation("7"));
        assert_eq!(prune_observations(&mut messages, 5, 3), 0);
        messages.push(observation("8"));
        assert_eq!(prune_observations(&mut messages, 5, 3), 3);
        assert_eq!(messages.iter().filter(|m| has_image(m)).count(), 3);
    }

    #[test]
    fn images_carry_their_media_type() {
        let UserContent::Image(sent) = image(&model_image()) else {
            panic!("expected an image part");
        };
        assert_eq!(sent.media_type, Some(ImageMediaType::PNG));
    }
}
