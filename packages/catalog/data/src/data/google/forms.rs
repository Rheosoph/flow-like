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
// Google Forms Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoogleForm {
    pub form_id: String,
    pub title: String,
    pub document_title: String,
    pub description: Option<String>,
    pub responder_uri: String,
    pub revision_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoogleFormResponse {
    pub response_id: String,
    pub form_id: String,
    pub create_time: String,
    pub last_submitted_time: String,
    pub respondent_email: Option<String>,
    pub answers: Vec<GoogleFormAnswer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoogleFormAnswer {
    pub question_id: String,
    pub text_answers: Option<Vec<String>>,
    pub file_upload_answers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GoogleFormQuestion {
    pub question_id: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub question_type: String,
}

fn parse_form(value: &Value) -> Option<GoogleForm> {
    Some(GoogleForm {
        form_id: value["formId"].as_str()?.to_string(),
        title: value["info"]["title"].as_str().unwrap_or("").to_string(),
        document_title: value["info"]["documentTitle"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        description: value["info"]["description"].as_str().map(String::from),
        responder_uri: value["responderUri"].as_str().unwrap_or("").to_string(),
        revision_id: value["revisionId"].as_str().map(String::from),
    })
}

fn parse_response(value: &Value, form_id: &str) -> Option<GoogleFormResponse> {
    let answers = value["answers"]
        .as_object()
        .map(|obj| {
            obj.iter()
                .map(|(question_id, answer)| {
                    let text_answers = answer["textAnswers"]["answers"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|a| a["value"].as_str().map(String::from))
                            .collect()
                    });
                    let file_answers =
                        answer["fileUploadAnswers"]["answers"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|a| a["fileId"].as_str().map(String::from))
                                    .collect()
                            });
                    GoogleFormAnswer {
                        question_id: question_id.clone(),
                        text_answers,
                        file_upload_answers: file_answers,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Some(GoogleFormResponse {
        response_id: value["responseId"].as_str()?.to_string(),
        form_id: form_id.to_string(),
        create_time: value["createTime"].as_str().unwrap_or("").to_string(),
        last_submitted_time: value["lastSubmittedTime"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        respondent_email: value["respondentEmail"].as_str().map(String::from),
        answers,
    })
}

fn parse_question(item: &Value) -> Option<GoogleFormQuestion> {
    let question = &item["questionItem"]["question"];
    if question.is_null() {
        return None;
    }

    let question_type = if question["choiceQuestion"].is_object() {
        "choice"
    } else if question["textQuestion"].is_object() {
        "text"
    } else if question["scaleQuestion"].is_object() {
        "scale"
    } else if question["dateQuestion"].is_object() {
        "date"
    } else if question["timeQuestion"].is_object() {
        "time"
    } else if question["fileUploadQuestion"].is_object() {
        "fileUpload"
    } else {
        "unknown"
    };

    Some(GoogleFormQuestion {
        question_id: question["questionId"].as_str()?.to_string(),
        title: item["title"].as_str().unwrap_or("").to_string(),
        description: item["description"].as_str().map(String::from),
        required: question["required"].as_bool().unwrap_or(false),
        question_type: question_type.to_string(),
    })
}

// =============================================================================
// Get Form Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleFormNode {}

impl GetGoogleFormNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleFormNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_forms_get",
            "Get Form",
            "Get details of a Google Form",
            "Data/Google/Forms",
        );
        node.set_flowscript_name("google.forms", "get");
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
            "form_id",
            "Form ID",
            "The ID of the form",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("form", "Form", "Form details", VariableType::Struct)
            .set_schema::<GoogleForm>();
        node.add_output_pin(
            "questions",
            "Questions",
            "Form questions",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GoogleFormQuestion>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/forms.body.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let form_id: String = context.evaluate_pin("form_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("https://forms.googleapis.com/v1/forms/{}", form_id))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;

                if let Some(form) = parse_form(&body) {
                    context.set_pin_value("form", json!(form)).await?;
                }

                let questions: Vec<GoogleFormQuestion> = body["items"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_question).collect())
                    .unwrap_or_default();

                context.set_pin_value("questions", json!(questions)).await?;
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
// List Form Responses Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGoogleFormResponsesNode {}

impl ListGoogleFormResponsesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGoogleFormResponsesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_forms_list_responses",
            "List Form Responses",
            "List all responses to a Google Form",
            "Data/Google/Forms",
        );
        node.set_flowscript_name("google.forms", "listResponses");
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
            "form_id",
            "Form ID",
            "The ID of the form",
            VariableType::String,
        );
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "responses",
            "Responses",
            "Form responses",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GoogleFormResponse>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/forms.responses.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let form_id: String = context.evaluate_pin("form_id").await?;
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let mut url = format!(
            "https://forms.googleapis.com/v1/forms/{}/responses",
            form_id
        );
        if !page_token.is_empty() {
            url = format!("{}?pageToken={}", url, page_token);
        }

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;

                let responses: Vec<GoogleFormResponse> = body["responses"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|r| parse_response(r, &form_id))
                            .collect()
                    })
                    .unwrap_or_default();

                let next_token = body["nextPageToken"].as_str().unwrap_or("").to_string();

                context.set_pin_value("responses", json!(responses)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_token))
                    .await?;
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
// Get Form Response Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleFormResponseNode {}

