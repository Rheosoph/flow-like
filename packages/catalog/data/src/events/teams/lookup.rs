use super::{
    ANY_SESSION, TeamsCall, add_session_pin,
    context::{TeamsHistoryMessage, TeamsMember},
    teams_node,
};
use flow_like::{
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
    hub::HubRetry,
};
use flow_like_model_provider::history::{History, HistoryMessage, Role};
use flow_like_types::{Value, anyhow, async_trait, json::json, reqwest};
use serde::Deserialize;
use std::time::Duration;

const MESSAGE_SCOPES: [&str; 2] = ["thread", "conversation"];
const MAX_MESSAGES: i64 = 50;
const MAX_MEMBERS: i64 = 500;

// A lookup 409 is a setup error (permission off, admin consent missing), never in flight.
fn lookup_transient(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

const LOOKUP_RETRY: HubRetry = HubRetry {
    retryable: lookup_transient,
    retries: 3,
    max_wait: Duration::from_secs(60),
};

fn lookup_node(name: &str, label: &str, description: &str, alias: &str, action: &str) -> Node {
    let mut node = teams_node(
        name,
        label,
        description,
        alias,
        NodeScores::new()
            .set_privacy(4)
            .set_security(6)
            .set_performance(6)
            .set_governance(6)
            .set_reliability(6)
            .set_cost(8)
            .build(),
    );
    node.set_version(1);
    node.add_input_pin("exec_in", "In", action, VariableType::Execution);
    add_session_pin(&mut node, ANY_SESSION);
    node.add_output_pin(
        "exec_out",
        "Out",
        "Teams returned the result",
        VariableType::Execution,
    );
    node
}

fn limit(value: i64, max: i64) -> flow_like_types::Result<i64> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(anyhow!("Limit must be between 1 and {max}, got {value}"))
    }
}

fn messages_request(session_id: &str, scope: &str, count: i64) -> flow_like_types::Result<Value> {
    if !MESSAGE_SCOPES.contains(&scope) {
        return Err(anyhow!(
            "Scope must be 'thread' or 'conversation', got '{scope}'"
        ));
    }
    Ok(json!({
        "session_id": session_id,
        "scope": scope,
        "limit": limit(count, MAX_MESSAGES)?,
    }))
}

fn members_request(
    session_id: &str,
    count: i64,
    continuation_token: &str,
) -> flow_like_types::Result<Value> {
    let mut body = json!({
        "session_id": session_id,
        "limit": limit(count, MAX_MEMBERS)?,
    });
    let continuation_token = continuation_token.trim();
    if !continuation_token.is_empty() {
        body["continuation_token"] = json!(continuation_token);
    }
    Ok(body)
}

fn is_bot(message: &TeamsHistoryMessage) -> bool {
    message.is_bot.unwrap_or(false)
}

fn author(message: &TeamsHistoryMessage) -> &str {
    message
        .author_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(if is_bot(message) { "Bot" } else { "Teams user" })
}

fn body(message: &TeamsHistoryMessage) -> String {
    let mut body = message
        .text
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_owned();
    for file in &message.attachments {
        let kind = match file.content_type.as_deref() {
            Some(kind) if kind.starts_with("image/") => "image",
            _ => "file",
        };
        let name = file.name.as_deref().unwrap_or("attachment");
        body.push_str(&format!("\n[{kind}: {name}]"));
    }
    body.trim().to_owned()
}

fn spoken(
    messages: &[TeamsHistoryMessage],
) -> impl Iterator<Item = (&TeamsHistoryMessage, String)> {
    messages
        .iter()
        .map(|message| (message, body(message)))
        .filter(|(_, body)| !body.is_empty())
}

fn history(messages: &[TeamsHistoryMessage]) -> History {
    let entries = spoken(messages)
        .map(|(message, body)| {
            if is_bot(message) {
                HistoryMessage::from_string(Role::Assistant, &body)
            } else {
                HistoryMessage::from_string(Role::User, &format!("{}: {body}", author(message)))
            }
        })
        .collect();
    let mut history = History::new(String::new(), entries);
    // Same non-streaming default as the Chat Event history.
    history.stream = Some(false);
    history
}

