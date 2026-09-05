use super::{
    graph::{graph_error_message, graph_get_paginated_values},
    provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Teams Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Team {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Channel {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub web_url: Option<String>,
    pub membership_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    pub id: String,
    pub body_content: String,
    pub body_content_type: String,
    pub created_date_time: Option<String>,
    pub from_user: Option<String>,
}

fn parse_team(value: &Value) -> Option<Team> {
    Some(Team {
        id: value["id"].as_str()?.to_string(),
        display_name: value["displayName"].as_str()?.to_string(),
        description: value["description"].as_str().map(String::from),
        web_url: value["webUrl"].as_str().map(String::from),
    })
}

fn parse_channel(value: &Value) -> Option<Channel> {
    Some(Channel {
        id: value["id"].as_str()?.to_string(),
        display_name: value["displayName"].as_str()?.to_string(),
        description: value["description"].as_str().map(String::from),
        web_url: value["webUrl"].as_str().map(String::from),
        membership_type: value["membershipType"].as_str().map(String::from),
    })
}

fn parse_message(value: &Value) -> Option<ChatMessage> {
    Some(ChatMessage {
        id: value["id"].as_str()?.to_string(),
        body_content: value["body"]["content"].as_str().unwrap_or("").to_string(),
        body_content_type: value["body"]["contentType"]
            .as_str()
            .unwrap_or("text")
            .to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        from_user: value["from"]["user"]["displayName"]
            .as_str()
            .map(String::from),
    })
}

// =============================================================================
// List Joined Teams Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListJoinedTeamsNode {}

impl ListJoinedTeamsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListJoinedTeamsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_list_joined",
            "List Joined Teams",
            "List all Microsoft Teams the user has joined",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "listJoined");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("teams", "Teams", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<Team>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Team.ReadBasic.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, provider.api_url("/me/joinedTeams"))
            .await
        {
            Ok(values) => {
                let teams: Vec<Team> = values.iter().filter_map(parse_team).collect();
                let count = teams.len() as i64;
                context.set_pin_value("teams", json!(teams)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Team Channels Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListTeamChannelsNode {}

impl ListTeamChannelsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListTeamChannelsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_list_channels",
            "List Team Channels",
            "List all channels in a Microsoft Team",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "listChannels");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("team_id", "Team ID", "ID of the team", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("channels", "Channels", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<Channel>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Channel.ReadBasic.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let team_id: String = context.evaluate_pin("team_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/teams/{}/channels", team_id)),
        )
        .await
        {
            Ok(values) => {
                let channels: Vec<Channel> = values.iter().filter_map(parse_channel).collect();
                let count = channels.len() as i64;
                context.set_pin_value("channels", json!(channels)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Send Channel Message Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SendChannelMessageNode {}

impl SendChannelMessageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SendChannelMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_send_message",
            "Send Channel Message",
            "Send a message to a Microsoft Teams channel",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "sendMessage");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("team_id", "Team ID", "ID of the team", VariableType::String);
        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "ID of the channel",
            VariableType::String,
        );
        node.add_input_pin(
            "message",
            "Message",
            "Message content",
            VariableType::String,
        );
        node.add_input_pin(
            "content_type",
            "Content Type",
            "Message content type",
            VariableType::String,
        )
        .set_default_value(Some(json!("text")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["text".to_string(), "html".to_string()])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("message_id", "Message ID", "", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["ChannelMessage.Send"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let team_id: String = context.evaluate_pin("team_id").await?;
        let channel_id: String = context.evaluate_pin("channel_id").await?;
        let message: String = context.evaluate_pin("message").await?;
        let content_type: String = context
            .evaluate_pin("content_type")
            .await
            .unwrap_or_else(|_| "text".to_string());

        let body = json!({
            "body": {
                "contentType": content_type,
                "content": message
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!(
                "/teams/{}/channels/{}/messages",
                team_id, channel_id
            )))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let message_id = body["id"].as_str().unwrap_or("").to_string();
                context
                    .set_pin_value("message_id", json!(message_id))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                context
                    .set_pin_value("error_message", json!(graph_error_message(resp).await))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Channel Messages Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetChannelMessagesNode {}

impl GetChannelMessagesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetChannelMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_get_messages",
            "Get Channel Messages",
            "Get messages from a Microsoft Teams channel",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "getMessages");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("team_id", "Team ID", "ID of the team", VariableType::String);
        node.add_input_pin(
            "channel_id",
            "Channel ID",
            "ID of the channel",
            VariableType::String,
        );
        node.add_input_pin(
            "top",
            "Top",
            "Number of messages to retrieve",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("messages", "Messages", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<ChatMessage>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["ChannelMessage.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let team_id: String = context.evaluate_pin("team_id").await?;
        let channel_id: String = context.evaluate_pin("channel_id").await?;
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(50);

        let client = reqwest::Client::new();
        let url = provider.api_url(&format!(
            "/teams/{}/channels/{}/messages?$top={}",
            team_id, channel_id, top
        ));

        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let messages: Vec<ChatMessage> = values.iter().filter_map(parse_message).collect();
                let count = messages.len() as i64;
                context.set_pin_value("messages", json!(messages)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Create Team Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateTeamNode {}

impl CreateTeamNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateTeamNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_create_team",
            "Create Team",
            "Create a new Microsoft Team",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "createTeam");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "display_name",
            "Display Name",
            "Team name",
            VariableType::String,
        );
        node.add_input_pin(
            "description",
            "Description",
            "Team description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "visibility",
            "Visibility",
            "Team visibility",
            VariableType::String,
        )
        .set_default_value(Some(json!("private")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["private".to_string(), "public".to_string()])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("team_id", "Team ID", "", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Team.Create"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let display_name: String = context.evaluate_pin("display_name").await?;
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let visibility: String = context
            .evaluate_pin("visibility")
            .await
            .unwrap_or_else(|_| "private".to_string());

        let body = json!({
            "template@odata.bind": provider.api_url("/teamsTemplates('standard')"),
            "displayName": display_name,
            "description": description,
            "visibility": visibility
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/teams"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
                // Team creation is async, get ID from Content-Location header
                let team_id = resp
                    .headers()
                    .get("Content-Location")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split('/').next_back())
                    .unwrap_or("")
                    .to_string();
                context.set_pin_value("team_id", json!(team_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                context
                    .set_pin_value("error_message", json!(graph_error_message(resp).await))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Create Channel Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateChannelNode {}

impl CreateChannelNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateChannelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_teams_create_channel",
            "Create Channel",
            "Create a new channel in a Microsoft Team",
            "Data/Microsoft/Teams",
        );
        node.set_flowscript_name("microsoft.teams", "createChannel");
        node.set_version(1);
        node.add_icon("/flow/icons/teams.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("team_id", "Team ID", "ID of the team", VariableType::String);
        node.add_input_pin(
            "display_name",
            "Display Name",
            "Channel name",
            VariableType::String,
        );
        node.add_input_pin(
            "description",
            "Description",
            "Channel description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "membership_type",
            "Membership Type",
            "Channel type",
            VariableType::String,
        )
        .set_default_value(Some(json!("standard")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "standard".to_string(),
                    "private".to_string(),
                    "shared".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("channel", "Channel", "", VariableType::Struct)
            .set_schema::<Channel>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Channel.Create"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let team_id: String = context.evaluate_pin("team_id").await?;
        let display_name: String = context.evaluate_pin("display_name").await?;
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let membership_type: String = context
            .evaluate_pin("membership_type")
            .await
            .unwrap_or_else(|_| "standard".to_string());

        let body = json!({
            "displayName": display_name,
            "description": description,
            "membershipType": membership_type
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!("/teams/{}/channels", team_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(channel) = parse_channel(&body) {
                    context.set_pin_value("channel", json!(channel)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                context
                    .set_pin_value("error_message", json!(graph_error_message(resp).await))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
