use super::provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Outlook Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookMailFolder {
    pub id: String,
    pub display_name: String,
    pub parent_folder_id: Option<String>,
    pub child_folder_count: i64,
    pub unread_item_count: i64,
    pub total_item_count: i64,
}

fn parse_mail_folder(folder: &Value) -> Option<OutlookMailFolder> {
    Some(OutlookMailFolder {
        id: folder["id"].as_str()?.to_string(),
        display_name: folder["displayName"].as_str()?.to_string(),
        parent_folder_id: folder["parentFolderId"].as_str().map(String::from),
        child_folder_count: folder["childFolderCount"].as_i64().unwrap_or(0),
        unread_item_count: folder["unreadItemCount"].as_i64().unwrap_or(0),
        total_item_count: folder["totalItemCount"].as_i64().unwrap_or(0),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookEmailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookMessage {
    pub id: String,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub body_content: Option<String>,
    pub body_content_type: Option<String>,
    pub from: Option<OutlookEmailAddress>,
    pub to_recipients: Vec<OutlookEmailAddress>,
    pub cc_recipients: Vec<OutlookEmailAddress>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_draft: bool,
    pub importance: Option<String>,
    pub web_link: Option<String>,
}

fn parse_email_address(addr: &Value) -> Option<OutlookEmailAddress> {
    Some(OutlookEmailAddress {
        name: addr["emailAddress"]["name"].as_str().map(String::from),
        address: addr["emailAddress"]["address"].as_str()?.to_string(),
    })
}

fn parse_message(msg: &Value) -> Option<OutlookMessage> {
    let to_recipients = msg["toRecipients"]
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_email_address).collect())
        .unwrap_or_default();

    let cc_recipients = msg["ccRecipients"]
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_email_address).collect())
        .unwrap_or_default();

    Some(OutlookMessage {
        id: msg["id"].as_str()?.to_string(),
        subject: msg["subject"].as_str().map(String::from),
        body_preview: msg["bodyPreview"].as_str().map(String::from),
        body_content: msg["body"]["content"].as_str().map(String::from),
        body_content_type: msg["body"]["contentType"].as_str().map(String::from),
        from: parse_email_address(&msg["from"]),
        to_recipients,
        cc_recipients,
        received_date_time: msg["receivedDateTime"].as_str().map(String::from),
        sent_date_time: msg["sentDateTime"].as_str().map(String::from),
        has_attachments: msg["hasAttachments"].as_bool().unwrap_or(false),
        is_read: msg["isRead"].as_bool().unwrap_or(false),
        is_draft: msg["isDraft"].as_bool().unwrap_or(false),
        importance: msg["importance"].as_str().map(String::from),
        web_link: msg["webLink"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookAttachment {
    pub id: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: i64,
    pub is_inline: bool,
    pub content_id: Option<String>,
    pub content_location: Option<String>,
    pub attachment_type: Option<String>,
    pub data: Vec<u8>,
}

fn parse_attachment(attachment: &Value) -> Option<OutlookAttachment> {
    use base64::{Engine, engine::general_purpose::STANDARD};

    Some(OutlookAttachment {
        id: attachment["id"].as_str()?.to_string(),
        filename: attachment["name"].as_str().map(String::from),
        content_type: attachment["contentType"].as_str().map(String::from),
        size: attachment["size"].as_i64().unwrap_or(0),
        is_inline: attachment["isInline"].as_bool().unwrap_or(false),
        content_id: attachment["contentId"].as_str().map(String::from),
        content_location: attachment["contentLocation"].as_str().map(String::from),
        attachment_type: attachment["@odata.type"]
            .as_str()
            .map(|value| value.trim_start_matches("#microsoft.graph.").to_string()),
        data: attachment["contentBytes"]
            .as_str()
            .and_then(|value| STANDARD.decode(value).ok())
            .unwrap_or_default(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookCalendarEvent {
    pub id: String,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub body_content: Option<String>,
    pub start_date_time: Option<String>,
    pub start_time_zone: Option<String>,
    pub end_date_time: Option<String>,
    pub end_time_zone: Option<String>,
    pub location: Option<String>,
    pub is_all_day: bool,
    pub is_cancelled: bool,
    pub is_organizer: bool,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub web_link: Option<String>,
}

fn parse_calendar_event(event: &Value) -> Option<OutlookCalendarEvent> {
    Some(OutlookCalendarEvent {
        id: event["id"].as_str()?.to_string(),
        subject: event["subject"].as_str().map(String::from),
        body_preview: event["bodyPreview"].as_str().map(String::from),
        body_content: event["body"]["content"].as_str().map(String::from),
        start_date_time: event["start"]["dateTime"].as_str().map(String::from),
        start_time_zone: event["start"]["timeZone"].as_str().map(String::from),
        end_date_time: event["end"]["dateTime"].as_str().map(String::from),
        end_time_zone: event["end"]["timeZone"].as_str().map(String::from),
        location: event["location"]["displayName"].as_str().map(String::from),
        is_all_day: event["isAllDay"].as_bool().unwrap_or(false),
        is_cancelled: event["isCancelled"].as_bool().unwrap_or(false),
        is_organizer: event["isOrganizer"].as_bool().unwrap_or(false),
        organizer_name: event["organizer"]["emailAddress"]["name"]
            .as_str()
            .map(String::from),
        organizer_email: event["organizer"]["emailAddress"]["address"]
            .as_str()
            .map(String::from),
        web_link: event["webLink"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlookContact {
    pub id: String,
    pub display_name: Option<String>,
    pub given_name: Option<String>,
    pub surname: Option<String>,
    pub email_addresses: Vec<String>,
    pub mobile_phone: Option<String>,
    pub business_phones: Vec<String>,
    pub company_name: Option<String>,
    pub job_title: Option<String>,
}

fn parse_contact(contact: &Value) -> Option<OutlookContact> {
    let email_addresses = contact["emailAddresses"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e["address"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let business_phones = contact["businessPhones"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Some(OutlookContact {
        id: contact["id"].as_str()?.to_string(),
        display_name: contact["displayName"].as_str().map(String::from),
        given_name: contact["givenName"].as_str().map(String::from),
        surname: contact["surname"].as_str().map(String::from),
        email_addresses,
        mobile_phone: contact["mobilePhone"].as_str().map(String::from),
        business_phones,
        company_name: contact["companyName"].as_str().map(String::from),
        job_title: contact["jobTitle"].as_str().map(String::from),
    })
}

// =============================================================================
// List Mail Folders Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListOutlookMailFoldersNode {}

impl ListOutlookMailFoldersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListOutlookMailFoldersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_list_mail_folders",
            "List Mail Folders",
            "List Outlook mail folders",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "folders",
            "Folders",
            "List of mail folders",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<OutlookMailFolder>();
        node.add_output_pin("count", "Count", "Number of folders", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Mail.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let url = format!("{}/me/mailFolders", provider.base_url);

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let folders = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_mail_folder).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = folders.len() as i64;
                context.set_pin_value("folders", json!(folders)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Messages Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListOutlookMessagesNode {}

impl ListOutlookMessagesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListOutlookMessagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_list_messages",
            "List Messages",
            "List Outlook email messages",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "folder_id",
            "Folder ID",
            "Mail folder ID (empty for inbox)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "filter",
            "Filter",
            "OData filter (e.g., 'isRead eq false')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "top",
            "Top",
            "Maximum messages to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "messages",
            "Messages",
            "List of email messages",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<OutlookMessage>();
        node.add_output_pin(
            "count",
            "Count",
            "Number of messages",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Mail.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let folder_id: String = context.evaluate_pin("folder_id").await.unwrap_or_default();
        let filter: String = context.evaluate_pin("filter").await.unwrap_or_default();
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(25);

        let base_url = if folder_id.is_empty() {
            format!("{}/me/messages", provider.base_url)
        } else {
            format!(
                "{}/me/mailFolders/{}/messages",
                provider.base_url, folder_id
            )
        };

        let mut query_params = vec![format!("$top={}", top)];
        if !filter.is_empty() {
            query_params.push(format!("$filter={}", urlencoding::encode(&filter)));
        }
        query_params.push("$orderby=receivedDateTime desc".to_string());

        let url = format!("{}?{}", base_url, query_params.join("&"));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let messages = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_message).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = messages.len() as i64;
                context.set_pin_value("messages", json!(messages)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Message Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetOutlookMessageNode {}

impl GetOutlookMessageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetOutlookMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_get_message",
            "Get Message",
            "Get a single Outlook email message by ID",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "message_id",
            "Message ID",
            "The message ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "message",
            "Message",
            "The email message",
            VariableType::Struct,
        )
        .set_schema::<OutlookMessage>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Mail.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let message_id: String = context.evaluate_pin("message_id").await?;

        let url = format!("{}/me/messages/{}", provider.base_url, message_id);

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(message) = parse_message(&body) {
                    context.set_pin_value("message", json!(message)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse message"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Message Attachments Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetOutlookMessageAttachmentsNode {}

impl GetOutlookMessageAttachmentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetOutlookMessageAttachmentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_get_message_attachments",
            "Get Message Attachments",
            "Fetch attachments for an Outlook message",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "message",
            "Message",
            "Outlook message from List Messages or Get Message",
            VariableType::Struct,
        )
        .set_schema::<OutlookMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "attachments",
            "Attachments",
            "List of message attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<OutlookAttachment>();
        node.add_output_pin(
            "count",
            "Count",
            "Number of attachments",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Mail.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let message: OutlookMessage = context.evaluate_pin("message").await?;

        let url = format!(
            "{}/me/messages/{}/attachments",
            provider.base_url,
            urlencoding::encode(&message.id)
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let attachments = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_attachment).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = attachments.len() as i64;
                context
                    .set_pin_value("attachments", json!(attachments))
                    .await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct OutlookAttachmentFieldsNode {}

impl OutlookAttachmentFieldsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OutlookAttachmentFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_attachment_fields",
            "Attachment → Fields",
            "Access Outlook attachment fields and bytes",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin(
            "attachment",
            "Attachment",
            "Outlook attachment struct",
            VariableType::Struct,
        )
        .set_schema::<OutlookAttachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("id", "ID", "Attachment ID", VariableType::String);
        node.add_output_pin(
            "filename",
            "Filename",
            "Attachment filename",
            VariableType::String,
        );
        node.add_output_pin(
            "content_type",
            "Content Type",
            "Attachment MIME type",
            VariableType::String,
        );
        node.add_output_pin(
            "size",
            "Size",
            "Attachment size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin(
            "is_inline",
            "Is Inline",
            "Whether the attachment is inline",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "content_id",
            "Content ID",
            "Inline content ID",
            VariableType::String,
        );
        node.add_output_pin(
            "content_location",
            "Content Location",
            "Inline content location",
            VariableType::String,
        );
        node.add_output_pin(
            "attachment_type",
            "Attachment Type",
            "Graph attachment type",
            VariableType::String,
        );
        node.add_output_pin("data", "Data", "Raw attachment bytes", VariableType::Byte)
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let attachment: OutlookAttachment = context.evaluate_pin("attachment").await?;

        context.set_pin_value("id", json!(attachment.id)).await?;
        context
            .set_pin_value("filename", json!(attachment.filename))
            .await?;
        context
            .set_pin_value("content_type", json!(attachment.content_type))
            .await?;
        context
            .set_pin_value("size", json!(attachment.size))
            .await?;
        context
            .set_pin_value("is_inline", json!(attachment.is_inline))
            .await?;
        context
            .set_pin_value("content_id", json!(attachment.content_id))
            .await?;
        context
            .set_pin_value("content_location", json!(attachment.content_location))
            .await?;
        context
            .set_pin_value("attachment_type", json!(attachment.attachment_type))
            .await?;
        context
            .set_pin_value("data", json!(attachment.data))
            .await?;

        Ok(())
    }
}

// =============================================================================
// Send Message Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SendOutlookMessageNode {}

impl SendOutlookMessageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SendOutlookMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_send_message",
            "Send Message",
            "Send an email through Outlook",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");
        node.set_version(1);

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
            "to",
            "To",
            "Recipient email addresses (comma-separated)",
            VariableType::String,
        );
        node.add_input_pin(
            "cc",
            "CC",
            "CC recipients (comma-separated)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("subject", "Subject", "Email subject", VariableType::String);
        node.add_input_pin("body", "Body", "Email body content", VariableType::String);
        node.add_input_pin(
            "is_html",
            "Is HTML",
            "Whether the body is HTML content",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "attachments",
            "Attachments",
            "Optional file attachments to include in the message",
            VariableType::Struct,
        )
        .set_schema::<OutlookAttachment>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "save_to_sent_items",
            "Save to Sent Items",
            "Save the message to Sent Items folder",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Mail.Send"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        use base64::{Engine, engine::general_purpose::STANDARD};

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let to: String = context.evaluate_pin("to").await?;
        let cc: String = context.evaluate_pin("cc").await.unwrap_or_default();
        let subject: String = context.evaluate_pin("subject").await?;
        let body: String = context.evaluate_pin("body").await?;
        let is_html: bool = context.evaluate_pin("is_html").await.unwrap_or(false);
        let attachments: Vec<OutlookAttachment> = context
            .evaluate_pin("attachments")
            .await
            .unwrap_or_default();
        let save_to_sent: bool = context
            .evaluate_pin("save_to_sent_items")
            .await
            .unwrap_or(true);

        let to_recipients: Vec<Value> = to
            .split(',')
            .map(|email| {
                json!({
                    "emailAddress": {
                        "address": email.trim()
                    }
                })
            })
            .collect();

        let cc_recipients: Vec<Value> = if cc.is_empty() {
            vec![]
        } else {
            cc.split(',')
                .map(|email| {
                    json!({
                        "emailAddress": {
                            "address": email.trim()
                        }
                    })
                })
                .collect()
        };

        let unsupported_attachments: Vec<String> = attachments
            .iter()
            .filter(|attachment| attachment.data.is_empty())
            .map(|attachment| {
                attachment
                    .filename
                    .clone()
                    .unwrap_or_else(|| attachment.id.clone())
            })
            .collect();

        if !unsupported_attachments.is_empty() {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!(
                        "Outlook sendMail JSON only supports file attachments with bytes. These attachments do not include file data: {}",
                        unsupported_attachments.join(", ")
                    )),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let attachment_payloads: Vec<Value> = attachments
            .into_iter()
            .map(|attachment| {
                json!({
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "name": attachment.filename.unwrap_or_else(|| "attachment.bin".to_string()),
                    "contentType": attachment.content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                    "contentBytes": STANDARD.encode(&attachment.data),
                    "isInline": attachment.is_inline,
                    "contentId": attachment.content_id,
                })
            })
            .collect();

        let content_type = if is_html { "HTML" } else { "Text" };

        let message_payload = json!({
            "message": {
                "subject": subject,
                "body": {
                    "contentType": content_type,
                    "content": body
                },
                "toRecipients": to_recipients,
                "ccRecipients": cc_recipients,
                "attachments": attachment_payloads
            },
            "saveToSentItems": save_to_sent
        });

        let url = format!("{}/me/sendMail", provider.base_url);

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&message_payload)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Calendar Events Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListOutlookCalendarEventsNode {}

impl ListOutlookCalendarEventsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListOutlookCalendarEventsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_list_calendar_events",
            "List Calendar Events",
            "List Outlook calendar events",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "start_date_time",
            "Start DateTime",
            "Start of time range (ISO 8601 format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "End of time range (ISO 8601 format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "top",
            "Top",
            "Maximum events to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "events",
            "Events",
            "List of calendar events",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<OutlookCalendarEvent>();
        node.add_output_pin("count", "Count", "Number of events", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let start_date_time: String = context
            .evaluate_pin("start_date_time")
            .await
            .unwrap_or_default();
        let end_date_time: String = context
            .evaluate_pin("end_date_time")
            .await
            .unwrap_or_default();
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(50);

        let url = if !start_date_time.is_empty() && !end_date_time.is_empty() {
            format!(
                "{}/me/calendarView?startDateTime={}&endDateTime={}&$top={}",
                provider.base_url,
                urlencoding::encode(&start_date_time),
                urlencoding::encode(&end_date_time),
                top
            )
        } else {
            format!(
                "{}/me/events?$top={}&$orderby=start/dateTime",
                provider.base_url, top
            )
        };

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let events = body["value"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(parse_calendar_event)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let count = events.len() as i64;
                context.set_pin_value("events", json!(events)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Contacts Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListOutlookContactsNode {}

impl ListOutlookContactsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListOutlookContactsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_list_contacts",
            "List Contacts",
            "List Outlook contacts",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "search",
            "Search",
            "Search term to filter contacts",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "top",
            "Top",
            "Maximum contacts to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "contacts",
            "Contacts",
            "List of contacts",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<OutlookContact>();
        node.add_output_pin(
            "count",
            "Count",
            "Number of contacts",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Contacts.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let search: String = context.evaluate_pin("search").await.unwrap_or_default();
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(100);

        let mut url = format!("{}/me/contacts?$top={}", provider.base_url, top);
        if !search.is_empty() {
            url.push_str(&format!("&$search=\"{}\"", urlencoding::encode(&search)));
        }

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let contacts = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_contact).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = contacts.len() as i64;
                context.set_pin_value("contacts", json!(contacts)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Create Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateOutlookCalendarEventNode {}

impl CreateOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_create_calendar_event",
            "Create Calendar Event",
            "Create a new Outlook calendar event",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("subject", "Subject", "Event subject", VariableType::String);
        node.add_input_pin(
            "body",
            "Body",
            "Event description (HTML)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "start_date_time",
            "Start DateTime",
            "Start date/time (ISO format)",
            VariableType::String,
        );
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "End date/time (ISO format)",
            VariableType::String,
        );
        node.add_input_pin(
            "time_zone",
            "Time Zone",
            "Time zone (e.g., 'UTC', 'Pacific Standard Time')",
            VariableType::String,
        )
        .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "location",
            "Location",
            "Event location",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "attendees",
            "Attendees",
            "Comma-separated email addresses",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "is_all_day",
            "Is All Day",
            "Whether this is an all-day event",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "is_online_meeting",
            "Is Online Meeting",
            "Create as Teams meeting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "importance",
            "Importance",
            "Event importance",
            VariableType::String,
        )
        .set_default_value(Some(json!("normal")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "low".to_string(),
                    "normal".to_string(),
                    "high".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Created event", VariableType::Struct)
            .set_schema::<OutlookCalendarEvent>();
        node.add_output_pin(
            "event_id",
            "Event ID",
            "Created event ID",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let start_date_time: String = context.evaluate_pin("start_date_time").await?;
        let end_date_time: String = context.evaluate_pin("end_date_time").await?;
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();
        let attendees: String = context.evaluate_pin("attendees").await.unwrap_or_default();
        let is_all_day: bool = context.evaluate_pin("is_all_day").await.unwrap_or(false);
        let is_online_meeting: bool = context
            .evaluate_pin("is_online_meeting")
            .await
            .unwrap_or(false);
        let importance: String = context
            .evaluate_pin("importance")
            .await
            .unwrap_or_else(|_| "normal".to_string());

        let mut event_payload = json!({
            "subject": subject,
            "start": {
                "dateTime": start_date_time,
                "timeZone": time_zone
            },
            "end": {
                "dateTime": end_date_time,
                "timeZone": time_zone
            },
            "isAllDay": is_all_day,
            "importance": importance
        });

        if !body.is_empty() {
            event_payload["body"] = json!({
                "contentType": "HTML",
                "content": body
            });
        }

        if !location.is_empty() {
            event_payload["location"] = json!({
                "displayName": location
            });
        }

        if !attendees.is_empty() {
            let attendee_list: Vec<Value> = attendees
                .split(',')
                .map(|email| email.trim())
                .filter(|email| !email.is_empty())
                .map(|email| {
                    json!({
                        "emailAddress": { "address": email },
                        "type": "required"
                    })
                })
                .collect();
            event_payload["attendees"] = json!(attendee_list);
        }

        if is_online_meeting {
            event_payload["isOnlineMeeting"] = json!(true);
            event_payload["onlineMeetingProvider"] = json!("teamsForBusiness");
        }

        let url = format!("{}/me/events", provider.base_url);

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&event_payload)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let event_id = body["id"].as_str().unwrap_or_default().to_string();
                if let Some(event) = parse_calendar_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                    context.set_pin_value("event_id", json!(event_id)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse created event"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Update Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateOutlookCalendarEventNode {}

impl UpdateOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_update_calendar_event",
            "Update Calendar Event",
            "Update an existing Outlook calendar event",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "event_id",
            "Event ID",
            "ID of the event to update",
            VariableType::String,
        );
        node.add_input_pin(
            "subject",
            "Subject",
            "New subject (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "body",
            "Body",
            "New body (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "start_date_time",
            "Start DateTime",
            "New start (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "New end (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "time_zone",
            "Time Zone",
            "Time zone for dates",
            VariableType::String,
        )
        .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "location",
            "Location",
            "New location (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Updated event", VariableType::Struct)
            .set_schema::<OutlookCalendarEvent>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let subject: String = context.evaluate_pin("subject").await.unwrap_or_default();
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let start_date_time: String = context
            .evaluate_pin("start_date_time")
            .await
            .unwrap_or_default();
        let end_date_time: String = context
            .evaluate_pin("end_date_time")
            .await
            .unwrap_or_default();
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();

        let mut event_payload = json!({});

        if !subject.is_empty() {
            event_payload["subject"] = json!(subject);
        }
        if !body.is_empty() {
            event_payload["body"] = json!({
                "contentType": "HTML",
                "content": body
            });
        }
        if !start_date_time.is_empty() {
            event_payload["start"] = json!({
                "dateTime": start_date_time,
                "timeZone": time_zone
            });
        }
        if !end_date_time.is_empty() {
            event_payload["end"] = json!({
                "dateTime": end_date_time,
                "timeZone": time_zone
            });
        }
        if !location.is_empty() {
            event_payload["location"] = json!({ "displayName": location });
        }

        let url = format!("{}/me/events/{}", provider.base_url, event_id);

        let client = reqwest::Client::new();
        let response = client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&event_payload)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(event) = parse_calendar_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse updated event"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteOutlookCalendarEventNode {}

impl DeleteOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_delete_calendar_event",
            "Delete Calendar Event",
            "Delete an Outlook calendar event",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "event_id",
            "Event ID",
            "ID of the event to delete",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;

        let url = format!("{}/me/events/{}", provider.base_url, event_id);

        let client = reqwest::Client::new();
        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetOutlookCalendarEventNode {}

impl GetOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_get_calendar_event",
            "Get Calendar Event",
            "Get a single Outlook calendar event by ID",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "event_id",
            "Event ID",
            "ID of the event",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "The calendar event", VariableType::Struct)
            .set_schema::<OutlookCalendarEvent>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;

        let url = format!("{}/me/events/{}", provider.base_url, event_id);

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(event) = parse_calendar_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse event"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// RSVP Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RsvpOutlookCalendarEventNode {}

impl RsvpOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RsvpOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_rsvp_calendar_event",
            "RSVP Calendar Event",
            "Accept, decline, or tentatively accept a calendar event invitation",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "event_id",
            "Event ID",
            "ID of the event",
            VariableType::String,
        );
        node.add_input_pin(
            "response",
            "Response",
            "Your response",
            VariableType::String,
        )
        .set_default_value(Some(json!("accept")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "accept".to_string(),
                    "tentativelyAccept".to_string(),
                    "decline".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "comment",
            "Comment",
            "Optional comment to send with response",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "send_response",
            "Send Response",
            "Whether to send a response to the organizer",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let response: String = context
            .evaluate_pin("response")
            .await
            .unwrap_or_else(|_| "accept".to_string());
        let comment: String = context.evaluate_pin("comment").await.unwrap_or_default();
        let send_response: bool = context.evaluate_pin("send_response").await.unwrap_or(true);

        let url = format!("{}/me/events/{}/{}", provider.base_url, event_id, response);

        let payload = json!({
            "comment": comment,
            "sendResponse": send_response
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Forward Calendar Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ForwardOutlookCalendarEventNode {}

impl ForwardOutlookCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ForwardOutlookCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_outlook_forward_calendar_event",
            "Forward Calendar Event",
            "Forward a calendar event to other recipients",
            "Data/Microsoft/Outlook",
        );
        node.add_icon("/flow/icons/outlook.svg");

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
            "event_id",
            "Event ID",
            "ID of the event to forward",
            VariableType::String,
        );
        node.add_input_pin(
            "to_recipients",
            "To Recipients",
            "Comma-separated email addresses",
            VariableType::String,
        );
        node.add_input_pin(
            "comment",
            "Comment",
            "Optional comment to include",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let to_recipients: String = context.evaluate_pin("to_recipients").await?;
        let comment: String = context.evaluate_pin("comment").await.unwrap_or_default();

        let recipients: Vec<Value> = to_recipients
            .split(',')
            .map(|email| email.trim())
            .filter(|email| !email.is_empty())
            .map(|email| json!({ "emailAddress": { "address": email } }))
            .collect();

        let url = format!("{}/me/events/{}/forward", provider.base_url, event_id);

        let payload = json!({
            "toRecipients": recipients,
            "comment": comment
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("API error {}: {}", status, error_text)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
