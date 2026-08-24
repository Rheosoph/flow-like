use super::{
    graph::{graph_error_message, graph_get_json, graph_get_paginated_values, graph_version_url},
    provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider},
};
use crate::events::chat_event::{Attachment, ComplexAttachment};
use ahash::AHashSet;
use flow_like::flow::{
    board::Board,
    execution::{
        LogLevel,
        context::ExecutionContext,
        internal_node::InternalNode,
        log::{LogMessage, LogStat},
    },
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_model_provider::{response::Response, response_chunk::ResponseChunk};
use flow_like_types::{
    JsonSchema, Value, async_trait,
    futures::StreamExt,
    json::json,
    reqwest,
    sync::{DashMap, Mutex},
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

// =============================================================================
// Copilot Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotInteraction {
    pub id: String,
    pub session_id: String,
    pub request_id: Option<String>,
    pub app_class: String,
    pub interaction_type: String,
    pub conversation_type: Option<String>,
    pub created_date_time: String,
    pub locale: Option<String>,
    pub body_content: String,
    pub body_content_type: String,
    pub from_user_id: Option<String>,
    pub from_user_name: Option<String>,
    pub from_app_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeetingInsight {
    pub id: String,
    pub call_id: String,
    pub content_correlation_id: Option<String>,
    pub created_date_time: Option<String>,
    pub end_date_time: Option<String>,
    pub action_items: Vec<ActionItem>,
    pub meeting_notes: Vec<MeetingNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActionItem {
    pub id: Option<String>,
    pub description: String,
    pub owner_name: Option<String>,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeetingNote {
    pub id: Option<String>,
    pub content: String,
    pub created_date_time: Option<String>,
}

fn parse_interaction(value: &Value) -> Option<CopilotInteraction> {
    Some(CopilotInteraction {
        id: value["id"].as_str()?.to_string(),
        session_id: value["sessionId"].as_str()?.to_string(),
        request_id: value["requestId"].as_str().map(String::from),
        app_class: value["appClass"].as_str().unwrap_or("").to_string(),
        interaction_type: value["interactionType"].as_str().unwrap_or("").to_string(),
        conversation_type: value["conversationType"].as_str().map(String::from),
        created_date_time: value["createdDateTime"].as_str()?.to_string(),
        locale: value["locale"].as_str().map(String::from),
        body_content: value["body"]["content"].as_str().unwrap_or("").to_string(),
        body_content_type: value["body"]["contentType"]
            .as_str()
            .unwrap_or("text")
            .to_string(),
        from_user_id: value["from"]["user"]["id"].as_str().map(String::from),
        from_user_name: value["from"]["user"]["displayName"]
            .as_str()
            .map(String::from),
        from_app_name: value["from"]["application"]["displayName"]
            .as_str()
            .map(String::from),
    })
}

fn parse_action_item(value: &Value) -> Option<ActionItem> {
    Some(ActionItem {
        id: value["id"].as_str().map(String::from),
        description: value["description"].as_str()?.to_string(),
        owner_name: value["owner"]["displayName"].as_str().map(String::from),
        due_date: value["dueDateTime"].as_str().map(String::from),
    })
}

fn parse_meeting_note(value: &Value) -> Option<MeetingNote> {
    Some(MeetingNote {
        id: value["id"].as_str().map(String::from),
        content: value["content"].as_str()?.to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
    })
}

fn parse_meeting_insight(value: &Value) -> Option<MeetingInsight> {
    Some(MeetingInsight {
        id: value["id"].as_str()?.to_string(),
        call_id: value["callId"].as_str()?.to_string(),
        content_correlation_id: value["contentCorrelationId"].as_str().map(String::from),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        end_date_time: value["endDateTime"].as_str().map(String::from),
        action_items: value["actionItems"]
            .as_array()
            .map(|arr| arr.iter().filter_map(parse_action_item).collect())
            .unwrap_or_default(),
        meeting_notes: value["meetingNotes"]
            .as_array()
            .map(|arr| arr.iter().filter_map(parse_meeting_note).collect())
            .unwrap_or_default(),
    })
}

// =============================================================================
// Get Copilot Interactions Node (Interaction Export API)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetCopilotInteractionsNode {}

impl GetCopilotInteractionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetCopilotInteractionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_get_interactions",
            "Get Copilot Interactions",
            "Get Microsoft 365 Copilot interaction history (prompts and responses)",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "getInteractions");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "user_id",
            "User ID",
            "User ID to get interactions for",
            VariableType::String,
        );
        node.add_input_pin(
            "app_class_filter",
            "App Class Filter",
            "Filter by app (e.g., IPM.SkypeTeams.Message.Copilot.BizChat)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "top",
            "Top",
            "Maximum number of results",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "use_beta",
            "Use Beta API",
            "Use beta endpoint instead of v1.0",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("interactions", "Interactions", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<CopilotInteraction>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["AiEnterpriseInteraction.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let user_id: String = context.evaluate_pin("user_id").await?;
        let app_class_filter: String = context
            .evaluate_pin("app_class_filter")
            .await
            .unwrap_or_default();
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(100);
        let use_beta: bool = context.evaluate_pin("use_beta").await.unwrap_or(false);

        let api_version = if use_beta { "beta" } else { "v1.0" };
        let base_url = graph_version_url(
            &provider,
            api_version,
            &format!(
                "/copilot/users/{}/interactionHistory/getAllEnterpriseInteractions",
                user_id
            ),
        );
        let mut query_params = vec![("$top".to_string(), top.to_string())];
        if !app_class_filter.is_empty() {
            query_params.push((
                "$filter".to_string(),
                format!("appClass eq '{}'", app_class_filter),
            ));
        }
        let url = reqwest::Url::parse_with_params(&base_url, query_params)?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url.to_string()).await {
            Ok(values) => {
                let interactions: Vec<CopilotInteraction> =
                    values.iter().filter_map(parse_interaction).collect();
                let count = interactions.len() as i64;
                context
                    .set_pin_value("interactions", json!(interactions))
                    .await?;
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
// List Meeting Insights Node (Meeting Insights API)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListMeetingInsightsNode {}

impl ListMeetingInsightsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListMeetingInsightsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_list_meeting_insights",
            "List Meeting Insights",
            "Get AI-generated meeting notes and action items from Teams meetings",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "listMeetingInsights");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "meeting_id",
            "Meeting ID",
            "Online meeting ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("insights", "Insights", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<MeetingInsight>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(
            MICROSOFT_PROVIDER_ID,
            vec!["OnlineMeetings.Read", "OnlineMeetingAiInsight.Read.All"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let meeting_id: String = context.evaluate_pin("meeting_id").await?;

        let client = reqwest::Client::new();
        let url = graph_version_url(
            &provider,
            "beta",
            &format!("/me/onlineMeetings/{}/aiInsights", meeting_id),
        );

        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let insights: Vec<MeetingInsight> =
                    values.iter().filter_map(parse_meeting_insight).collect();
                let count = insights.len() as i64;
                context.set_pin_value("insights", json!(insights)).await?;
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
// Get Meeting Insight Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetMeetingInsightNode {}

impl GetMeetingInsightNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetMeetingInsightNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_get_meeting_insight",
            "Get Meeting Insight",
            "Get a specific AI insight from a Teams meeting",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "getMeetingInsight");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "meeting_id",
            "Meeting ID",
            "Online meeting ID",
            VariableType::String,
        );
        node.add_input_pin(
            "insight_id",
            "Insight ID",
            "AI insight ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("insight", "Insight", "", VariableType::Struct)
            .set_schema::<MeetingInsight>();
        node.add_output_pin("action_items", "Action Items", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<ActionItem>();
        node.add_output_pin("meeting_notes", "Meeting Notes", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<MeetingNote>();
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(
            MICROSOFT_PROVIDER_ID,
            vec!["OnlineMeetings.Read", "OnlineMeetingAiInsight.Read.All"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let meeting_id: String = context.evaluate_pin("meeting_id").await?;
        let insight_id: String = context.evaluate_pin("insight_id").await?;

        let client = reqwest::Client::new();
        let url = graph_version_url(
            &provider,
            "beta",
            &format!(
                "/me/onlineMeetings/{}/aiInsights/{}",
                meeting_id, insight_id
            ),
        );

        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
                if let Some(insight) = parse_meeting_insight(&body) {
                    context
                        .set_pin_value("action_items", json!(insight.action_items.clone()))
                        .await?;
                    context
                        .set_pin_value("meeting_notes", json!(insight.meeting_notes.clone()))
                        .await?;
                    context.set_pin_value("insight", json!(insight)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
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
// Microsoft Graph Search Node (Microsoft Search API)
// https://learn.microsoft.com/en-us/graph/api/resources/search-api-overview
// =============================================================================

/// Search hit from Microsoft Graph Search API
/// Based on https://learn.microsoft.com/en-us/graph/api/resources/searchhit
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphSearchHit {
    /// The internal identifier for the item (format varies by entity type)
    pub hit_id: String,
    /// The rank or order of the result
    pub rank: Option<i32>,
    /// A summary of the result, if available
    pub summary: Option<String>,
    /// ID of the result template used to render the search result
    pub result_template_id: Option<String>,
    /// The underlying resource data
    pub resource: GraphSearchResource,
}

/// Resource data from a search hit
/// Combines properties from driveItem, message, event, site, list, listItem, etc.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphSearchResource {
    /// The OData type of the resource (e.g., "#microsoft.graph.driveItem")
    #[serde(rename = "@odata.type")]
    pub odata_type: Option<String>,
    /// Unique identifier for the resource
    pub id: String,
    /// Display name (for files, sites, lists)
    pub name: Option<String>,
    /// Display name alternative
    pub display_name: Option<String>,
    /// Subject (for emails, events)
    pub subject: Option<String>,
    /// URL to access the resource in a browser
    pub web_url: Option<String>,
    /// Alternative web link (for emails)
    pub web_link: Option<String>,
    /// Date and time the item was created (ISO 8601)
    pub created_date_time: Option<String>,
    /// Date and time the item was last modified (ISO 8601)
    pub last_modified_date_time: Option<String>,
    /// Size in bytes (for files)
    pub size: Option<i64>,
    /// Parent reference information
    pub parent_reference: Option<GraphParentReference>,
    /// File-specific metadata (present if item is a file)
    pub file: Option<GraphFileFacet>,
    /// Folder-specific metadata (present if item is a folder)
    pub folder: Option<GraphFolderFacet>,
    /// Email sender information
    pub sender: Option<GraphRecipient>,
    /// Email from information
    pub from: Option<GraphRecipient>,
    /// Whether the email has attachments
    pub has_attachments: Option<bool>,
    /// Whether the email is read
    pub is_read: Option<bool>,
    /// Body preview (for emails)
    pub body_preview: Option<String>,
    /// Date and time email was received
    pub received_date_time: Option<String>,
    /// Date and time email was sent
    pub sent_date_time: Option<String>,
    /// Event start time
    pub start: Option<GraphDateTimeTimeZone>,
    /// Event end time
    pub end: Option<GraphDateTimeTimeZone>,
    /// Event location
    pub location: Option<GraphLocation>,
    /// Event organizer
    pub organizer: Option<GraphRecipient>,
}

/// Parent reference for drive items
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphParentReference {
    pub id: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub drive_id: Option<String>,
    pub site_id: Option<String>,
}

/// File facet for drive items
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphFileFacet {
    pub mime_type: Option<String>,
}

/// Folder facet for drive items
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphFolderFacet {
    pub child_count: Option<i32>,
}

/// Recipient information (for emails, events)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphRecipient {
    pub email_address: Option<GraphEmailAddress>,
}

/// Email address details
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphEmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// Date/time with timezone (for events)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphDateTimeTimeZone {
    pub date_time: Option<String>,
    pub time_zone: Option<String>,
}

/// Location information (for events)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GraphLocation {
    pub display_name: Option<String>,
}

fn parse_graph_search_hit(value: &Value) -> Option<GraphSearchHit> {
    let resource = &value["resource"];

    Some(GraphSearchHit {
        hit_id: value["hitId"].as_str().unwrap_or("").to_string(),
        rank: value["rank"].as_i64().map(|v| v as i32),
        summary: value["summary"].as_str().map(String::from),
        result_template_id: value["resultTemplateId"].as_str().map(String::from),
        resource: GraphSearchResource {
            odata_type: resource["@odata.type"].as_str().map(String::from),
            id: resource["id"].as_str().unwrap_or("").to_string(),
            name: resource["name"].as_str().map(String::from),
            display_name: resource["displayName"].as_str().map(String::from),
            subject: resource["subject"].as_str().map(String::from),
            web_url: resource["webUrl"].as_str().map(String::from),
            web_link: resource["webLink"].as_str().map(String::from),
            created_date_time: resource["createdDateTime"].as_str().map(String::from),
            last_modified_date_time: resource["lastModifiedDateTime"].as_str().map(String::from),
            size: resource["size"].as_i64(),
            parent_reference: if resource["parentReference"].is_object() {
                Some(GraphParentReference {
                    id: resource["parentReference"]["id"].as_str().map(String::from),
                    name: resource["parentReference"]["name"]
                        .as_str()
                        .map(String::from),
                    path: resource["parentReference"]["path"]
                        .as_str()
                        .map(String::from),
                    drive_id: resource["parentReference"]["driveId"]
                        .as_str()
                        .map(String::from),
                    site_id: resource["parentReference"]["siteId"]
                        .as_str()
                        .map(String::from),
                })
            } else {
                None
            },
            file: if resource["file"].is_object() {
                Some(GraphFileFacet {
                    mime_type: resource["file"]["mimeType"].as_str().map(String::from),
                })
            } else {
                None
            },
            folder: if resource["folder"].is_object() {
                Some(GraphFolderFacet {
                    child_count: resource["folder"]["childCount"].as_i64().map(|v| v as i32),
                })
            } else {
                None
            },
            sender: parse_graph_recipient(&resource["sender"]),
            from: parse_graph_recipient(&resource["from"]),
            has_attachments: resource["hasAttachments"].as_bool(),
            is_read: resource["isRead"].as_bool(),
            body_preview: resource["bodyPreview"].as_str().map(String::from),
            received_date_time: resource["receivedDateTime"].as_str().map(String::from),
            sent_date_time: resource["sentDateTime"].as_str().map(String::from),
            start: if resource["start"].is_object() {
                Some(GraphDateTimeTimeZone {
                    date_time: resource["start"]["dateTime"].as_str().map(String::from),
                    time_zone: resource["start"]["timeZone"].as_str().map(String::from),
                })
            } else {
                None
            },
            end: if resource["end"].is_object() {
                Some(GraphDateTimeTimeZone {
                    date_time: resource["end"]["dateTime"].as_str().map(String::from),
                    time_zone: resource["end"]["timeZone"].as_str().map(String::from),
                })
            } else {
                None
            },
            location: if resource["location"].is_object() {
                Some(GraphLocation {
                    display_name: resource["location"]["displayName"]
                        .as_str()
                        .map(String::from),
                })
            } else {
                None
            },
            organizer: parse_graph_recipient(&resource["organizer"]),
        },
    })
}

fn parse_graph_recipient(value: &Value) -> Option<GraphRecipient> {
    if !value.is_object() {
        return None;
    }
    Some(GraphRecipient {
        email_address: if value["emailAddress"].is_object() {
            Some(GraphEmailAddress {
                name: value["emailAddress"]["name"].as_str().map(String::from),
                address: value["emailAddress"]["address"].as_str().map(String::from),
            })
        } else {
            None
        },
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct GraphSearchNode {}

impl GraphSearchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GraphSearchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_graph_search",
            "Microsoft Search",
            "Search across Microsoft 365 content using the Microsoft Graph Search API. Supports files, emails, calendar events, Teams messages, SharePoint sites, and more.",
            "Data/Microsoft/Search",
        );
        node.set_flowscript_name("microsoft.search", "query");
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

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
            "query",
            "Query",
            "Search query (supports KQL syntax for advanced queries)",
            VariableType::String,
        );
        node.add_input_pin(
            "entity_types",
            "Entity Types",
            "Comma-separated entity types to search",
            VariableType::String,
        )
        .set_default_value(Some(json!("driveItem")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "driveItem".to_string(),
                    "message".to_string(),
                    "chatMessage".to_string(),
                    "event".to_string(),
                    "site".to_string(),
                    "list".to_string(),
                    "listItem".to_string(),
                    "drive".to_string(),
                    "acronym".to_string(),
                    "bookmark".to_string(),
                    "qna".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "size",
            "Size",
            "Maximum number of results per page (default: 25, max: 1000 for SharePoint/OneDrive, 25 for message/event)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));
        node.add_input_pin(
            "from",
            "From",
            "Starting offset for pagination (0-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "fields",
            "Fields",
            "Comma-separated list of fields to return (empty for default)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "results",
            "Results",
            "Search results (array of GraphSearchHit)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GraphSearchHit>();
        node.add_output_pin(
            "count",
            "Count",
            "Number of results returned",
            VariableType::Integer,
        );
        node.add_output_pin(
            "total",
            "Total",
            "Total estimated results available",
            VariableType::Integer,
        );
        node.add_output_pin(
            "more_results",
            "More Results",
            "Whether more results are available",
            VariableType::Boolean,
        );
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        // Permissions vary by entity type, include common ones
        node.add_required_oauth_scopes(
            MICROSOFT_PROVIDER_ID,
            vec![
                "Files.Read.All",
                "Mail.Read",
                "Calendars.Read",
                "Sites.Read.All",
                "Chat.Read",
            ],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;
        let entity_types: String = context
            .evaluate_pin("entity_types")
            .await
            .unwrap_or_else(|_| "driveItem".to_string());
        let size: i64 = context.evaluate_pin("size").await.unwrap_or(25);
        let from: i64 = context.evaluate_pin("from").await.unwrap_or(0);
        let fields: String = context.evaluate_pin("fields").await.unwrap_or_default();

        let entity_list: Vec<String> = entity_types
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let client = reqwest::Client::new();

        // For driveItem-only searches, try the Drive API first (more reliable)
        let is_drive_only = entity_list.len() == 1 && entity_list[0] == "driveItem";

        if is_drive_only {
            // Try OneDrive search API first - it's more universally available
            match search_onedrive(&client, &provider, &query, size as u32).await {
                Ok((results, total)) => {
                    let count = results.len() as i64;
                    context.set_pin_value("results", json!(results)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("total", json!(total)).await?;
                    context
                        .set_pin_value("more_results", json!(total > count))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    return Ok(());
                }
                Err(_) => {
                    // Fall through to try the Search API
                }
            }
        }

        // Check if we're searching only message types (enableTopResults only works for message/chatMessage)
        let is_message_only = entity_list
            .iter()
            .all(|t| t == "message" || t == "chatMessage");

        let mut request = json!({
            "entityTypes": entity_list,
            "query": {
                "queryString": query
            },
            "from": from,
            "size": size
        });

        // Only add enableTopResults for message searches
        if is_message_only && !entity_list.is_empty() {
            request["enableTopResults"] = json!(true);
        }

        // Add fields if specified
        if !fields.is_empty() {
            let field_list: Vec<String> = fields
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !field_list.is_empty() {
                request["fields"] = json!(field_list);
            }
        }

        let request_body = json!({
            "requests": [request]
        });

        // Retry logic for transient errors (SearchPlatformResolutionFailed is often transient)
        let max_retries = 3;
        let mut last_error = String::new();

        for attempt in 0..max_retries {
            if attempt > 0 {
                // Exponential backoff: 500ms, 1s, 2s
                tokio::time::sleep(tokio::time::Duration::from_millis(500 * (1 << attempt))).await;
            }

            let response = client
                .post(provider.api_url("/search/query"))
                .header("Authorization", format!("Bearer {}", provider.access_token))
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await?;
                    let mut results: Vec<GraphSearchHit> = Vec::new();
                    let mut total: i64 = 0;
                    let mut more_results = false;

                    if let Some(responses) = body["value"].as_array() {
                        for response in responses {
                            if let Some(hits_containers) = response["hitsContainers"].as_array() {
                                for container in hits_containers {
                                    if let Some(t) = container["total"].as_i64() {
                                        total = t;
                                    }
                                    if let Some(more) = container["moreResultsAvailable"].as_bool()
                                    {
                                        more_results = more;
                                    }

                                    if let Some(hits) = container["hits"].as_array() {
                                        for hit in hits {
                                            if let Some(result) = parse_graph_search_hit(hit) {
                                                results.push(result);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let count = results.len() as i64;
                    context.set_pin_value("results", json!(results)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("total", json!(total)).await?;
                    context
                        .set_pin_value("more_results", json!(more_results))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    return Ok(());
                }
                Ok(resp) => {
                    let status = resp.status();
                    let error = resp.text().await.unwrap_or_default();

                    // Check for retryable errors
                    let is_retryable = status.as_u16() == 500
                        || status.as_u16() == 503
                        || status.as_u16() == 429
                        || error.contains("SearchPlatformResolutionFailed")
                        || error.contains("ServiceUnavailable")
                        || error.contains("TooManyRequests");

                    if is_retryable && attempt < max_retries - 1 {
                        last_error = format_search_error(&error, &entity_list);
                        continue;
                    }

                    context
                        .set_pin_value(
                            "error_message",
                            json!(format_search_error(&error, &entity_list)),
                        )
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
                Err(e) => {
                    if attempt < max_retries - 1 {
                        last_error = e.to_string();
                        continue;
                    }
                    context
                        .set_pin_value("error_message", json!(e.to_string()))
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            }
        }

        // If we exhausted retries, output the last error
        context
            .set_pin_value("error_message", json!(last_error))
            .await?;
        context.activate_exec_pin("error").await?;
        Ok(())
    }
}

/// Search OneDrive using the Drive API (more reliable than Microsoft Search API)
/// This uses GET /me/drive/root/search(q='{query}')
async fn search_onedrive(
    client: &reqwest::Client,
    provider: &MicrosoftGraphProvider,
    query: &str,
    top: u32,
) -> Result<(Vec<GraphSearchHit>, i64), String> {
    let url = format!(
        "{}?$top={}",
        provider.api_url(&format!(
            "/me/drive/root/search(q='{}')",
            urlencoding::encode(query)
        )),
        top
    );

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", provider.access_token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(response.text().await.unwrap_or_default());
    }

    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let mut results = Vec::new();

    if let Some(items) = body["value"].as_array() {
        for (index, item) in items.iter().enumerate() {
            results.push(GraphSearchHit {
                hit_id: item["id"].as_str().unwrap_or("").to_string(),
                rank: Some(index as i32),
                summary: item["description"].as_str().map(String::from),
                result_template_id: None,
                resource: GraphSearchResource {
                    odata_type: Some("#microsoft.graph.driveItem".to_string()),
                    id: item["id"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().map(String::from),
                    display_name: None,
                    subject: None,
                    web_url: item["webUrl"].as_str().map(String::from),
                    web_link: None,
                    created_date_time: item["createdDateTime"].as_str().map(String::from),
                    last_modified_date_time: item["lastModifiedDateTime"]
                        .as_str()
                        .map(String::from),
                    size: item["size"].as_i64(),
                    parent_reference: if item["parentReference"].is_object() {
                        Some(GraphParentReference {
                            id: item["parentReference"]["id"].as_str().map(String::from),
                            name: item["parentReference"]["name"].as_str().map(String::from),
                            path: item["parentReference"]["path"].as_str().map(String::from),
                            drive_id: item["parentReference"]["driveId"]
                                .as_str()
                                .map(String::from),
                            site_id: item["parentReference"]["siteId"].as_str().map(String::from),
                        })
                    } else {
                        None
                    },
                    file: if item["file"].is_object() {
                        Some(GraphFileFacet {
                            mime_type: item["file"]["mimeType"].as_str().map(String::from),
                        })
                    } else {
                        None
                    },
                    folder: if item["folder"].is_object() {
                        Some(GraphFolderFacet {
                            child_count: item["folder"]["childCount"].as_i64().map(|v| v as i32),
                        })
                    } else {
                        None
                    },
                    sender: None,
                    from: None,
                    has_attachments: None,
                    is_read: None,
                    body_preview: None,
                    received_date_time: None,
                    sent_date_time: None,
                    start: None,
                    end: None,
                    location: None,
                    organizer: None,
                },
            });
        }
    }

    let total = body["@odata.count"]
        .as_i64()
        .unwrap_or(results.len() as i64);
    Ok((results, total))
}

/// Format search error with helpful suggestions
fn format_search_error(error: &str, entity_types: &[String]) -> String {
    if error.contains("SearchPlatformResolutionFailed") {
        let mut msg =
            "Microsoft Search Platform Error (SearchPlatformResolutionFailed).\n\n".to_string();
        msg.push_str("This is a server-side error from Microsoft. Common causes:\n");
        msg.push_str("1. SharePoint search index may not be ready for this tenant\n");
        msg.push_str("2. The entity type may not have searchable content yet\n");
        msg.push_str("3. Permissions may not be fully propagated\n\n");
        msg.push_str("Suggestions:\n");
        msg.push_str(
            "- Try a different entity type (e.g., 'driveItem' for OneDrive/SharePoint files)\n",
        );
        msg.push_str("- Wait a few minutes and retry\n");
        msg.push_str("- Ensure the account has content in the selected entity type\n");
        if entity_types.contains(&"message".to_string())
            || entity_types.contains(&"chatMessage".to_string())
        {
            msg.push_str("- For message/chat search, ensure Mail.Read or Chat.Read permissions\n");
        }
        msg.push_str("\nOriginal error: ");
        msg.push_str(error);
        msg
    } else if error.contains("AccessDenied") || error.contains("Authorization") {
        format!(
            "Access denied. Ensure the required permissions are granted:\n- Files.Read.All (for driveItem, site, list)\n- Mail.Read (for message)\n- Chat.Read (for chatMessage)\n- Calendars.Read (for event)\n\nOriginal error: {}",
            error
        )
    } else {
        error.to_string()
    }
}

// =============================================================================
// Copilot Chat API Types
// Based on https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/chat/copilotconversation-chatoverstream
// =============================================================================

/// Attribution from Copilot response (citations and annotations)
/// Attributions provide source references for information in Copilot's response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotAttribution {
    /// Type of attribution: "citation" (document/file reference) or "annotation" (inline reference)
    pub attribution_type: String,
    /// Display name of the content provider or source
    pub provider_display_name: Option<String>,
    /// Source of the attribution: "model" (AI generated), "grounding" (from user's data)
    pub attribution_source: Option<String>,
    /// URL to view more details about the referenced content
    pub see_more_web_url: Option<String>,
    /// URL to an image associated with the attribution
    pub image_web_url: Option<String>,
    /// Favicon URL for the source
    pub image_fav_icon: Option<String>,
    /// Width of the image in pixels
    pub image_width: Option<i32>,
    /// Height of the image in pixels
    pub image_height: Option<i32>,
}

/// Sensitivity label information from Copilot
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotSensitivityLabel {
    pub sensitivity_label_id: Option<String>,
    pub display_name: Option<String>,
    pub tooltip: Option<String>,
    pub priority: Option<i32>,
    pub color: Option<String>,
    pub is_encrypted: Option<bool>,
}

/// Adaptive Card from Copilot response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotAdaptiveCard {
    pub card_type: String,
    pub version: String,
    pub body: Vec<Value>,
}

/// Message from Copilot conversation response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotConversationMessage {
    pub id: String,
    pub text: String,
    pub created_date_time: Option<String>,
    pub attributions: Vec<CopilotAttribution>,
    pub adaptive_cards: Vec<CopilotAdaptiveCard>,
    pub sensitivity_label: Option<CopilotSensitivityLabel>,
}

/// Full Copilot Chat response structure
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotChatResponse {
    pub id: String,
    pub content: String,
    pub created_date_time: Option<String>,
    pub turn_count: i32,
    pub state: String,
    pub messages: Vec<CopilotConversationMessage>,
}

fn parse_attribution(value: &Value) -> Option<CopilotAttribution> {
    Some(CopilotAttribution {
        attribution_type: value["attributionType"].as_str()?.to_string(),
        provider_display_name: value["providerDisplayName"].as_str().map(String::from),
        attribution_source: value["attributionSource"].as_str().map(String::from),
        see_more_web_url: value["seeMoreWebUrl"].as_str().map(String::from),
        image_web_url: value["imageWebUrl"].as_str().map(String::from),
        image_fav_icon: value["imageFavIcon"].as_str().map(String::from),
        image_width: value["imageWidth"].as_i64().map(|v| v as i32),
        image_height: value["imageHeight"].as_i64().map(|v| v as i32),
    })
}

fn parse_adaptive_card(value: &Value) -> Option<CopilotAdaptiveCard> {
    Some(CopilotAdaptiveCard {
        card_type: value["type"].as_str().unwrap_or("AdaptiveCard").to_string(),
        version: value["version"].as_str().unwrap_or("1.0").to_string(),
        body: value["body"].as_array().cloned().unwrap_or_default(),
    })
}

fn parse_copilot_message(value: &Value) -> Option<CopilotConversationMessage> {
    Some(CopilotConversationMessage {
        id: value["id"].as_str()?.to_string(),
        text: value["text"].as_str().unwrap_or("").to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        attributions: value["attributions"]
            .as_array()
            .map(|arr| arr.iter().filter_map(parse_attribution).collect())
            .unwrap_or_default(),
        adaptive_cards: value["adaptiveCards"]
            .as_array()
            .map(|arr| arr.iter().filter_map(parse_adaptive_card).collect())
            .unwrap_or_default(),
        sensitivity_label: if value["sensitivityLabel"].is_null() {
            None
        } else {
            Some(CopilotSensitivityLabel {
                sensitivity_label_id: value["sensitivityLabel"]["sensitivityLabelId"]
                    .as_str()
                    .map(String::from),
                display_name: value["sensitivityLabel"]["displayName"]
                    .as_str()
                    .map(String::from),
                tooltip: value["sensitivityLabel"]["toolTip"]
                    .as_str()
                    .map(String::from),
                priority: value["sensitivityLabel"]["priority"]
                    .as_i64()
                    .map(|v| v as i32),
                color: value["sensitivityLabel"]["color"]
                    .as_str()
                    .map(String::from),
                is_encrypted: value["sensitivityLabel"]["isEncrypted"].as_bool(),
            })
        },
    })
}

/// Convert attributions to annotations for Response
fn attributions_to_annotations(
    attributions: &[CopilotAttribution],
) -> Vec<flow_like_model_provider::response::Annotation> {
    attributions
        .iter()
        .filter(|a| a.attribution_type == "citation" && a.see_more_web_url.is_some())
        .map(|a| {
            // Create annotation using serde_json since Annotation fields are private
            let annotation_json = json!({
                "type": "url_citation",
                "url_citation": {
                    "start_index": 0,
                    "end_index": 0,
                    "title": a.provider_display_name.clone().unwrap_or_default(),
                    "url": a.see_more_web_url.clone().unwrap_or_default()
                }
            });
            flow_like_types::json::from_value(annotation_json).unwrap_or_else(|_| {
                flow_like_types::json::from_value(json!({
                    "type": "url_citation",
                    "url_citation": null
                }))
                .unwrap()
            })
        })
        .collect()
}

/// Convert adaptive cards to markdown
#[allow(dead_code)] // adaptive-card → markdown rendering is parsed (parse_adaptive_card) but not yet surfaced on a pin
fn adaptive_cards_to_markdown(cards: &[CopilotAdaptiveCard]) -> String {
    let mut markdown = String::new();
    for card in cards {
        for block in &card.body {
            if let Some(text) = block["text"].as_str() {
                if !markdown.is_empty() {
                    markdown.push_str("\n\n");
                }
                markdown.push_str(text);
            }
        }
    }
    markdown
}

// =============================================================================
// IANA Timezone Helpers
// =============================================================================

/// Get the system's IANA timezone, falling back to "Etc/UTC" if detection fails
fn get_system_timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "Etc/UTC".to_string())
}

/// Validate and normalize a timezone string to a valid IANA timezone.
/// Returns the closest matching IANA timezone or None if no reasonable match found.
fn normalize_timezone(input: &str) -> Option<String> {
    let input_trimmed = input.trim();

    // Empty input - use system timezone
    if input_trimmed.is_empty() {
        return Some(get_system_timezone());
    }

    // Try exact match first (case-insensitive)
    let input_lower = input_trimmed.to_lowercase();
    for tz in chrono_tz::TZ_VARIANTS {
        if tz.name().to_lowercase() == input_lower {
            return Some(tz.name().to_string());
        }
    }

    // Handle common abbreviations and special cases
    let normalized = match input_lower.as_str() {
        "utc" | "gmt" | "z" => "Etc/UTC",
        "est" => "America/New_York",
        "edt" => "America/New_York",
        "cst" => "America/Chicago",
        "cdt" => "America/Chicago",
        "mst" => "America/Denver",
        "mdt" => "America/Denver",
        "pst" => "America/Los_Angeles",
        "pdt" => "America/Los_Angeles",
        "cet" => "Europe/Paris",
        "cest" => "Europe/Paris",
        "gmt+0" | "gmt-0" | "utc+0" | "utc-0" => "Etc/UTC",
        "bst" => "Europe/London",
        "ist" => "Asia/Kolkata",
        "jst" => "Asia/Tokyo",
        "kst" => "Asia/Seoul",
        "cst china" | "china" => "Asia/Shanghai",
        "aest" => "Australia/Sydney",
        "aedt" => "Australia/Sydney",
        "nzst" => "Pacific/Auckland",
        "nzdt" => "Pacific/Auckland",
        _ => "",
    };

    if !normalized.is_empty() {
        return Some(normalized.to_string());
    }

    // Handle UTC offset formats like "UTC+5", "GMT-8", etc.
    if let Some(offset_str) = input_lower
        .strip_prefix("utc")
        .or_else(|| input_lower.strip_prefix("gmt"))
        && let Some(offset) = parse_utc_offset(offset_str)
    {
        return Some(utc_offset_to_iana(offset));
    }

    // Try fuzzy matching - find timezone containing the input
    let mut best_match: Option<&str> = None;
    let mut best_score = 0usize;

    for tz in chrono_tz::TZ_VARIANTS {
        let tz_name_lower = tz.name().to_lowercase();

        // Check if input is a substring
        if tz_name_lower.contains(&input_lower) {
            let score = input_lower.len() * 10; // Prefer longer matches
            if score > best_score {
                best_score = score;
                best_match = Some(tz.name());
            }
        }

        // Check if any part matches (e.g., "New_York" matches "America/New_York")
        let parts: Vec<&str> = tz_name_lower.split('/').collect();
        for part in parts {
            if part == input_lower.replace(' ', "_") {
                return Some(tz.name().to_string());
            }
        }
    }

    best_match.map(|s| s.to_string())
}

/// Parse UTC offset string like "+5", "-8:30", "+05:30"
fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }

    let (sign, rest) = if let Some(r) = s.strip_prefix('+') {
        (1, r)
    } else if let Some(r) = s.strip_prefix('-') {
        (-1, r)
    } else {
        (1, s)
    };

    let parts: Vec<&str> = rest.split(':').collect();
    let hours: i32 = parts.first()?.parse().ok()?;
    let minutes: i32 = parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);

    Some(sign * (hours * 60 + minutes))
}

/// Convert UTC offset in minutes to closest IANA timezone
fn utc_offset_to_iana(offset_minutes: i32) -> String {
    // Use Etc/GMT timezones (note: signs are inverted in Etc/GMT)
    let hours = offset_minutes / 60;
    if offset_minutes % 60 == 0 && (-12..=14).contains(&hours) {
        if hours == 0 {
            "Etc/UTC".to_string()
        } else if hours > 0 {
            // Etc/GMT uses inverted signs
            format!("Etc/GMT-{}", hours)
        } else {
            format!("Etc/GMT+{}", -hours)
        }
    } else {
        // For non-whole hour offsets, try to find a matching timezone
        // Common half-hour offsets
        match offset_minutes {
            330 => "Asia/Kolkata".to_string(),       // +5:30
            345 => "Asia/Kathmandu".to_string(),     // +5:45
            -210 => "America/St_Johns".to_string(),  // -3:30
            570 => "Australia/Adelaide".to_string(), // +9:30
            _ => "Etc/UTC".to_string(),
        }
    }
}

/// Get the Microsoft Graph Search API region code based on system timezone.
/// Maps IANA timezone to the appropriate Microsoft datacenter region.
#[allow(dead_code)] // staged: Graph Search
fn get_system_region() -> String {
    let timezone = get_system_timezone();
    timezone_to_region(&timezone)
}

/// Map an IANA timezone to Microsoft Graph Search API region code.
#[allow(dead_code)]
fn timezone_to_region(timezone: &str) -> String {
    let tz_lower = timezone.to_lowercase();

    // Extract the region prefix (e.g., "America", "Europe", "Asia")
    let region = if tz_lower.starts_with("america/") {
        // North/South America
        if tz_lower.contains("sao_paulo")
            || tz_lower.contains("brasilia")
            || tz_lower.contains("fortaleza")
            || tz_lower.contains("recife")
            || tz_lower.contains("belem")
            || tz_lower.contains("manaus")
            || tz_lower.contains("cuiaba")
            || tz_lower.contains("porto_velho")
            || tz_lower.contains("rio_branco")
            || tz_lower.starts_with("america/brazil")
        {
            "BRA" // Brazil
        } else if tz_lower.contains("toronto")
            || tz_lower.contains("vancouver")
            || tz_lower.contains("montreal")
            || tz_lower.contains("winnipeg")
            || tz_lower.contains("edmonton")
            || tz_lower.contains("calgary")
            || tz_lower.contains("halifax")
            || tz_lower.contains("st_johns")
            || tz_lower.starts_with("canada/")
        {
            "CAN" // Canada
        } else {
            "NAM" // North America (US, Mexico, etc.)
        }
    } else if tz_lower.starts_with("europe/") {
        // European regions
        if tz_lower.contains("london")
            || tz_lower.contains("belfast")
            || tz_lower.contains("isle_of_man")
            || tz_lower.contains("jersey")
            || tz_lower.contains("guernsey")
        {
            "GBR" // United Kingdom
        } else if tz_lower.contains("paris")
            || tz_lower.contains("lyon")
            || tz_lower.contains("marseille")
        {
            "FRA" // France
        } else if tz_lower.contains("berlin")
            || tz_lower.contains("munich")
            || tz_lower.contains("frankfurt")
            || tz_lower.contains("hamburg")
        {
            "DEU" // Germany
        } else if tz_lower.contains("zurich")
            || tz_lower.contains("geneva")
            || tz_lower.contains("bern")
        {
            "CHE" // Switzerland
        } else if tz_lower.contains("oslo") || tz_lower.contains("tromso") {
            "NOR" // Norway
        } else {
            "EUR" // General Europe
        }
    } else if tz_lower.starts_with("asia/") {
        // Asian regions
        if tz_lower.contains("tokyo") || tz_lower.contains("osaka") {
            "JPN" // Japan
        } else if tz_lower.contains("seoul") {
            "KOR" // Korea
        } else if tz_lower.contains("kolkata")
            || tz_lower.contains("mumbai")
            || tz_lower.contains("chennai")
            || tz_lower.contains("delhi")
            || tz_lower.contains("bangalore")
            || tz_lower.contains("calcutta")
        {
            "IND" // India
        } else if tz_lower.contains("dubai")
            || tz_lower.contains("abu_dhabi")
            || tz_lower.contains("muscat")
        {
            "ARE" // UAE
        } else {
            "APC" // Asia Pacific
        }
    } else if tz_lower.starts_with("australia/")
        || tz_lower.starts_with("pacific/auckland")
        || tz_lower.starts_with("pacific/fiji")
    {
        if tz_lower.starts_with("pacific/auckland") || tz_lower.starts_with("pacific/chatham") {
            "APC" // New Zealand goes to APC
        } else {
            "AUS" // Australia
        }
    } else if tz_lower.starts_with("africa/") {
        if tz_lower.contains("johannesburg")
            || tz_lower.contains("cape_town")
            || tz_lower.contains("durban")
            || tz_lower.contains("pretoria")
        {
            "ZAF" // South Africa
        } else {
            "EUR" // Other African timezones default to Europe region
        }
    } else if tz_lower.starts_with("pacific/") {
        "APC" // Pacific islands default to Asia Pacific
    } else if tz_lower.starts_with("atlantic/") {
        "EUR" // Atlantic timezones default to Europe
    } else {
        "NAM" // Default to North America
    };

    region.to_string()
}

// =============================================================================
// Copilot Chat Node (Chat API - Preview)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CopilotChatNode {}

impl CopilotChatNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CopilotChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_chat",
            "Copilot Chat",
            "Send a message to Microsoft 365 Copilot using the official Chat API with streaming support. Supports file context from OneDrive/SharePoint and web search grounding.",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "chat");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "prompt",
            "Prompt",
            "User message to send to Copilot",
            VariableType::String,
        );
        node.add_input_pin(
            "additional_context",
            "Additional Context",
            "Extra grounding context (e.g., document excerpts, facts) to provide to Copilot",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "file_urls",
            "File URLs",
            "OneDrive/SharePoint file URLs to include as context (full URLs like https://contoso.sharepoint.com/...)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "web_grounding",
            "Web Search",
            "Enable web search grounding for real-time information",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "timezone",
            "Timezone",
            "User timezone in IANA format (e.g., America/New_York, Europe/London). Auto-detected from system if empty.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "conversation_id",
            "Conversation ID",
            "Optional conversation ID to continue a chat (leave empty for new conversation)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "on_stream",
            "On Stream",
            "Triggers on streaming output",
            VariableType::Execution,
        );
        node.add_output_pin("chunk", "Chunk", "Streaming chunk", VariableType::Struct)
            .set_schema::<ResponseChunk>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("done", "Done", "Completed", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "result",
            "Result",
            "Complete response with annotations from citations",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "response",
            "RAW Response",
            "Full Copilot response with attributions and adaptive cards",
            VariableType::Struct,
        )
        .set_schema::<CopilotChatResponse>();
        node.add_output_pin(
            "attachments",
            "Attachments",
            "Attachments created from Copilot's attributions (citations and references)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Attachment>();
        node.add_output_pin(
            "new_conversation_id",
            "Conversation ID",
            "Conversation ID for follow-up messages",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        // Official permissions required for Copilot Chat API
        // https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/chat/copilotconversation-chatoverstream
        node.add_required_oauth_scopes(
            MICROSOFT_PROVIDER_ID,
            vec![
                "Sites.Read.All",
                "Mail.Read",
                "People.Read.All",
                "OnlineMeetingTranscript.Read.All",
                "Chat.Read",
                "ChannelMessage.Read.All",
                "ExternalItem.Read.All",
            ],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("done").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        let additional_context: Vec<String> = context
            .evaluate_pin("additional_context")
            .await
            .unwrap_or_default();
        let file_urls: Vec<String> = context.evaluate_pin("file_urls").await.unwrap_or_default();
        let web_grounding: bool = context.evaluate_pin("web_grounding").await.unwrap_or(true);
        let timezone_input: String = context.evaluate_pin("timezone").await.unwrap_or_default();
        let timezone = normalize_timezone(&timezone_input).unwrap_or_else(get_system_timezone);
        let mut conversation_id: String = context
            .evaluate_pin("conversation_id")
            .await
            .unwrap_or_default();

        let on_stream = context.get_pin_by_name("on_stream").await?;
        context.activate_exec_pin_ref(&on_stream).await?;

        let connected_nodes = Arc::new(DashMap::new());
        let connected = on_stream.get_connected_nodes();
        for node in connected {
            let sub_ctx = Arc::new(Mutex::new(context.create_sub_context(&node).await));
            connected_nodes.insert(node.node.lock().await.id.clone(), sub_ctx);
        }

        let parent_node_id = context.node.node.lock().await.id.clone();
        let callback_count = Arc::new(AtomicUsize::new(0));
        let client = reqwest::Client::new();

        // Step 1: Create or use existing conversation
        // https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/chat/copilotroot-post-conversations
        if conversation_id.is_empty() {
            let create_response = client
                .post(graph_version_url(
                    &provider,
                    "beta",
                    "/copilot/conversations",
                ))
                .header("Authorization", format!("Bearer {}", provider.access_token))
                .header("Content-Type", "application/json")
                .json(&json!({}))
                .send()
                .await;

            match create_response {
                Ok(resp) if resp.status().is_success() => {
                    let body: Value = resp.json().await?;
                    conversation_id = body["id"].as_str().unwrap_or("").to_string();
                    if conversation_id.is_empty() {
                        context
                            .set_pin_value(
                                "error_message",
                                json!("Failed to create conversation: no ID returned"),
                            )
                            .await?;
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }
                }
                Ok(resp) => {
                    let error = resp.text().await.unwrap_or_default();
                    context
                        .set_pin_value(
                            "error_message",
                            json!(format!("Failed to create conversation: {}", error)),
                        )
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
                Err(e) => {
                    context
                        .set_pin_value(
                            "error_message",
                            json!(format!("Failed to create conversation: {}", e)),
                        )
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            }
        }

        // Step 2: Build chatOverStream request
        // https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/chat/copilotconversation-chatoverstream
        let mut request_body = json!({
            "message": {
                "text": prompt
            },
            "locationHint": {
                "timeZone": timezone
            }
        });

        // Add additional context if provided
        if !additional_context.is_empty() {
            let context_messages: Vec<Value> = additional_context
                .iter()
                .map(|text| json!({ "text": text }))
                .collect();
            request_body["additionalContext"] = json!(context_messages);
        }

        // Add contextual resources (files and web grounding)
        let has_files = !file_urls.is_empty();
        let needs_web_config = !web_grounding;

        if has_files || needs_web_config {
            let mut contextual_resources = json!({});

            if has_files {
                let files: Vec<Value> = file_urls.iter().map(|url| json!({ "uri": url })).collect();
                contextual_resources["files"] = json!(files);
            }

            if needs_web_config {
                contextual_resources["webContext"] = json!({
                    "isWebEnabled": false
                });
            }

            request_body["contextualResources"] = contextual_resources;
        }

        let chat_url = graph_version_url(
            &provider,
            "beta",
            &format!("/copilot/conversations/{}/chatOverStream", conversation_id),
        );

        let response = client
            .post(&chat_url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        let mut message =
            LogMessage::new("Invoking Microsoft 365 Copilot Chat", LogLevel::Info, None);
        let mut full_content = String::new();
        let mut all_messages: Vec<CopilotConversationMessage> = Vec::new();
        let mut all_attributions: Vec<CopilotAttribution> = Vec::new();
        let mut response_id = conversation_id.clone();
        let mut turn_count = 0;
        let mut state = "active".to_string();
        let mut last_created_date_time: Option<String> = None;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let mut stream = resp.bytes_stream();
                let mut buffer = String::new();

                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            let text = String::from_utf8_lossy(&chunk);
                            buffer.push_str(&text);

                            // Process complete SSE events
                            while let Some(data_start) = buffer.find("data: ") {
                                let data_content = &buffer[data_start + 6..];

                                // Find end of JSON object (next "id:" or "data:" line)
                                let end_pos = data_content
                                    .find("\nid:")
                                    .or_else(|| data_content.find("\ndata:"))
                                    .unwrap_or(data_content.len());

                                let json_str = data_content[..end_pos].trim();

                                if json_str.is_empty() {
                                    buffer = buffer[data_start + 6 + end_pos..].to_string();
                                    continue;
                                }

                                // Try to parse the JSON
                                match flow_like_types::json::from_str::<Value>(json_str) {
                                    Ok(parsed) => {
                                        // Extract conversation metadata
                                        if let Some(id) = parsed["id"].as_str() {
                                            response_id = id.to_string();
                                        }
                                        if let Some(tc) = parsed["turnCount"].as_i64() {
                                            turn_count = tc as i32;
                                        }
                                        if let Some(s) = parsed["state"].as_str() {
                                            state = s.to_string();
                                        }
                                        if let Some(dt) = parsed["createdDateTime"].as_str() {
                                            last_created_date_time = Some(dt.to_string());
                                        }

                                        // Process messages in this SSE event
                                        if let Some(messages_arr) = parsed["messages"].as_array() {
                                            for msg_value in messages_arr {
                                                // Only process response messages (AI responses)
                                                let odata_type =
                                                    msg_value["@odata.type"].as_str().unwrap_or("");
                                                if !odata_type.contains("ResponseMessage") {
                                                    continue;
                                                }

                                                if let Some(copilot_msg) =
                                                    parse_copilot_message(msg_value)
                                                {
                                                    // Check if this is new content
                                                    let msg_text = &copilot_msg.text;
                                                    if !msg_text.is_empty()
                                                        && !full_content.contains(msg_text)
                                                    {
                                                        // Calculate the delta (new content)
                                                        let delta = if full_content.is_empty() {
                                                            msg_text.clone()
                                                        } else if msg_text
                                                            .starts_with(&full_content)
                                                        {
                                                            msg_text[full_content.len()..]
                                                                .to_string()
                                                        } else {
                                                            msg_text.clone()
                                                        };

                                                        if !delta.is_empty() {
                                                            full_content = msg_text.clone();

                                                            // Stream the chunk
                                                            let chunk = ResponseChunk::from_text(
                                                                &delta,
                                                                "microsoft-copilot",
                                                            );

                                                            context
                                                                .set_pin_value(
                                                                    "chunk",
                                                                    json!(chunk),
                                                                )
                                                                .await?;
                                                            callback_count
                                                                .fetch_add(1, Ordering::SeqCst);

                                                            let mut recursion_guard =
                                                                AHashSet::new();
                                                            recursion_guard
                                                                .insert(parent_node_id.clone());

                                                            for entry in connected_nodes.iter() {
                                                                let (id, sub_context) =
                                                                    entry.pair();
                                                                let mut sub_context =
                                                                    sub_context.lock().await;
                                                                let mut log_msg = LogMessage::new(
                                                                    &format!(
                                                                        "Streaming chunk: {} chars",
                                                                        delta.len()
                                                                    ),
                                                                    LogLevel::Debug,
                                                                    None,
                                                                );
                                                                let run = InternalNode::trigger(
                                                                    &mut sub_context,
                                                                    &mut Some(
                                                                        recursion_guard.clone(),
                                                                    ),
                                                                    true,
                                                                )
                                                                .await;
                                                                log_msg.end();
                                                                sub_context.log(log_msg);
                                                                sub_context.end_trace();
                                                                if run.is_err() {
                                                                    context.log_message(
                                                                        &format!("Error running stream node {}", id),
                                                                        LogLevel::Warn,
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }

                                                    // Collect attributions
                                                    for attr in &copilot_msg.attributions {
                                                        if !all_attributions.iter().any(|a| {
                                                            a.see_more_web_url
                                                                == attr.see_more_web_url
                                                        }) {
                                                            all_attributions.push(attr.clone());
                                                        }
                                                    }

                                                    // Store the message
                                                    if !all_messages
                                                        .iter()
                                                        .any(|m| m.id == copilot_msg.id)
                                                    {
                                                        all_messages.push(copilot_msg);
                                                    }
                                                }
                                            }
                                        }

                                        buffer = buffer[data_start + 6 + end_pos..].to_string();
                                    }
                                    Err(_) => {
                                        // Incomplete JSON, wait for more data
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            context
                                .set_pin_value("error_message", json!(e.to_string()))
                                .await?;
                            context.activate_exec_pin("error").await?;
                            return Ok(());
                        }
                    }
                }

                message.end();
                message.put_stats(LogStat::new(
                    None,
                    Some(callback_count.load(Ordering::SeqCst) as u64),
                    None,
                ));
                context.log(message);

                for entry in connected_nodes.iter() {
                    let (_, sub_context) = entry.pair();
                    let mut sub_context = sub_context.lock().await;
                    context.push_sub_context(&mut sub_context);
                }

                // Build the Copilot response
                let copilot_response = CopilotChatResponse {
                    id: response_id.clone(),
                    content: full_content.clone(),
                    created_date_time: last_created_date_time,
                    turn_count,
                    state,
                    messages: all_messages,
                };

                // Build Response with annotations from attributions
                let annotations = attributions_to_annotations(&all_attributions);
                let mut result = Response::from_text(&full_content, "microsoft-copilot");
                if let Some(choice) = result.choices.first_mut() {
                    choice.message.annotations = if annotations.is_empty() {
                        None
                    } else {
                        Some(annotations)
                    };
                }

                // Convert attributions to Attachment objects
                let attachments: Vec<Attachment> = all_attributions
                    .iter()
                    .filter_map(|attr| {
                        attr.see_more_web_url.as_ref().map(|url| {
                            Attachment::Complex(ComplexAttachment {
                                url: url.clone(),
                                preview_text: attr.provider_display_name.clone(),
                                thumbnail_url: attr.image_web_url.clone(),
                                name: attr.provider_display_name.clone(),
                                size: None,
                                r#type: Some(attr.attribution_type.clone()),
                                anchor: None,
                                page: None,
                            })
                        })
                    })
                    .collect();

                context.set_pin_value("result", json!(result)).await?;
                context
                    .set_pin_value("response", json!(copilot_response))
                    .await?;
                context
                    .set_pin_value("attachments", json!(attachments))
                    .await?;
                context
                    .set_pin_value("new_conversation_id", json!(response_id))
                    .await?;
                context.deactivate_exec_pin("on_stream").await?;
                context.activate_exec_pin("done").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("HTTP {}: {}", status, error)),
                    )
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

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        // Validate and normalize the timezone input
        if let Some(pin) = node.get_pin_mut_by_name("timezone")
            && let Some(ref value_bytes) = pin.default_value
            && let Ok(value) = flow_like_types::json::from_slice::<Value>(value_bytes)
            && let Some(input) = value.as_str()
            && let Some(normalized) = normalize_timezone(input)
            && normalized != input
        {
            pin.set_default_value(Some(json!(normalized)));
        }
    }
}

// =============================================================================
// Copilot Semantic Search Node (Official Microsoft 365 Copilot Search API)
// https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/search/copilotroot-search
// =============================================================================

/// Search hit from the Microsoft 365 Copilot Search API
/// Based on copilotSearchHit from official API
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopilotSemanticSearchResult {
    pub web_url: Option<String>,
    pub preview: Option<String>,
    pub resource_type: Option<String>,
    pub resource_metadata: Option<std::collections::HashMap<String, String>>,
}

fn parse_semantic_search_result(value: &Value) -> Option<CopilotSemanticSearchResult> {
    Some(CopilotSemanticSearchResult {
        web_url: value["webUrl"].as_str().map(String::from),
        preview: value["preview"].as_str().map(String::from),
        resource_type: value["resourceType"].as_str().map(String::from),
        resource_metadata: value["resourceMetadata"].as_object().map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        }),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotSemanticSearchNode {}

impl CopilotSemanticSearchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CopilotSemanticSearchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_semantic_search",
            "Copilot Search",
            "Perform hybrid semantic and lexical search across OneDrive for work or school content using the official Microsoft 365 Copilot Search API",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "semanticSearch");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "query",
            "Query",
            "Natural language query (max 1500 characters)",
            VariableType::String,
        );
        node.add_input_pin(
            "page_size",
            "Page Size",
            "Number of results per page (1-100, default: 25)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));
        node.add_input_pin(
            "filter_expression",
            "Filter Expression",
            "Optional KQL path filter (e.g., path:\"https://contoso.sharepoint.com/...\")",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "resource_metadata",
            "Resource Metadata",
            "Optional comma-separated metadata fields to return (e.g., title,author)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "results",
            "Search Hits",
            "Search results",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CopilotSemanticSearchResult>();
        node.add_output_pin(
            "total_count",
            "Total Count",
            "Total number of results available",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(
            MICROSOFT_PROVIDER_ID,
            vec!["Files.Read.All", "Sites.Read.All"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await.unwrap_or(25);
        let filter_expression: String = context
            .evaluate_pin("filter_expression")
            .await
            .unwrap_or_default();
        let resource_metadata: String = context
            .evaluate_pin("resource_metadata")
            .await
            .unwrap_or_default();

        // Build request body according to official Microsoft 365 Copilot Search API
        // https://learn.microsoft.com/en-us/microsoft-365-copilot/extensibility/api/ai-services/search/copilotroot-search
        let mut request_body = json!({
            "query": query,
            "pageSize": page_size.clamp(1, 100)
        });

        // Add dataSources.oneDrive configuration if filter or metadata is specified
        if !filter_expression.is_empty() || !resource_metadata.is_empty() {
            let mut one_drive_config = json!({});

            if !filter_expression.is_empty() {
                one_drive_config["filterExpression"] = json!(filter_expression);
            }

            if !resource_metadata.is_empty() {
                let metadata_fields: Vec<String> = resource_metadata
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !metadata_fields.is_empty() {
                    one_drive_config["resourceMetadataNames"] = json!(metadata_fields);
                }
            }

            request_body["dataSources"] = json!({
                "oneDrive": one_drive_config
            });
        }

        let client = reqwest::Client::new();
        let response = client
            .post(graph_version_url(&provider, "beta", "/copilot/search"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let mut results: Vec<CopilotSemanticSearchResult> = Vec::new();

                // Parse searchHits array from response
                if let Some(search_hits) = body["searchHits"].as_array() {
                    for hit in search_hits {
                        if let Some(result) = parse_semantic_search_result(hit) {
                            results.push(result);
                        }
                    }
                }

                let total_count = body["totalCount"].as_i64().unwrap_or(results.len() as i64);
                context.set_pin_value("results", json!(results)).await?;
                context
                    .set_pin_value("total_count", json!(total_count))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
                context.set_pin_value("error_message", json!(error)).await?;
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
// Subscribe to Copilot Notifications Node (Change Notifications API)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SubscribeCopilotNotificationsNode {}

impl SubscribeCopilotNotificationsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SubscribeCopilotNotificationsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_subscribe_notifications",
            "Subscribe Copilot Notifications",
            "Subscribe to change notifications for Copilot interactions",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "subscribeNotifications");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
            "notification_url",
            "Notification URL",
            "Webhook URL to receive notifications",
            VariableType::String,
        );
        node.add_input_pin(
            "expiration_minutes",
            "Expiration (Minutes)",
            "Subscription expiration time",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4230)));
        node.add_input_pin(
            "client_state",
            "Client State",
            "Optional client state for validation",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "subscription_id",
            "Subscription ID",
            "",
            VariableType::String,
        );
        node.add_output_pin(
            "expiration_date_time",
            "Expiration DateTime",
            "",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["AiEnterpriseInteraction.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let notification_url: String = context.evaluate_pin("notification_url").await?;
        let expiration_minutes: i64 = context
            .evaluate_pin("expiration_minutes")
            .await
            .unwrap_or(4230);
        let client_state: String = context
            .evaluate_pin("client_state")
            .await
            .unwrap_or_default();

        // Calculate expiration datetime
        let expiration = chrono::Utc::now() + chrono::Duration::minutes(expiration_minutes);
        let expiration_str = expiration.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        let mut request_body = json!({
            "changeType": "created",
            "notificationUrl": notification_url,
            "resource": "/copilot/users/getAllEnterpriseInteractions",
            "expirationDateTime": expiration_str
        });

        if !client_state.is_empty() {
            request_body["clientState"] = json!(client_state);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(graph_version_url(&provider, "beta", "/subscriptions"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let subscription_id = body["id"].as_str().unwrap_or("").to_string();
                let exp_dt = body["expirationDateTime"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                context
                    .set_pin_value("subscription_id", json!(subscription_id))
                    .await?;
                context
                    .set_pin_value("expiration_date_time", json!(exp_dt))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
                context.set_pin_value("error_message", json!(error)).await?;
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
// Get User Copilot Settings Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetUserCopilotSettingsNode {}

impl GetUserCopilotSettingsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetUserCopilotSettingsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_get_user_settings",
            "Get User Copilot Settings",
            "Get the current user's Copilot settings and preferences",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "getUserSettings");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

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
        node.add_output_pin(
            "settings",
            "Settings",
            "Raw settings data",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin("error_message", "Error", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["AiEnterpriseInteraction.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_json(
            &client,
            &provider,
            graph_version_url(&provider, "beta", "/me/settings/copilot"),
        )
        .await
        {
            Ok(body) => {
                context.set_pin_value("settings", body).await?;
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
// Filter Copilot Interactions by Type Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct FilterCopilotInteractionsByTypeNode {}

impl FilterCopilotInteractionsByTypeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FilterCopilotInteractionsByTypeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_filter_interactions",
            "Filter Copilot Interactions",
            "Filter Copilot interactions by type (user prompts vs AI responses)",
            "Data/Microsoft/Copilot",
        );
        node.set_flowscript_name("microsoft.copilot", "filterInteractions");
        node.set_version(1);
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "interactions",
            "Interactions",
            "Copilot interactions to filter",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CopilotInteraction>();
        node.add_input_pin(
            "interaction_type",
            "Interaction Type",
            "Type to filter by",
            VariableType::String,
        )
        .set_default_value(Some(json!("aiResponse")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["userPrompt".to_string(), "aiResponse".to_string()])
                .build(),
        );

        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);
        node.add_output_pin("filtered", "Filtered", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<CopilotInteraction>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let interactions: Vec<CopilotInteraction> = context.evaluate_pin("interactions").await?;
        let interaction_type: String = context
            .evaluate_pin("interaction_type")
            .await
            .unwrap_or_else(|_| "aiResponse".to_string());

        let filtered: Vec<CopilotInteraction> = interactions
            .into_iter()
            .filter(|i| i.interaction_type == interaction_type)
            .collect();

        let count = filtered.len() as i64;
        context.set_pin_value("filtered", json!(filtered)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
