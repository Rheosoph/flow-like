use super::{add_session_pin, session_id, teams_node};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, anyhow, async_trait,
    json::{from_value, json},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsContext {
    pub session_id: Option<String>,
    pub permissions: Vec<String>,
    pub conversation: Option<TeamsConversation>,
    pub user: Option<TeamsUser>,
    pub team: Option<TeamsTeam>,
    pub channel: Option<TeamsChannel>,
    pub meeting: Option<TeamsMeeting>,
    pub message: Option<TeamsMessageInfo>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsConversation {
    pub id: Option<String>,
    /// `personal`, `groupChat` or `channel`
    #[serde(rename = "type")]
    pub conversation_type: Option<String>,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub web_url: Option<String>,
    pub tenant_id: Option<String>,
    pub thread_id: Option<String>,
    pub is_group: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsUser {
    pub id: Option<String>,
    pub aad_object_id: Option<String>,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub surname: Option<String>,
    pub email: Option<String>,
    pub user_principal_name: Option<String>,
    pub tenant_id: Option<String>,
    pub role: Option<String>,
    pub external: Option<bool>,
    pub anonymous: Option<bool>,
    pub meeting_role: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub platform: Option<String>,
    pub country: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsTeam {
    pub id: Option<String>,
    pub graph_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub web_url: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsChannel {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub membership_type: Option<String>,
    pub web_url: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsMeeting {
    pub id: Option<String>,
    pub title: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub join_url: Option<String>,
    pub organizer: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsMessageInfo {
    pub id: Option<String>,
    pub timestamp: Option<String>,
    pub local_timestamp: Option<String>,
    pub text: Option<String>,
    pub html: Option<String>,
    pub reply_to_id: Option<String>,
    pub mentions_bot: Option<bool>,
    pub mentions: Vec<TeamsMention>,
    pub quoted: Option<TeamsQuote>,
    pub files: Vec<TeamsFile>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsMention {
    pub id: Option<String>,
    pub aad_object_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsQuote {
    pub message_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub preview: Option<String>,
    pub time: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsFile {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub content_type: Option<String>,
    pub size: Option<u64>,
    /// Signed download URL, present when the server downloaded the file
    pub url: Option<String>,
    pub path: Option<FlowPath>,
    pub downloadable: Option<bool>,
    /// SharePoint or OneDrive link for files the bot cannot download
    pub link: Option<String>,
    /// Why the file was skipped
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsFileLink {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub content_type: Option<String>,
    pub link: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsHistoryMessage {
    pub id: Option<String>,
    pub author_id: Option<String>,
    pub author_aad_object_id: Option<String>,
    pub author_name: Option<String>,
    pub is_bot: Option<bool>,
    pub text: Option<String>,
    pub html: Option<String>,
    pub created_at: Option<String>,
    pub reply_to_id: Option<String>,
    pub attachments: Vec<TeamsFileLink>,
}

#[derive(Serialize, Deserialize, JsonSchema, Default, Clone, Debug)]
#[serde(default)]
pub struct TeamsMember {
    pub id: Option<String>,
    pub aad_object_id: Option<String>,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub surname: Option<String>,
    pub email: Option<String>,
    pub user_principal_name: Option<String>,
    pub tenant_id: Option<String>,
    pub user_role: Option<String>,
}

fn read_context(session: &Value) -> flow_like_types::Result<TeamsContext> {
    session_id(session).map_err(|_| {
        anyhow!("Connect the Local Session of a Teams Chat Event to read its Teams context")
    })?;
    from_value(session["teams"].clone()).map_err(|error| {
        anyhow!("The Teams context in the Chat Event local session is malformed: {error}")
    })
}

fn text(value: Option<&String>) -> Value {
    json!(value.map(String::as_str).unwrap_or_default())
}

fn context_pins(teams: &TeamsContext) -> Vec<(&'static str, Value)> {
    let user = teams.user.clone().unwrap_or_default();
    let conversation = teams.conversation.as_ref();
    let conversation_type = conversation.and_then(|c| c.conversation_type.as_ref());
    let is_group = conversation.and_then(|c| c.is_group).unwrap_or_else(|| {
        conversation_type.is_some_and(|kind| kind == "groupChat" || kind == "channel")
    });
    let message = teams.message.as_ref();
    let files: Vec<&FlowPath> = message
        .into_iter()
        .flat_map(|message| &message.files)
        .filter_map(|file| file.path.as_ref())
        .collect();
    vec![
        ("context", json!(teams)),
        ("user_name", text(user.name.as_ref())),
        ("user_email", text(user.email.as_ref())),
        ("conversation_type", text(conversation_type)),
        (
            "team_name",
            text(teams.team.as_ref().and_then(|team| team.name.as_ref())),
        ),
        (
            "channel_name",
            text(
                teams
                    .channel
                    .as_ref()
                    .and_then(|channel| channel.name.as_ref()),
            ),
        ),
        ("locale", text(user.locale.as_ref())),
        ("timezone", text(user.timezone.as_ref())),
        (
            "message_html",
            text(message.and_then(|message| message.html.as_ref())),
        ),
        ("is_group", json!(is_group)),
        ("files", json!(files)),
        ("user", json!(user)),
    ]
}

fn add_string_output(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_output_pin(name, label, description, VariableType::String);
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsContextNode;

#[async_trait]
impl NodeLogic for TeamsContextNode {
    fn get_node(&self) -> Node {
        let mut node = teams_node(
            "events_teams_context",
            "Teams Context",
            "Read the conversation, user, team, channel, meeting and message details of a Teams Chat Event",
            "context",
            NodeScores::new()
                .set_privacy(5)
                .set_security(8)
                .set_performance(10)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_version(1);
        add_session_pin(&mut node, "Local Session from the Teams Chat Event");
        node.add_output_pin(
            "context",
            "Context",
            "Everything Teams and Microsoft shared about this message",
            VariableType::Struct,
        )
        .set_schema::<TeamsContext>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "user",
            "User",
            "The person who sent the message",
            VariableType::Struct,
        )
        .set_schema::<TeamsUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        for (name, label, description) in [
            ("user_name", "User Name", "Display name of the sender"),
            (
                "user_email",
                "User Email",
                "Email address of the sender, empty when Teams does not share it",
            ),
            (
                "conversation_type",
                "Conversation Type",
                "personal, groupChat or channel",
            ),
            (
                "team_name",
                "Team Name",
                "Team of a channel conversation, empty elsewhere",
            ),
            (
                "channel_name",
                "Channel Name",
                "Channel of a channel conversation, empty elsewhere",
            ),
            (
                "locale",
                "Locale",
                "Locale of the sender's Teams client, e.g. de-DE",
            ),
            (
                "timezone",
                "Timezone",
                "IANA timezone of the sender's Teams client, e.g. Europe/Berlin",
            ),
            (
                "message_html",
                "Message HTML",
                "Formatted HTML of the message, empty for plain text messages",
            ),
        ] {
            add_string_output(&mut node, name, label, description);
        }
        node.add_output_pin(
            "is_group",
            "Is Group",
            "True in group chats and channels",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "files",
            "Files",
            "Files and images from the message that the server downloaded",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: Value = context.evaluate_pin("session").await?;
        let teams = read_context(&session)?;
        for (pin, value) in context_pins(&teams) {
            context.set_pin_value(pin, value).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample() -> Value {
        json!({
            "session_id": "run-1",
            "permissions": ["read_messages"],
            "conversation": {
                "id": "19:abc@thread.tacv2;messageid=1700",
                "type": "channel",
                "name": "Weekly sync",
                "topic": "Planning",
                "web_url": "https://teams.microsoft.com/l/channel/abc",
                "tenant_id": "tenant-1",
                "thread_id": "1700",
                "is_group": true
            },
            "user": {
                "id": "29:user",
                "aad_object_id": "aad-1",
                "name": "Felix Schultz",
                "given_name": "Felix",
                "surname": "Schultz",
                "email": "felix@example.com",
                "user_principal_name": "felix@example.com",
                "tenant_id": "tenant-1",
                "role": "user",
                "external": false,
                "anonymous": false,
                "meeting_role": "Organizer",
                "locale": "de-DE",
                "timezone": "Europe/Berlin",
                "platform": "Windows",
                "country": "DE"
            },
            "team": {
                "id": "19:team",
                "graph_id": "group-1",
                "name": "Engineering",
                "description": "Builders",
                "web_url": "https://teams.microsoft.com/l/team/abc",
                "visibility": "private"
            },
            "channel": {
                "id": "19:channel",
                "name": "General",
                "description": "Everything",
                "membership_type": "standard",
                "web_url": "https://teams.microsoft.com/l/channel/abc"
            },
            "meeting": {
                "id": "meeting-1",
                "title": "Weekly sync",
                "start": "2026-09-28T09:00:00Z",
                "end": "2026-09-28T10:00:00Z",
                "join_url": "https://teams.microsoft.com/l/meetup-join/abc",
                "organizer": "Anna Berg"
            },
            "message": {
                "id": "1701",
                "timestamp": "2026-09-28T09:05:00Z",
                "local_timestamp": "2026-09-28T11:05:00+02:00",
                "text": "@Anna please check the report",
                "html": "<p><at>Anna</at> please check the report</p>",
                "reply_to_id": "1700",
                "mentions_bot": true,
                "mentions": [{ "id": "29:anna", "aad_object_id": "aad-2", "name": "Anna Berg" }],
                "quoted": {
                    "message_id": "1699",
                    "sender_id": "29:anna",
                    "sender_name": "Anna Berg",
                    "preview": "Draft is ready",
                    "time": "2026-09-28T08:00:00Z"
                },
                "files": [
                    {
                        "name": "chart.png",
                        "type": "image/png",
                        "size": 1234,
                        "url": "https://storage.example.com/signed",
                        "path": {
                            "path": "tmp/runs/run-1/request/teams/0000-chart.png",
                            "store_ref": "__flow_like_http_request_files",
                            "cache_store_ref": null
                        },
                        "downloadable": true
                    },
                    {
                        "name": "report.pdf",
                        "downloadable": false,
                        "link": "https://contoso.sharepoint.com/report.pdf",
                        "error": "Files shared in channels and group chats cannot be downloaded by the bot"
                    }
                ]
            }
        })
    }

    fn pins(teams: &TeamsContext) -> HashMap<&'static str, Value> {
        context_pins(teams).into_iter().collect()
    }

    #[test]
    fn deserialises_the_full_session_contract() {
        let teams: TeamsContext = from_value(sample()).unwrap();
        assert_eq!(teams.session_id.as_deref(), Some("run-1"));
        assert_eq!(teams.permissions, ["read_messages"]);
        let conversation = teams.conversation.as_ref().unwrap();
        assert_eq!(conversation.conversation_type.as_deref(), Some("channel"));
        assert_eq!(conversation.thread_id.as_deref(), Some("1700"));
        let user = teams.user.as_ref().unwrap();
        assert_eq!(user.meeting_role.as_deref(), Some("Organizer"));
        assert_eq!(user.external, Some(false));
        assert_eq!(
            teams.team.as_ref().unwrap().graph_id.as_deref(),
            Some("group-1")
        );
        assert_eq!(
            teams.channel.as_ref().unwrap().membership_type.as_deref(),
            Some("standard")
        );
        assert_eq!(
            teams.meeting.as_ref().unwrap().organizer.as_deref(),
            Some("Anna Berg")
        );
        let message = teams.message.as_ref().unwrap();
        assert_eq!(message.mentions[0].name.as_deref(), Some("Anna Berg"));
        assert_eq!(
            message.quoted.as_ref().unwrap().preview.as_deref(),
            Some("Draft is ready")
        );
        assert_eq!(message.files[0].content_type.as_deref(), Some("image/png"));
        assert_eq!(message.files[0].size, Some(1234));
        assert_eq!(
            message.files[0].path.as_ref().unwrap().store_ref,
            "__flow_like_http_request_files"
        );
        assert!(message.files[1].path.is_none());
        assert_eq!(message.files[1].downloadable, Some(false));
    }

    #[test]
    fn deserialises_an_empty_context_and_tolerates_unknown_keys() {
        let empty: TeamsContext = from_value(json!({})).unwrap();
        assert!(empty.session_id.is_none());
        assert!(empty.permissions.is_empty());
        assert!(empty.user.is_none() && empty.message.is_none());
        let extended: TeamsContext =
            from_value(json!({"session_id": "run-1", "future": 1, "user": {"extra": true}}))
                .unwrap();
        assert!(extended.user.unwrap().name.is_none());
    }

    #[test]
    fn maps_the_context_onto_its_output_pins() {
        let teams: TeamsContext = from_value(sample()).unwrap();
        let pins = pins(&teams);
        assert_eq!(pins["user_name"], json!("Felix Schultz"));
        assert_eq!(pins["user_email"], json!("felix@example.com"));
        assert_eq!(pins["conversation_type"], json!("channel"));
        assert_eq!(pins["team_name"], json!("Engineering"));
        assert_eq!(pins["channel_name"], json!("General"));
        assert_eq!(pins["locale"], json!("de-DE"));
        assert_eq!(pins["timezone"], json!("Europe/Berlin"));
        assert_eq!(
            pins["message_html"],
            json!("<p><at>Anna</at> please check the report</p>")
        );
        assert_eq!(pins["is_group"], json!(true));
        assert_eq!(pins["user"]["aad_object_id"], json!("aad-1"));
        assert_eq!(pins["context"]["session_id"], json!("run-1"));
        assert_eq!(
            pins["files"],
            json!([{
                "path": "tmp/runs/run-1/request/teams/0000-chart.png",
                "store_ref": "__flow_like_http_request_files",
                "cache_store_ref": null
            }])
        );
        let node = TeamsContextNode.get_node();
        for (name, _) in context_pins(&teams) {
            assert!(node.get_pin_by_name(name).is_some(), "no pin {name}");
        }
    }

    #[test]
    fn missing_details_become_empty_defaults() {
        let teams = read_context(&json!({"teams": {"session_id": "run-1"}})).unwrap();
        let empty = pins(&teams);
        for name in [
            "user_name",
            "user_email",
            "conversation_type",
            "team_name",
            "channel_name",
            "locale",
            "timezone",
            "message_html",
        ] {
            assert_eq!(empty[name], json!(""), "{name}");
        }
        assert_eq!(empty["is_group"], json!(false));
        assert_eq!(empty["files"], json!([]));
        assert!(empty["user"].is_object());

        let group = read_context(&json!({
            "teams": {"session_id": "run-1", "conversation": {"type": "groupChat"}}
        }))
        .unwrap();
        assert_eq!(pins(&group)["is_group"], json!(true));
    }

    #[test]
    fn requires_the_chat_event_local_session() {
        for session in [
            json!({}),
            json!({"teams": {}}),
            json!({"teams": {"session_id": ""}}),
        ] {
            let error = read_context(&session).unwrap_err().to_string();
            assert!(error.contains("Local Session"), "{error}");
        }
    }
}