fn transcript(messages: &[TeamsHistoryMessage]) -> String {
    spoken(messages)
        .map(|(message, body)| format!("{}: {body}", author(message)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    source: String,
    #[serde(default)]
    messages: Vec<TeamsHistoryMessage>,
}

#[derive(Deserialize)]
struct MembersResponse {
    #[serde(default)]
    members: Vec<TeamsMember>,
    #[serde(default)]
    continuation_token: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsGetMessages;

#[async_trait]
impl NodeLogic for TeamsGetMessages {
    fn get_node(&self) -> Node {
        let mut node = lookup_node(
            "events_teams_get_messages",
            "Get Teams Messages",
            "Read recent messages of the Teams thread or conversation of a Chat Event session, with author names",
            "getMessages",
            "Read the Teams messages",
        );
        node.add_input_pin(
            "scope",
            "Scope",
            "thread reads the channel thread of the message, conversation reads the whole channel or chat",
            VariableType::String,
        )
        .set_default_value(Some(json!("thread")))
        .set_options(
            PinOptions::new()
                .set_valid_values(MESSAGE_SCOPES.iter().map(|scope| scope.to_string()).collect())
                .build(),
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of messages, 1 to 50",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)))
        .set_options(
            PinOptions::new()
                .set_range((1., MAX_MESSAGES as f64))
                .build(),
        );
        node.add_output_pin(
            "messages",
            "Messages",
            "Messages oldest first",
            VariableType::Struct,
        )
        .set_schema::<TeamsHistoryMessage>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "history",
            "History",
            "Chat history with people's messages as \"Name: text\" and bot messages as assistant",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "transcript",
            "Transcript",
            "One \"Name: text\" line per message",
            VariableType::String,
        );
        node.add_output_pin(
            "source",
            "Source",
            "graph for Microsoft Graph, bot_history for the bot's own 1:1 history",
            VariableType::String,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let call = TeamsCall::bind(context, "read Teams messages").await?;
        let scope: String = context.evaluate_pin("scope").await?;
        let count: i64 = context.evaluate_pin("limit").await?;
        let request = messages_request(&call.session_id, &scope, count)?;
        let response: MessagesResponse = call
            .post("messages", "Teams messages lookup", &request, LOOKUP_RETRY)
            .await?;
        context
            .set_pin_value("history", json!(history(&response.messages)))
            .await?;
        context
            .set_pin_value("transcript", json!(transcript(&response.messages)))
            .await?;
        context
            .set_pin_value("messages", json!(response.messages))
            .await?;
        context
            .set_pin_value("source", json!(response.source))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsGetMembers;

#[async_trait]
impl NodeLogic for TeamsGetMembers {
    fn get_node(&self) -> Node {
        let mut node = lookup_node(
            "events_teams_get_members",
            "Get Teams Members",
            "List the members of the Teams conversation of a Chat Event session, one page at a time",
            "getMembers",
            "Read the Teams members",
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of members in this page, 1 to 500",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)))
        .set_options(
            PinOptions::new()
                .set_range((1., MAX_MEMBERS as f64))
                .build(),
        );
        node.add_input_pin(
            "continuation_token",
            "Continuation Token",
            "Next Continuation Token of the previous page, empty for the first page",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_output_pin(
            "members",
            "Members",
            "Members of the conversation",
            VariableType::Struct,
        )
        .set_schema::<TeamsMember>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "next_continuation_token",
            "Next Continuation Token",
            "Pass it to the next call for more members, empty on the last page",
            VariableType::String,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let call = TeamsCall::bind(context, "read Teams members").await?;
        let count: i64 = context.evaluate_pin("limit").await?;
        let continuation_token: String = context.evaluate_pin("continuation_token").await?;
        let request = members_request(&call.session_id, count, &continuation_token)?;
        let response: MembersResponse = call
            .post("members", "Teams members lookup", &request, LOOKUP_RETRY)
            .await?;
        context
            .set_pin_value("members", json!(response.members))
            .await?;
        context
            .set_pin_value(
                "next_continuation_token",
                json!(response.continuation_token.unwrap_or_default()),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_model_provider::history::{Content, MessageContent};
    use flow_like_types::json::from_value;

    fn message(value: Value) -> TeamsHistoryMessage {
        from_value(value).unwrap()
    }

    fn text_of(message: &HistoryMessage) -> &str {
        match &message.content {
            MessageContent::String(text) => text,
            MessageContent::Contents(parts) => match parts.as_slice() {
                [Content::Text { text, .. }] => text,
                other => panic!("unexpected content {other:?}"),
            },
        }
    }

    fn conversation() -> Vec<TeamsHistoryMessage> {
        vec![
            message(json!({"author_name": "Anna Berg", "text": "Draft is ready", "is_bot": false})),
            message(json!({"author_name": "Flow Bot", "text": "I summarised it.", "is_bot": true})),
            message(json!({"author_name": "  ", "text": " Thanks! "})),
            message(
                json!({"author_name": "Anna Berg", "text": "", "attachments": [
                    {"name": "chart.png", "type": "image/png", "link": "https://contoso.sharepoint.com/chart.png"},
                    {"name": "report.pdf", "type": "application/pdf"}
                ]}),
            ),
            message(json!({"author_name": "Anna Berg", "text": "   "})),
            message(json!({"is_bot": true, "text": "Done", "unknown": 1})),
        ]
    }

    #[test]
    fn history_prefixes_people_and_maps_bots_to_assistant() {
        let history = history(&conversation());
        assert_eq!(history.stream, Some(false));
        let entries: Vec<_> = history
            .messages
            .iter()
            .map(|entry| (entry.role.clone(), text_of(entry)))
            .collect();
        assert_eq!(
            entries,
            [
                (Role::User, "Anna Berg: Draft is ready"),
                (Role::Assistant, "I summarised it."),
                (Role::User, "Teams user: Thanks!"),
                (
                    Role::User,
                    "Anna Berg: [image: chart.png]\n[file: report.pdf]"
                ),
                (Role::Assistant, "Done"),
            ]
        );
    }

    #[test]
    fn transcript_lists_every_author() {
        assert_eq!(
            transcript(&conversation()),
            "Anna Berg: Draft is ready\n\
             Flow Bot: I summarised it.\n\
             Teams user: Thanks!\n\
             Anna Berg: [image: chart.png]\n[file: report.pdf]\n\
             Bot: Done"
        );
        assert_eq!(transcript(&[]), "");
    }

    #[test]
    fn requests_validate_scope_and_limits() {
        assert_eq!(
            messages_request("run-1", "thread", 20).unwrap(),
            json!({"session_id": "run-1", "scope": "thread", "limit": 20})
        );
        assert!(messages_request("run-1", "conversation", 50).is_ok());
        assert!(messages_request("run-1", "channel", 20).is_err());
        assert!(messages_request("run-1", "thread", 0).is_err());
        assert!(messages_request("run-1", "thread", 51).is_err());

        assert_eq!(
            members_request("run-1", 100, "").unwrap(),
            json!({"session_id": "run-1", "limit": 100})
        );
        assert_eq!(
            members_request("run-1", 500, " next ").unwrap(),
            json!({"session_id": "run-1", "limit": 500, "continuation_token": "next"})
        );
        let error = members_request("run-1", 501, "").unwrap_err().to_string();
        assert!(error.contains("501"), "{error}");
    }

    #[test]
    fn lookups_retry_only_throttled_and_server_failures() {
        let status = |code| reqwest::StatusCode::from_u16(code).unwrap();
        for code in [429, 500, 502, 503, 504] {
            assert!(lookup_transient(status(code)), "{code} should retry");
            assert!(
                (LOOKUP_RETRY.retryable)(status(code)),
                "{code} should retry"
            );
        }
        for code in [400, 403, 404, 409, 422] {
            assert!(!lookup_transient(status(code)), "{code} is final");
            assert!(!(LOOKUP_RETRY.retryable)(status(code)), "{code} is final");
        }
        assert_eq!(LOOKUP_RETRY.retries, 3);
        assert_eq!(LOOKUP_RETRY.max_wait, Duration::from_secs(60));
    }

    #[test]
    fn responses_tolerate_missing_fields() {
        let messages: MessagesResponse = from_value(json!({})).unwrap();
        assert!(messages.source.is_empty() && messages.messages.is_empty());
        let members: MembersResponse = from_value(json!({
            "members": [{"id": "29:anna", "name": "Anna Berg", "user_role": "user"}],
            "continuation_token": null
        }))
        .unwrap();
        assert_eq!(members.members[0].name.as_deref(), Some("Anna Berg"));
        assert!(members.continuation_token.is_none());
    }
}