impl GetGoogleFormResponseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleFormResponseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_forms_get_response",
            "Get Form Response",
            "Get a specific response from a Google Form",
            "Data/Google/Forms",
        );
        node.set_flowscript_name("google.forms", "getResponse");
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
            "form_id",
            "Form ID",
            "The ID of the form",
            VariableType::String,
        );
        node.add_input_pin(
            "response_id",
            "Response ID",
            "The ID of the response",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "response",
            "Response",
            "Form response",
            VariableType::Struct,
        )
        .set_schema::<GoogleFormResponse>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/forms.responses.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let form_id: String = context.evaluate_pin("form_id").await?;
        let response_id: String = context.evaluate_pin("response_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://forms.googleapis.com/v1/forms/{}/responses/{}",
                form_id, response_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;

                if let Some(form_response) = parse_response(&body, &form_id) {
                    context
                        .set_pin_value("response", json!(form_response))
                        .await?;
                }
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
// Create Form Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleFormNode {}

impl CreateGoogleFormNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleFormNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_forms_create",
            "Create Form",
            "Create a new Google Form",
            "Data/Google/Forms",
        );
        node.set_flowscript_name("google.forms", "create");
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
        node.add_input_pin("title", "Title", "Form title", VariableType::String);
        node.add_input_pin(
            "document_title",
            "Document Title",
            "Document title (filename)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("form", "Form", "Created form", VariableType::Struct)
            .set_schema::<GoogleForm>();
        node.add_output_pin("form_id", "Form ID", "", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/forms.body"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let title: String = context.evaluate_pin("title").await?;
        let document_title: String = context
            .evaluate_pin("document_title")
            .await
            .unwrap_or_default();

        let doc_title = if document_title.is_empty() {
            &title
        } else {
            &document_title
        };

        let body = json!({
            "info": {
                "title": title,
                "documentTitle": doc_title
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://forms.googleapis.com/v1/forms")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let form_id = body["formId"].as_str().unwrap_or("").to_string();

                if let Some(form) = parse_form(&body) {
                    context.set_pin_value("form", json!(form)).await?;
                }
                context.set_pin_value("form_id", json!(form_id)).await?;
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
// Update Form Info Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateGoogleFormInfoNode {}

impl UpdateGoogleFormInfoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateGoogleFormInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_forms_update_info",
            "Update Form Info",
            "Update title and description of a Google Form",
            "Data/Google/Forms",
        );
        node.set_flowscript_name("google.forms", "updateInfo");
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
            "form_id",
            "Form ID",
            "The ID of the form",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "New form title", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "description",
            "Description",
            "New form description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("form", "Form", "Updated form", VariableType::Struct)
            .set_schema::<GoogleForm>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/forms.body"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let form_id: String = context.evaluate_pin("form_id").await?;
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();

        let mut requests = Vec::new();
        let mut update_mask = Vec::new();

        if !title.is_empty() {
            update_mask.push("title");
        }
        if !description.is_empty() {
            update_mask.push("description");
        }

        if update_mask.is_empty() {
            context
                .set_pin_value("error_message", json!("No updates specified"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        requests.push(json!({
            "updateFormInfo": {
                "info": {
                    "title": if title.is_empty() { None } else { Some(&title) },
                    "description": if description.is_empty() { None } else { Some(&description) }
                },
                "updateMask": update_mask.join(",")
            }
        }));

        let body = json!({
            "requests": requests
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://forms.googleapis.com/v1/forms/{}:batchUpdate",
                form_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                // Fetch the updated form
                let form_response = client
                    .get(format!("https://forms.googleapis.com/v1/forms/{}", form_id))
                    .header("Authorization", format!("Bearer {}", provider.access_token))
                    .send()
                    .await;

                if let Ok(form_resp) = form_response
                    && form_resp.status().is_success()
                {
                    let form_body: Value = form_resp.json().await?;
                    if let Some(form) = parse_form(&form_body) {
                        context.set_pin_value("form", json!(form)).await?;
                    }
                }
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
