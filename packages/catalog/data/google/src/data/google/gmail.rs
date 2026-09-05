use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Gmail Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    pub label_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GmailDraft {
    pub id: String,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
}

// =============================================================================
// Send Email Node (gmail.send - non-restricted)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SendGmailNode {}

impl SendGmailNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SendGmailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_gmail_send",
            "Send Email",
            "Send an email via Gmail",
            "Data/Google/Gmail",
        );
        node.set_flowscript_name("google.gmail", "send");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "to",
            "To",
            "Recipient email address(es), comma-separated",
            VariableType::String,
        );
        node.add_input_pin("subject", "Subject", "Email subject", VariableType::String);
        node.add_input_pin(
            "body",
            "Body",
            "Email body (plain text)",
            VariableType::String,
        );
        node.add_input_pin("cc", "CC", "CC recipients (optional)", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "bcc",
            "BCC",
            "BCC recipients (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent message",
            VariableType::String,
        );
        node.add_output_pin(
            "thread_id",
            "Thread ID",
            "Thread ID of the message",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/gmail.send"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let to: String = context.evaluate_pin("to").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body: String = context.evaluate_pin("body").await?;
        let cc: String = context.evaluate_pin("cc").await.unwrap_or_default();
        let bcc: String = context.evaluate_pin("bcc").await.unwrap_or_default();

        let mut email = format!(
            "To: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n",
            to, subject
        );

        if !cc.is_empty() {
            email.push_str(&format!("Cc: {}\r\n", cc));
        }
        if !bcc.is_empty() {
            email.push_str(&format!("Bcc: {}\r\n", bcc));
        }

        email.push_str(&format!("\r\n{}", body));

        use base64::Engine;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(email.as_bytes());

        let request_body = json!({
            "raw": encoded
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://gmail.googleapis.com/gmail/v1/users/me/messages/send")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let message_id = body["id"].as_str().unwrap_or("").to_string();
                let thread_id = body["threadId"].as_str().unwrap_or("").to_string();
                context
                    .set_pin_value("message_id", json!(message_id))
                    .await?;
                context.set_pin_value("thread_id", json!(thread_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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
// Create Draft Node (gmail.compose - non-restricted)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGmailDraftNode {}

impl CreateGmailDraftNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGmailDraftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_gmail_create_draft",
            "Create Draft",
            "Create a draft email in Gmail",
            "Data/Google/Gmail",
        );
        node.set_flowscript_name("google.gmail", "createDraft");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "to",
            "To",
            "Recipient email address(es)",
            VariableType::String,
        );
        node.add_input_pin("subject", "Subject", "Email subject", VariableType::String);
        node.add_input_pin(
            "body",
            "Body",
            "Email body (plain text)",
            VariableType::String,
        );
        node.add_input_pin("cc", "CC", "CC recipients (optional)", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "bcc",
            "BCC",
            "BCC recipients (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("draft", "Draft", "Created draft", VariableType::Struct)
            .set_schema::<GmailDraft>();
        node.add_output_pin(
            "draft_id",
            "Draft ID",
            "ID of the created draft",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/gmail.compose"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let to: String = context.evaluate_pin("to").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body: String = context.evaluate_pin("body").await?;
        let cc: String = context.evaluate_pin("cc").await.unwrap_or_default();
        let bcc: String = context.evaluate_pin("bcc").await.unwrap_or_default();

        let mut email = format!(
            "To: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n",
            to, subject
        );

        if !cc.is_empty() {
            email.push_str(&format!("Cc: {}\r\n", cc));
        }
        if !bcc.is_empty() {
            email.push_str(&format!("Bcc: {}\r\n", bcc));
        }

        email.push_str(&format!("\r\n{}", body));

        use base64::Engine;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(email.as_bytes());

        let request_body = json!({
            "message": {
                "raw": encoded
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://gmail.googleapis.com/gmail/v1/users/me/drafts")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let draft_id = body["id"].as_str().unwrap_or("").to_string();
                let draft = GmailDraft {
                    id: draft_id.clone(),
                    message_id: body["message"]["id"].as_str().map(String::from),
                    thread_id: body["message"]["threadId"].as_str().map(String::from),
                };
                context.set_pin_value("draft", json!(draft)).await?;
                context.set_pin_value("draft_id", json!(draft_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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
// List Labels Node (gmail.labels - non-restricted)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGmailLabelsNode {}

impl ListGmailLabelsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGmailLabelsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_gmail_list_labels",
            "List Labels",
            "List all labels in Gmail",
            "Data/Google/Gmail",
        );
        node.set_flowscript_name("google.gmail", "listLabels");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("labels", "Labels", "List of labels", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GmailLabel>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/gmail.labels"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        let response = client
            .get("https://gmail.googleapis.com/gmail/v1/users/me/labels")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let labels: Vec<GmailLabel> = body["labels"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|l| {
                                Some(GmailLabel {
                                    id: l["id"].as_str()?.to_string(),
                                    name: l["name"].as_str()?.to_string(),
                                    label_type: l["type"].as_str().map(String::from),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                context.set_pin_value("labels", json!(labels)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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
